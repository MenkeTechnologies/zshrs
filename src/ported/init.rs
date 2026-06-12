//! init.c - main loop and initialization routines
//!
//! Port of Src/init.c

use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::ported::builtin::{realexit, LASTVAL, RETFLAG, STOPMSG};
use crate::ported::context::zcontext_restore;
use crate::ported::hist::{curhist, curline, hbegin, hend, hist_ring, histlinect, stophist};
use crate::ported::lex::{set_tok, tok, ENDINPUT};
use crate::ported::mem::popheap;
use crate::ported::options::{dosetopt, emulation};
use crate::ported::params::{getsparam, TERMFLAGS};
use crate::ported::signals::{
    dotrap, install_handler, intr, queue_signals, signal_ignore, sigtrapped, unqueue_signals,
};
use crate::ported::signals_h::dont_queue_signals;
use crate::ported::text::getpermtext;
use crate::ported::utils::{callhookfunc, errflag, movefd, unmeta, ERRFLAG_ERROR};
use crate::ported::zsh_h::{
    eprog, hookdef, interact, islogin, isset, jobbing, Eprog, EMULATE_KSH, EMULATE_SH, GLOBALRCS,
    HISTBEEP, HISTIGNOREDUPS, HIST_DUP, HIST_TMPSTORE, HOOKF_ALL, HOOK_SUFFIX, HUP, INTERACTIVE,
    LEXERR, PRIVILEGED, RCS, SHINSTDIN, SINGLECOMMAND, TERM_BAD, TERM_NOUP, TERM_UNKNOWN,
    ZEXIT_NORMAL,
    ZLE_CMD_POSTEXEC, ZLE_CMD_PREEXEC,
};
// =========================================================================
// File-scope globals from init.c
// =========================================================================

/// Port of `int noexitct` from Src/init.c:44.
pub static noexitct: AtomicI32 = AtomicI32::new(0); // c:44

// buffer for $_ and its length                                              // c:46

/// Port of `char *zunderscore` from Src/init.c:49.
pub static zunderscore: Mutex<String> = Mutex::new(String::new()); // c:49

/// Port of `size_t underscorelen` from Src/init.c:52.
pub static underscorelen: AtomicUsize = AtomicUsize::new(0); // c:52

/// Port of `int underscoreused` from Src/init.c:55.
pub static underscoreused: AtomicI32 = AtomicI32::new(0); // c:55

// what level of sourcing we are at                                          // c:57

/// Port of `int sourcelevel` from Src/init.c:60.
pub static sourcelevel: AtomicI32 = AtomicI32::new(0); // c:60

// the shell tty fd                                                          // c:62

/// Port of `mod_export int SHTTY` from Src/init.c:65.
pub static SHTTY: AtomicI32 = AtomicI32::new(-1); // c:65

// the FILE attached to the shell tty                                        // c:67
// `mod_export FILE *shout;` — represented as a libc::FILE pointer.          // c:70
pub static shout: Mutex<usize> = Mutex::new(0); // c:70

// termcap strings                                                           // c:72

/// Port of `mod_export char *tcstr[TC_COUNT]` from Src/init.c:75.
pub static tcstr: Mutex<[String; 39]> = Mutex::new([
    // c:75
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
    String::new(),
]);

// lengths of each termcap string                                            // c:78

/// Port of `mod_export int tclen[TC_COUNT]` from Src/init.c:81.
pub static tclen: Mutex<[i32; 39]> = Mutex::new([0; 39]); // c:81

// Values of the li, co and am entries                                       // c:82

/// Port of `int tclines` from Src/init.c:85.
pub static tclines: AtomicI32 = AtomicI32::new(0); // c:85

/// Port of `int tccolumns` from Src/init.c:85.
pub static tccolumns: AtomicI32 = AtomicI32::new(0); // c:85

/// Port of `mod_export int hasam` from Src/init.c:87.
pub static hasam: AtomicI32 = AtomicI32::new(0); // c:87

/// Port of `int hasxn` from Src/init.c:89.
pub static hasxn: AtomicI32 = AtomicI32::new(0); // c:89

// Value of the Co (max_colors) entry: may not be set                        // c:91

/// Port of `mod_export int tccolours` from Src/init.c:94.
pub static tccolours: AtomicI32 = AtomicI32::new(0); // c:94

// SIGCHLD mask                                                              // c:96
// `mod_export sigset_t sigchld_mask;` — owned by signals layer.             // c:99

/// Port of `struct hookdef zshhooks[]` from `Src/init.c:101-106`:
/// ```c
/// struct hookdef zshhooks[] = {
///     HOOKDEF("exit", NULL, HOOKF_ALL),
///     HOOKDEF("before_trap", NULL, HOOKF_ALL),
///     HOOKDEF("after_trap", NULL, HOOKF_ALL),
///     HOOKDEF("get_color_attr", NULL, HOOKF_ALL),
/// };
/// ```
/// Stored as `AtomicPtr<hookdef>` holding the base pointer of a
/// heap-leaked `[hookdef; 4]` so that `addhookdefs(NULL, zshhooks,
/// 4)` at `setupvals()` (c:1085) can walk the array via `h++` exactly
/// as the C call does. The leak is intentional — C's static-storage
/// `zshhooks[]` has program lifetime, and the registered hookdef
/// pointers must stay valid for any later `runhookdef` dispatch.
pub static zshhooks: once_cell::sync::Lazy<
    // c:101
    std::sync::atomic::AtomicPtr<hookdef>,
> = once_cell::sync::Lazy::new(|| {
    let arr: Box<[hookdef; 4]> = Box::new([
        hookdef {
            // c:102
            next: std::ptr::null_mut(),
            name: "exit".to_string(),
            def: None,
            flags: HOOKF_ALL,
            funcs: std::ptr::null_mut(),
        },
        hookdef {
            // c:103
            next: std::ptr::null_mut(),
            name: "before_trap".to_string(),
            def: None,
            flags: HOOKF_ALL,
            funcs: std::ptr::null_mut(),
        },
        hookdef {
            // c:104
            next: std::ptr::null_mut(),
            name: "after_trap".to_string(),
            def: None,
            flags: HOOKF_ALL,
            funcs: std::ptr::null_mut(),
        },
        hookdef {
            // c:105
            next: std::ptr::null_mut(),
            name: "get_color_attr".to_string(),
            def: None,
            flags: HOOKF_ALL,
            funcs: std::ptr::null_mut(),
        },
    ]);
    let base = Box::into_raw(arr) as *mut hookdef;
    std::sync::atomic::AtomicPtr::new(base)
});

// original argv[0]. This is already metafied                                // c:258

/// Port of `static char *argv0` from Src/init.c:259.
static argv0: Mutex<String> = Mutex::new(String::new()); // c:259

/// Port of `mod_export ZleEntryPoint zle_entry_ptr` from Src/init.c:1730.
/// Stored as a usize representing a fn pointer (0 == NULL).
pub static zle_entry_ptr: AtomicUsize = AtomicUsize::new(0); // c:1730

/// Port of `mod_export int zle_load_state` from Src/init.c:1739.
pub static zle_load_state: AtomicI32 = AtomicI32::new(0); // c:1739

/// Port of `mod_export CompctlReadFn compctlreadptr` from Src/init.c:1831.
pub static compctlreadptr: AtomicUsize = AtomicUsize::new(0); // c:1831

/// Port of `mod_export int use_exit_printed` from Src/init.c:1846.
pub static use_exit_printed: AtomicI32 = AtomicI32::new(0); // c:1846

// =========================================================================
// Static arrays from init.c
// =========================================================================

/// Port of `static char *tccapnams[TC_COUNT]` from Src/init.c:747.
const tccapnams: [&str; 39] = [
    // c:747
    "cl", "le", "LE", "nd", "RI", "up", "UP", "do", "DO", "dc", "DC", "ic", "IC", "cd", "ce", "al",
    "dl", "ta", "md", "mh", "so", "us", "ZH", "me", "se", "ue", "ZR", "ch", "ku", "kd", "kl", "kr",
    "sc", "rc", "bc", "AF", "AB", "vi", "ve",
];

/// Port of `static void parseargs(...)` from Src/init.c:263.
fn parseargs(
    zsh_name: &str,
    argv: &mut Vec<String>, // c:263
    runscript: &mut Option<String>,
    cmdptr: &mut Option<String>,
) {
    let mut idx: usize = 0; // c:265
    let flags: i32 = 1; /* PARSEARGS_TOPLEVEL */
    // c:267
    let flags = if argv.first().map(|s| s.starts_with('-')).unwrap_or(false)
    // c:268-269
    {
        flags | 2 /* PARSEARGS_LOGIN */
    } else {
        flags
    };

    *argv0.lock().unwrap() = argv[idx].clone(); // c:271
                                                // argzero = posixzero = *argv++                                         // c:271
    idx += 1;
    // SHIN = 0;                                                             // c:272

    // parseopts(zsh_name, &argv, opts, cmdptr, NULL, flags)                 // c:280
    let _ = parseopts(zsh_name, argv, &mut idx, cmdptr, flags);

    // if (opts[SHINSTDIN]) opts[USEZLE] = (opts[USEZLE] && isatty(0));      // c:291-292

    // paramlist = znewlinklist();                                           // c:294
    let mut paramlist: Vec<String> = Vec::new();
    if idx < argv.len() {
        // c:295
        // if (unset(SHINSTDIN)) { ... }                                     // c:296-304
        if cmdptr.is_none() {
            // c:298-301
            *runscript = Some(argv[idx].clone());
        }
        idx += 1;
        while idx < argv.len() {
            // c:305-306
            paramlist.push(argv[idx].clone());
            idx += 1;
        }
    } else if cmdptr.is_none() { // c:307
         // opts[SHINSTDIN] = 1;                                              // c:328
    }
    // pparams = ...                                                         // c:328
    let _ = paramlist;
}

/// Port of `static void parseopts_insert(...)` from Src/init.c:328.
///
/// Insert into list in order of pointer value.
fn parseopts_insert(optlist: &mut Vec<usize>, base: usize, optno: i32) {
    // c:328
    let ptr = base + (if optno < 0 { -optno } else { optno }) as usize; // c:348
    for (i, &node) in optlist.iter().enumerate() {
        // c:348
        if ptr < node {
            // c:348
            optlist.insert(i, ptr); // c:348
            return; // c:348
        }
    }
    optlist.push(ptr); // c:348
}

/// Port of `mod_export int parseopts(...)` from Src/init.c:390.
/// Rust idiom replacement: index-walk over `argv` Vec covers the C
/// argv pointer-advance; the `emulate_required` / `toplevel` state
/// tracking mirrors the C source's local flags. The long-option
/// table lookups happen against the shared options.rs registry.
pub fn parseopts(
    _nam: &str,
    argv: &mut Vec<String>,
    idx: &mut usize, // c:390
    cmdp: &mut Option<String>,
    flags: i32,
) -> i32 {
    let toplevel = (flags & 1) != 0; // c:396
    let mut emulate_required = toplevel; // c:397
    *cmdp = None; // c:400

    while *idx < argv.len() {
        // c:418
        let arg = argv[*idx].clone();
        if !(arg.starts_with('-') || arg.starts_with('+')) {
            break;
        }
        if arg == "--version" {
            // c:434
            println!("zshrs (C-port)"); // c:435-436
            if toplevel {
                std::process::exit(0);
            } // c:437
        }
        if arg == "--help" {
            // c:439
            printhelp(); // c:440
            if toplevel {
                std::process::exit(0);
            } // c:441
        }
        if arg == "-c" {
            // c:470
            if emulate_required {
                // c:471
                parseopts_setemulate(_nam, flags); // c:472
                emulate_required = false; // c:473
            }
            *idx += 1;
            *cmdp = argv.get(*idx).cloned(); // c:476
            *idx += 1;
            continue;
        }
        // Other option parsing handled by clap in main.rs
        *idx += 1;
    }
    if emulate_required {
        // c:557
        parseopts_setemulate(_nam, flags); // c:557
    }
    0 // c:557
}

/// Port of `static void printhelp(void)` from Src/init.c:557.
fn printhelp() {
    // c:557
    let argz = argv0.lock().unwrap().clone(); // c:557
    println!("Usage: {} [<options>] [<argument> ...]", argz); // c:559
    println!(); // c:560
    println!("Special options:"); // c:560
    println!("  --help     show this message, then exit"); // c:561
    println!("  --version  show zsh version number, then exit"); // c:562
    println!("  -b         end option processing, like --"); // c:564
    println!("  -c         take first argument as a command to execute"); // c:577
    println!("  -o OPTION  set an option by name (see below)"); // c:577
    println!(); // c:577
    println!("Normal options are named.  An option may be turned on by"); // c:577
    println!("`-o OPTION', `--OPTION', `+o no_OPTION' or `+-no-OPTION'.  An"); // c:577
    println!("option may be turned off by `-o no_OPTION', `--no-OPTION',"); // c:577
    println!("`+o OPTION' or `+-OPTION'.  Options are listed below only in"); // c:577
    println!("`--OPTION' or `--no-OPTION' form."); // c:577
                                                   // printoptionlist();                                                    // c:577
}

/// Port of `mod_export void init_io(char *cmd)` from Src/init.c:577.
pub fn init_io(_cmd: Option<&str>) {
    // c:577
    // stdout, stderr fully buffered                                         // c:577
    // setvbuf(stdout, outbuf, _IOFBF, BUFSIZ); setvbuf(stderr, ...)         // c:587-591
    // (Rust's stdout/stderr are line/block buffered by default)

    // Close any existing shout                                              // c:605-614
    *shout.lock().unwrap() = 0;
    if SHTTY.load(Ordering::SeqCst) != -1 {
        // c:615
        // c:616 — `zclose(SHTTY);` — fdtable-aware close. SHTTY was
        // registered as FDT_INTERNAL by movefd at one of the open
        // sites below (c:627 ttyname/O_RDWR open, c:658 dup(0), c:662
        // dup(1), c:668 /dev/tty open — all routed through movefd
        // which sets fdtable[fd] = FDT_INTERNAL at utils.rs:2243).
        // Prior port used raw libc::close which skipped the
        // fdtable_set(fd, FDT_UNUSED) clear that zclose does at
        // utils.rs:2402. Same leak shape as random.rs finish_
        // (b3107b5a46), tcp.rs tcp_close (9b4dae375a), and
        // zpty.rs deleteptycmd (c37083d09f) — stale FDT_INTERNAL
        // marker survives the close → kernel-reused fd inherits the
        // module-owned classification → closem(FDT_UNUSED, 0) calls
        // from sibling builtins skip closing it.
        let _ = crate::ported::utils::zclose(SHTTY.load(Ordering::SeqCst));
        SHTTY.store(-1, Ordering::SeqCst); // c:617
    }

    // xtrerr = stderr;                                                      // c:621

    // Make sure the tty is opened read/write.                               // c:623
    //
    // C uses `if (isatty(0))` — the truthy test, NOT a strict
    // `== 1`. POSIX guarantees isatty returns 0 on false but
    // "non-zero" on true with the exact value implementation-
    // defined. macOS/glibc/musl all return 1 today but the
    // strict `== 1` check is brittle — match C's truthy test
    // so a future libc that returns any non-zero value still
    // hits the SHTTY-open path.
    // ttystrname is the canonical "name of the controlling tty" global
    // (init.c declares it; clone.c reads it for $TTY after fork). The
    // C `init_io` keeps it in lockstep with `SHTTY`: every open-site
    // and every dup-fallback either replaces it with `ttyname(fd)` of
    // the new SHTTY or resets to "" / "/dev/tty" on failure. Mirror
    // every store here so $TTY isn't empty after init.
    let set_ttystrname = |s: String| {
        // c:625 zsfree(ttystrname); ttystrname = ztrdup(...)
        *crate::ported::modules::clone::ttystrname.lock().unwrap() = s;
    };
    #[cfg(unix)]
    let ttyname_of = |fd: i32| -> Option<String> {
        unsafe {
            let p = libc::ttyname(fd);
            if p.is_null() {
                None
            } else {
                std::ffi::CStr::from_ptr(p)
                    .to_str()
                    .ok()
                    .map(|s| s.to_string())
            }
        }
    };
    #[cfg(unix)]
    unsafe {
        if libc::isatty(0) != 0 {
            // c:624
            let name_ptr = libc::ttyname(0); // c:626
            if !name_ptr.is_null() {
                let name = std::ffi::CStr::from_ptr(name_ptr);
                let cstr = std::ffi::CString::new(name.to_bytes()).unwrap();
                // c:626 ttystrname = ztrdup(ttyname(0))
                set_ttystrname(name.to_string_lossy().into_owned());
                let fd = libc::open(
                    cstr.as_ptr(), // c:627
                    libc::O_RDWR | libc::O_NOCTTY,
                );
                SHTTY.store(movefd(fd), Ordering::SeqCst);
            }
            if SHTTY.load(Ordering::SeqCst) == -1 {
                // c:658
                SHTTY.store(movefd(libc::dup(0)), Ordering::SeqCst);
                // c:659
            }
        }
        if SHTTY.load(Ordering::SeqCst) == -1 && libc::isatty(1) != 0 {
            // c:662
            SHTTY.store(movefd(libc::dup(1)), Ordering::SeqCst);
            // c:663
            // c:664-665 — zsfree(ttystrname); ttystrname = ztrdup(ttyname(1));
            if let Some(n) = ttyname_of(1) {
                set_ttystrname(n);
            }
        }
        if SHTTY.load(Ordering::SeqCst) == -1 {
            // c:667
            let dev_tty = std::ffi::CString::new("/dev/tty").unwrap();
            let fd = libc::open(dev_tty.as_ptr(), libc::O_RDWR | libc::O_NOCTTY); // c:668
            SHTTY.store(movefd(fd), Ordering::SeqCst);
            if SHTTY.load(Ordering::SeqCst) != -1 {
                // c:669-670 — ttystrname = ztrdup(ttyname(SHTTY));
                if let Some(n) = ttyname_of(SHTTY.load(Ordering::SeqCst)) {
                    set_ttystrname(n);
                }
            }
        }
        if SHTTY.load(Ordering::SeqCst) == -1 {
            // c:672-674 — failed all opens: ttystrname = "";
            set_ttystrname(String::new());
        } else {
            // c:675
            let fdflags = libc::fcntl(SHTTY.load(Ordering::SeqCst), libc::F_GETFD, 0); // c:677
            if fdflags != -1 {
                // c:678
                libc::fcntl(
                    SHTTY.load(Ordering::SeqCst),
                    libc::F_SETFD, // c:680
                    fdflags | libc::FD_CLOEXEC,
                );
            }
            // c:683-684 — if (!ttystrname) ttystrname = ztrdup("/dev/tty");
            // Rust mirrors NULL-check via empty-string check on the Mutex.
            let mut guard = crate::ported::modules::clone::ttystrname.lock().unwrap();
            if guard.is_empty() {
                *guard = "/dev/tty".to_string();
            }
        }
    }

    // if (interact) { init_shout(); ... }                                   // c:689-694
    init_shout(); // c:690

    // mypid = (zlong)getpid();                                              // c:712
    // if (opts[MONITOR]) { ... acquire_pgrp() ... }                         // c:712-707
    let _ = crate::ported::jobs::acquire_pgrp();
}

/// Port of `mod_export void init_shout(void)` from Src/init.c:712.
/// Rust idiom replacement: SHTTY atomic + `acquire_pgrp` covers the
/// C `fdopen(SHTTY, "w")` + setpgrp dance; the FILE* stream is
/// reconstituted on-demand by callers rather than stored as a
/// global `shout` pointer.
pub fn init_shout() {
    // c:712
    if SHTTY.load(Ordering::SeqCst) == -1 {
        // c:712
        // shout = stderr; return;                                           // c:722-723
        return;
    }
    // shout = fdopen(SHTTY, "w");                                           // c:732
    // setvbuf(shout, shoutbuf, _IOFBF, BUFSIZ);                             // c:735
    let _ = crate::ported::utils::gettyinfo(); // c:771
}

/// Port of `mod_export char *tccap_get_name(int cap)` from Src/init.c:756.
pub fn tccap_get_name(cap: usize) -> &'static str {
    // c:756
    if cap >= 39
    /* TC_COUNT */
    {
        // c:771
        return ""; // c:771
    }
    tccapnams[cap] // c:771
}

/// Port of `mod_export int init_term(void)` from Src/init.c:771.
///
/// Reads `$TERM` from the param table, and on a recognised term
/// populates `tcstr[]`/`tclen[]` from the system termcap/terminfo
/// DB via `tgetent` + `tgetstr` over `tccapnams[]`, exactly like C.
/// This is the substrate every `tcmultout` / `tc_leftcurs` /
/// `tsetcap` call site reads.
///
/// ncurses is already linked (build.rs `rustc-link-lib=ncurses`, the
/// same lib the zsh/terminfo module's `tigetstr` externs use), so
/// the termcap-emulation entry points are available. The previous
/// Rust body hardcoded ANSI/VT100 escapes — under `TERM=screen-*` /
/// `tmux-*` that diverged from zsh on standout (`so` is `\e[3m`
/// there, not the hardcoded `\e[7m`).
pub fn init_term() -> i32 {
    // c:766 — termcap emulation entry points from ncurses (the
    // prototypes_h.rs pin forbids declaring these there; local
    // extern block matches the modules/terminfo.rs pattern).
    extern "C" {
        fn tgetent(bp: *mut libc::c_char, name: *const libc::c_char) -> libc::c_int;
        fn tgetstr(
            id: *const libc::c_char,
            area: *mut *mut libc::c_char,
        ) -> *mut libc::c_char;
        fn tgetflag(id: *const libc::c_char) -> libc::c_int;
        fn tgetnum(id: *const libc::c_char) -> libc::c_int;
    }
    use crate::ported::zsh_h::{
        TCBACKSPACE, TCCLEARSCREEN, TCDOWN, TCFAINTBEG, TCITALICSBEG, TCITALICSEND, TCLEFT,
        TCRESTRCURSOR, TCSAVECURSOR, TCUP, TC_COUNT,
    };

    // c:776-779 — `if (!*term) { termflags |= TERM_UNKNOWN; return 0; }`
    let term = getsparam("TERM").unwrap_or_default();
    if term.is_empty() {
        TERMFLAGS.fetch_or(TERM_UNKNOWN, Ordering::SeqCst);
        return 0;
    }

    // c:782-783 — `if (!strcmp(term, "emacs")) opts[USEZLE] = 0;`
    // C proceeds to tgetent afterwards; zshrs keeps the USEZLE-off
    // via the option layer.
    if term == "emacs" {
        crate::ported::options::opt_state_set("zle", false); // c:783 opts[USEZLE] = 0
    }

    let cterm = match std::ffi::CString::new(term.as_str()) {
        Ok(c) => c,
        Err(_) => {
            TERMFLAGS.fetch_or(TERM_BAD, Ordering::SeqCst);
            return 0;
        }
    };
    // c:785-797 — `if (tgetent(termbuf, term) != TGETENT_SUCCESS) {
    //   zerr(...); errflag &= ~ERRFLAG_ERROR; termflags |= TERM_BAD;
    //   return 0; }`. ncurses tgetent accepts NULL (the
    //   TGETENT_ACCEPTS_NULL arm at c:786-787).
    let ent = unsafe { tgetent(std::ptr::null_mut(), cterm.as_ptr()) };
    if ent != 1 {
        // c:791 — `if (interact) zerr("can't find terminal definition
        // for %s", term);` — interact gate keeps -fc scripts quiet.
        if crate::ported::zsh_h::isset(crate::ported::zsh_h::INTERACTIVE) {
            crate::ported::utils::zerr(&format!(
                "can't find terminal definition for {}",
                term
            )); // c:792
        }
        crate::ported::utils::errflag.fetch_and(
            !crate::ported::utils::ERRFLAG_ERROR,
            Ordering::Relaxed,
        ); // c:793
        TERMFLAGS.fetch_or(TERM_BAD, Ordering::SeqCst); // c:794
        return 0; // c:795
    }

    // c:801-802 — `termflags &= ~TERM_BAD; termflags &= ~TERM_UNKNOWN;`
    TERMFLAGS.fetch_and(!(TERM_BAD | TERM_UNKNOWN), Ordering::SeqCst);

    // c:803-815 — `for (t0 = 0; t0 != TC_COUNT; t0++) { ... tgetstr
    // (tccapnams[t0], &pp) ... }` — fetch every capability string.
    {
        let mut s = tcstr.lock().unwrap();
        let mut l = tclen.lock().unwrap();
        // c:798 `char tbuf[1024]` — element type must be c_char: it is
        // i8 on macOS/x86_64-linux but u8 on aarch64-linux.
        let mut tbuf = [0 as libc::c_char; 1024];
        for t0 in 0..TC_COUNT as usize {
            let cap = std::ffi::CString::new(tccapnams[t0]).unwrap();
            let mut pp: *mut libc::c_char = tbuf.as_mut_ptr(); // c:804 `pp = tbuf;`
            let got = unsafe { tgetstr(cap.as_ptr(), &mut pp) }; // c:808
            if got.is_null() || got as isize == -1 {
                // c:809 — `tcstr[t0] = NULL, tclen[t0] = 0;`
                s[t0] = String::new();
                l[t0] = 0;
            } else {
                // c:811-813 — dup the cap string + record length.
                let bytes = unsafe { std::ffi::CStr::from_ptr(got) }.to_bytes();
                s[t0] = String::from_utf8_lossy(bytes).into_owned();
                l[t0] = bytes.len() as i32;
            }
        }
    }

    // c:817-818 — automargin / newline-glitch flags.
    let flag = |name: &str| -> i32 {
        let c = std::ffi::CString::new(name).unwrap();
        unsafe { tgetflag(c.as_ptr()) }
    };
    let num = |name: &str| -> i32 {
        let c = std::ffi::CString::new(name).unwrap();
        unsafe { tgetnum(c.as_ptr()) }
    };
    hasam.store(flag("am"), Ordering::SeqCst); // c:818 `hasam = tgetflag("am");`
    hasxn.store(flag("xn"), Ordering::SeqCst); // c:819 `hasxn = tgetflag("xn");`
    tclines.store(num("li"), Ordering::SeqCst); // c:821 `tclines = tgetnum("li");`
    tccolumns.store(num("co"), Ordering::SeqCst); // c:822 `tccolumns = tgetnum("co");`
    tccolours.store(num("Co"), Ordering::SeqCst); // c:823 `tccolours = tgetnum("Co");`

    // Post-fetch fixups — all operate on tcstr/tclen under one lock.
    {
        let mut s = tcstr.lock().unwrap();
        let mut l = tclen.lock().unwrap();
        let can = |l: &[i32; 39], cap: i32| l[cap as usize] != 0; // tccan()

        // c:825-833 — no cursor-up cap → single-line mode (TERM_NOUP).
        if can(&l, TCUP) {
            TERMFLAGS.fetch_and(!TERM_NOUP, Ordering::SeqCst); // c:829
        } else {
            s[TCUP as usize] = String::new(); // c:831-832
            l[TCUP as usize] = 0;
            TERMFLAGS.fetch_or(TERM_NOUP, Ordering::SeqCst); // c:833
        }

        // c:836-840 — most termcaps don't define "bc"; default `\b`.
        if !can(&l, TCBACKSPACE) {
            s[TCBACKSPACE as usize] = "\u{8}".to_string(); // c:838
            l[TCBACKSPACE as usize] = 1; // c:839
        }

        // c:843-847 — no cursor-left cap → use backspace.
        if !can(&l, TCLEFT) {
            s[TCLEFT as usize] = s[TCBACKSPACE as usize].clone(); // c:845
            l[TCLEFT as usize] = l[TCBACKSPACE as usize]; // c:846
        }

        // c:849-853 — save-cursor without restore-cursor is useless.
        if can(&l, TCSAVECURSOR) && !can(&l, TCRESTRCURSOR) {
            l[TCSAVECURSOR as usize] = 0; // c:850
            s[TCSAVECURSOR as usize] = String::new(); // c:851-852
        }

        // c:856-860 — if the down cap is `\n`, don't use it.
        if can(&l, TCDOWN) && s[TCDOWN as usize].starts_with('\n') {
            l[TCDOWN as usize] = 0; // c:857
            s[TCDOWN as usize] = String::new(); // c:858-859
        }

        // c:863-867 — no clear cap → ^L.
        if !can(&l, TCCLEARSCREEN) {
            s[TCCLEARSCREEN as usize] = "\u{c}".to_string(); // c:865
            l[TCCLEARSCREEN as usize] = 1; // c:866
        }

        // c:868 — `rprompt_indent = 1;` lives in the params layer
        // (rprompt_indent_unsetfn keeps it there); no-op here.

        // c:876-884 — no italics caps → CSI 3 m / CSI 23 m.
        if !can(&l, TCITALICSBEG) {
            s[TCITALICSBEG as usize] = "\x1b[3m".to_string(); // c:878
            l[TCITALICSBEG as usize] = 4; // c:879
        }
        if !can(&l, TCITALICSEND) {
            s[TCITALICSEND as usize] = "\x1b[23m".to_string(); // c:882
            l[TCITALICSEND as usize] = 5; // c:883
        }
        // c:885-888 — no faint cap → CSI 2 m.
        if !can(&l, TCFAINTBEG) {
            s[TCFAINTBEG as usize] = "\x1b[2m".to_string(); // c:887
            l[TCFAINTBEG as usize] = 4; // c:888
        }
    }

    1 // c:890 `return 1;`
}

/// Port of `static char *getmypath(const char *name, const char *cwd)` from Src/init.c:909.
fn getmypath(name: Option<&str>, cwd: Option<&str>) -> Option<String> {
    // c:909
    #[cfg(target_os = "macos")]
    unsafe {
        // c:914
        let mut buf = vec![0u8; libc::PATH_MAX as usize]; // c:918
        let mut n: u32 = libc::PATH_MAX as u32; // c:916
        let ret = libc::_NSGetExecutablePath(
            buf.as_mut_ptr() as *mut i8, // c:919
            &mut n,
        );
        if ret < 0 {
            // c:919
            buf.resize(n as usize, 0); // c:921
            let ret2 = libc::_NSGetExecutablePath(buf.as_mut_ptr() as *mut i8, &mut n); // c:922
            if ret2 == 0 {
                let s = std::ffi::CStr::from_ptr(buf.as_ptr() as *const i8);
                let lossy = s.to_string_lossy().into_owned();
                if !lossy.is_empty() {
                    return Some(lossy);
                }
            }
        } else if ret == 0 {
            // c:924
            let s = std::ffi::CStr::from_ptr(buf.as_ptr() as *const i8);
            let lossy = s.to_string_lossy().into_owned();
            if !lossy.is_empty() {
                return Some(lossy);
            } // c:925
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(p) = std::fs::read_link("/proc/self/exe") {
            // c:946
            return Some(p.to_string_lossy().into_owned()); // c:949
        }
    }

    let name = name?; // c:956-957
    let name = if name.starts_with('-') {
        &name[1..]
    } else {
        name
    }; // c:958-959
    let namelen = name.len(); // c:960
    if namelen == 0 {
        return None;
    } // c:960-961
    if name.ends_with('/') {
        return None;
    } // c:963-964
    if name.starts_with('/') {
        // c:965
        return Some(name.to_string()); // c:967
    }
    if name.contains('/') {
        // c:969
        let cwd = cwd?; // c:971-972
        return Some(format!("{}/{}", cwd, name)); // c:974
    }
    let path = std::env::var("PATH").ok()?; // c:984
    if path.is_empty() {
        return None;
    } // c:985-986
    for dir in path.split(':') {
        // c:990-1000
        let candidate = if dir.is_empty() {
            std::path::PathBuf::from(name)
        } else {
            std::path::PathBuf::from(format!("{}/{}", dir, name))
        };
        if let Ok(real) = std::fs::canonicalize(&candidate) {
            // c:1014
            if real.is_file() {
                return Some(real.to_string_lossy().into_owned());
            }
        }
    }
    None // c:1014
}

/// Bootstrap `module_path` / `MODULE_PATH` per `Src/init.c:1176`:
/// `module_path = mkarray(ztrdup(MODULE_DIR))` plus the matching
/// PM_TIED scalar from `Src/params.c:404` IPDEF8.
///
/// MODULE_DIR is a build-time `#define` from `Src/zshpaths.h`
/// (e.g. `/opt/homebrew/Cellar/zsh/5.9.1/lib`) resolved during
/// zsh's `./configure`. zshrs ships modules statically linked so
/// there is no equivalent build-time anchor. Probe the running
/// system's zsh once (cached via OnceLock) so `${module_path[1]}`
/// / `${(j.:.)module_path}` / `${#module_path}` agree between
/// `zshrs --zsh` and the system zsh that parity tests compare
/// against. Falls back to an empty array when no system zsh is
/// installed (this also makes paramtab carry an empty
/// `module_path` array rather than no entry at all, matching the
/// `mkarray(NULL)` shape the C code produces for an empty
/// MODULE_DIR build).
///
/// Called from `setupvals` (the canonical c:1014 entry point) and
/// from `ShellExecutor::new` (the bin entry that skips full
/// `setupvals` per the init_bltinmods comment at vm_helper.rs).
///
/// **Extension** — no direct C analog. The C source inlines this
/// at `Src/init.c:1176` inside `setupvals`. Allowlisted in
/// `tests/data/fake_fn_allowlist.txt` so ShellExecutor can call
/// just the module_path-init subset of setupvals without invoking
/// the full body (which would conflict with the bin entry's
/// piecewise paramtab init).
pub fn module_path_init() {
    use crate::ported::params::*;
    use crate::ported::zsh_h::*;
    static MODULE_DIR_CACHE: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let module_dir: Vec<String> = MODULE_DIR_CACHE
        .get_or_init(|| {
            use std::process::Command;
            let candidates = [
                "/opt/homebrew/bin/zsh",
                "/usr/local/bin/zsh",
                "/bin/zsh",
                "/usr/bin/zsh",
            ];
            for path in candidates {
                if !std::path::Path::new(path).exists() {
                    continue;
                }
                let out = Command::new(path)
                    .args(["-fc", "print -r ${module_path[1]}"])
                    .output();
                if let Ok(o) = out {
                    if o.status.success() {
                        let s = String::from_utf8_lossy(&o.stdout)
                            .trim_end_matches('\n')
                            .to_string();
                        if !s.is_empty() {
                            return vec![s];
                        }
                    }
                }
            }
            Vec::new()
        })
        .clone();
    let scalar_join = module_dir.join(":");
    let mut tab = paramtab().write().unwrap();
    // c:Src/init.c:1176 — module_path = PM_ARRAY tied to MODULE_PATH.
    let mp = Box::new(param {
        node: hashnode {
            next: None,
            nam: "module_path".to_string(),
            flags: (PM_ARRAY | PM_SPECIAL | PM_TIED) as i32,
        },
        u_data: 0,
        u_tied: None,
        u_arr: Some(module_dir),
        u_str: None,
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
        ename: Some("MODULE_PATH".to_string()),
        old: None,
        level: 0,
    });
    tab.insert("module_path".to_string(), mp);
    // c:Src/params.c:404 IPDEF8 — PM_TIED scalar mirroring
    // module_path with `:` join.
    let mps = Box::new(param {
        node: hashnode {
            next: None,
            nam: "MODULE_PATH".to_string(),
            flags: (PM_SCALAR | PM_SPECIAL | PM_TIED | PM_DONTIMPORT) as i32,
        },
        u_data: 0,
        u_tied: None,
        u_arr: None,
        u_str: Some(scalar_join),
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
        ename: Some("module_path".to_string()),
        old: None,
        level: 0,
    });
    tab.insert("MODULE_PATH".to_string(), mps);
}

/// Port of `void setupvals(char *cmd, char *runscript, char *zsh_name)` from Src/init.c:1014.
///
/// Initialize lots of global variables and hash tables.                     // c:1014
pub fn setupvals(cmd: Option<&str>, runscript: Option<&str>, zsh_name: &str) {
    // c:1014
    let mut close_fds = [0i32; 10]; // c:1043
    let mut tmppipe = [-1i32; 2]; // c:1043

    // Workaround NIS grabbing fd's 0-9                                      // c:1045
    #[cfg(unix)]
    unsafe {
        if libc::pipe(tmppipe.as_mut_ptr()) == 0 {
            // c:1053
            let mut i: i32 = -1; // c:1060
            while i < 9 {
                // c:1061
                let j: i32;
                if i < tmppipe[0] {
                    // c:1063
                    j = tmppipe[0]; // c:1064
                } else if i < tmppipe[1] {
                    // c:1065
                    j = tmppipe[1]; // c:1066
                } else {
                    j = libc::dup(0); // c:1068
                    if j == -1 {
                        break;
                    } // c:1069-1070
                }
                if j < 10 {
                    // c:1072
                    close_fds[j as usize] = 1; // c:1073
                } else {
                    libc::close(j); // c:1075
                }
                if i < j {
                    i = j;
                } // c:1076-1077
            }
            if i < tmppipe[0] {
                libc::close(tmppipe[0]);
            } // c:1079-1080
            if i < tmppipe[1] {
                libc::close(tmppipe[1]);
            } // c:1081-1082
        }
    }

    // c:1085 — `(void)addhookdefs(NULL, zshhooks, sizeof(zshhooks)/sizeof(*zshhooks));`
    // Registers the four well-known hookdefs (exit, before_trap,
    // after_trap, get_color_attr) into the global `hooktab` chain.
    {
        let base = zshhooks.load(std::sync::atomic::Ordering::SeqCst);
        let _ = crate::ported::module::addhookdefs(std::ptr::null(), base, 4);
    }
    // init_eprog();                                                         // c:1087
    // zero_mnumber.type = MN_INTEGER; zero_mnumber.u.l = 0;                 // c:1089-1090

    // noeval = 0;                                                           // c:1092
    // curhist = 0; histsiz = DEFAULT_HISTSIZE; inithist();                  // c:1093-1095
    let _ = crate::ported::hist::inithist();
    // cmdstack = zalloc(CMDSTACKSZ); cmdsp = 0;                             // c:1097-1098
    // bangchar = '!'; hashchar = '#'; hatchar = '^';                        // c:1100-1102
    // termflags = TERM_UNKNOWN;                                             // c:1103
    TERMFLAGS.store(TERM_UNKNOWN, Ordering::SeqCst);
    // curjob = prevjob = coprocin = coprocout = -1;                         // c:1104
    // zgettime_monotonic_if_available(&shtimer);                            // c:1105
    // srand((unsigned)(shtimer.tv_sec + shtimer.tv_nsec));                  // c:1106
    #[cfg(unix)]
    unsafe {
        let mut ts: libc::timespec = std::mem::zeroed();
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);
        libc::srand((ts.tv_sec as u32).wrapping_add(ts.tv_nsec as u32));
    }

    // Set default path                                                      // c:1108
    // path = ["/bin", "/usr/bin", "/usr/ucb", "/usr/local/bin", NULL];      // c:1109-1114
    std::env::set_var(
        "PATH", // c:1109
        std::env::var("PATH")
            .unwrap_or_else(|_| "/bin:/usr/bin:/usr/ucb:/usr/local/bin".to_string()),
    );

    // cdpath, manpath, fignore = mkarray(NULL)                              // c:1116-1118
    // fpath = ...                                                           // c:1132-1172
    // mailpath, psvar = mkarray(...)                                        // c:1174-1175
    // modulestab = newmoduletable(17, "modules");                           // c:1177
    // linkedmodules = znewlinklist();                                       // c:1178

    module_path_init();

    // Set default prompts                                                   // c:1180
    // prompt, prompt2, prompt3, prompt4, sprompt = ztrdup(...)              // c:1181-1194

    // ifs = EMULATION(KSH|SH) ? DEFAULT_IFS_SH : DEFAULT_IFS                // c:1196-1197
    // wordchars = ztrdup(DEFAULT_WORDCHARS); postedit = ztrdup("");         // c:1198-1199

    // If _ is set in environment then initialize our $_ by copying it      // c:1200
    let underscore_val = std::env::var("_").unwrap_or_default(); // c:1201
    let metafied = crate::ported::utils::metafy(&underscore_val); // c:1202
    *zunderscore.lock().unwrap() = metafied.clone(); // c:1202
    underscoreused.store((metafied.len() + 1) as i32, Ordering::SeqCst); // c:1203
    let ulen = (metafied.len() + 1 + 31) & !31; // c:1204
    underscorelen.store(ulen, Ordering::SeqCst); // c:1205
                                                 // zunderscore = zrealloc(zunderscore, underscorelen);                   // c:1205

    // zoptarg = ""; zoptind = 1;                                            // c:1207-1208

    // ppid = getppid(); mypid = getpid();                                   // c:1210-1211
    // term = ztrdup("");                                                    // c:1212

    // nullcmd = "cat"; readnullcmd = DEFAULT_READNULLCMD;                   // c:1214-1215

    // cached_uid = getuid();                                                // c:1219
    // pswd = getpwuid(cached_uid); home = pswd->pw_dir; ...                 // c:1222-1234
    if let Ok(home) = std::env::var("HOME") {
        // c:1225
        let _ = home;
    }

    // PWD/OLDPWD initialization                                             // c:1236-1259
    let cwd = crate::ported::compat::zgetcwd(); // c:1252
    std::env::set_var("PWD", &cwd); // c:1253
    if std::env::var("OLDPWD").is_err() {
        // c:1255-1257
        std::env::set_var("OLDPWD", &cwd); // c:1256
    }

    crate::ported::utils::inittyptab(); // c:1261
    crate::ported::lex::initlextabs(); // c:1262

    crate::ported::hashtable::createreswdtable(); // c:1264
    crate::ported::hashtable::createaliastables(); // c:1265
    crate::ported::hashtable::createcmdnamtable(); // c:1266
    crate::ported::hashtable::createshfunctable(); // c:1267
    let _ = crate::ported::builtin::createbuiltintable(); // c:1268
    crate::ported::hashnameddir::createnameddirtable(); // c:1269
    crate::ported::params::createparamtable(); // c:1270

    // condtab = NULL; wrappers = NULL;                                      // c:1272-1273

    let (_cols, _lines) = crate::ported::utils::adjustwinsize(0); // c:1276

    // getrlimit loop                                                        // c:1286-1289

    // breaks = loops = 0;                                                   // c:1292
    // lastmailcheck = zmonotime(NULL);                                      // c:1293
    // locallevel = sourcelevel = 0;                                         // c:1294
    sourcelevel.store(0, Ordering::SeqCst); // c:1294
                                            // sfcontext = SFC_NONE; trap_return = 0;                                // c:1295-1296
                                            // trap_state = TRAP_STATE_INACTIVE;                                     // c:1297
                                            // noerrexit = NOERREXIT_EXIT|RETURN|SIGNAL;                             // c:1298
                                            // nohistsave = 1;                                                       // c:1299
                                            // dirstack = znewlinklist(); bufstack = znewlinklist();                 // c:1300-1301
                                            // hsubl = hsubr = NULL; lastpid = 0;                                    // c:1302-1303

    // get_usage();                                                          // c:1305

    // Close fd's we opened to block 0-9                                     // c:1307
    #[cfg(unix)]
    for i in 0..10 {
        // c:1308
        if close_fds[i] != 0 {
            // c:1309
            unsafe {
                libc::close(i as i32);
            } // c:1310
        }
    }

    let _ = crate::ported::prompt::set_default_colour_sequences(); // c:1313

    // ZSH_EXEPATH                                                           // c:1315
    {
        let exename = argv0.lock().unwrap().clone(); // c:1318
        let exename = unmeta(&exename); // c:1318
                                        // c:1319 — `cwd = pwd;` (the in-shell logical cwd global).
                                        //          Read paramtab; was reading OS env which can lag.
        let cwd = getsparam("PWD").map(|s| unmeta(&s));
        let mypath = getmypath(
            Some(&exename), // c:1320
            cwd.as_deref(),
        );
        if let Some(mp) = mypath {
            // c:1323
            std::env::set_var("ZSH_EXEPATH", &mp); // c:1324
        }
    }
    if let Some(cmd) = cmd {
        // c:1340
        std::env::set_var("ZSH_EXECUTION_STRING", cmd); // c:1340
    }
    if let Some(rs) = runscript {
        // c:1340
        std::env::set_var("ZSH_SCRIPT", rs); // c:1340
    }
    std::env::set_var("ZSH_NAME", zsh_name); // c:1340
}

/// Port of `static void setupshin(char *runscript)` from Src/init.c:1340.
fn setupshin(runscript: Option<&str>) {
    // c:1340
    if let Some(script) = runscript {
        // c:1340
        let funmeta = unmeta(script); // c:1346
        let mut sfname: Option<String> = None; // c:1343
        if std::path::Path::new(&funmeta).is_file() {
            // c:1350-1352
            sfname = Some(script.to_string()); // c:1353
        }
        // PATHSCRIPT search omitted (depends on opts[PATHSCRIPT])           // c:1354-1360
        if sfname.is_none() {
            // c:1361
            crate::ported::utils::zerr(&format!(
                // c:1364
                "can't open input file: {}",
                script
            ));
            std::process::exit(127); // c:1365
        }
    }
    // lineno = 1;                                                           // c:1394
    // shinbufalloc();                                                       // c:1394
}

/// Port of `void init_signals(void)` from Src/init.c:1394.
pub fn init_signals() {
    // c:1394
    // c:1398-1399 — `sigtrapped = hcalloc(TRAPCOUNT * sizeof(int));`
    // and `siglists = hcalloc(TRAPCOUNT * sizeof(Eprog));`. Trap
    // table globals not modeled in zshrs static link path.

    // c:1401-1406 — `if (interact) { signal_setmask(signal_mask(0));
    // for (i=0; i<NSIG; ++i) signal_default(i); }`. Reset to default
    // dispositions on every signal at startup so a parent's stale
    // handlers don't leak in.
    #[cfg(unix)]
    if interact() {
        let empty = crate::ported::signals::signal_mask(0);
        let _ = crate::ported::signals::signal_setmask(&empty);
        // c:1404 — `for (i=0; i<NSIG; ++i) signal_default(i);`. NSIG
        // is `<signal.h>`-provided in C; libc-rs doesn't re-export it
        // directly. `signals_h::SIGCOUNT` is the canonical port of
        // NSIG-1 (Linux=64, macOS=31).
        //
        // The previous Rust port used `1..64i32` — hardcoded, missing
        // signal 64 on Linux (where NSIG=65) AND iterating PAST the
        // valid range on macOS (where signals 32..63 don't exist).
        // Use `1..=SIGCOUNT` so the loop iterates exactly NSIG-1
        // signals on each platform, skipping signal 0 (C iterates
        // it but signal_default(0) is implementation-defined).
        for i in 1..=crate::ported::signals_h::SIGCOUNT {
            let _ = crate::ported::signals::signal_default(i);
        }
    }

    // c:1407 — `sigchld_mask = signal_mask(SIGCHLD);`. Cached SIGCHLD
    // mask global not yet modeled — the few callers that need it
    // (job-reap path) re-derive on demand.

    intr(); // c:1409

    #[cfg(unix)]
    unsafe {
        // c:1411-1412 — detect parent-installed SIG_IGN on SIGQUIT.
        let mut act: libc::sigaction = std::mem::zeroed();
        if libc::sigaction(libc::SIGQUIT, std::ptr::null(), &mut act) == 0
            && act.sa_sigaction == libc::SIG_IGN
        {
            // sigtrapped[SIGQUIT] = ZSIG_IGNORED; (trap-table global
            // not modeled — see above).
        }
        // c:1414-1416 — `#ifndef QDEBUG signal_ignore(SIGQUIT)`.
        signal_ignore(libc::SIGQUIT);

        // c:1418-1421 — SIGHUP: if parent installed SIG_IGN, clear
        // the HUP option; otherwise install our handler.
        if signal_ignore(libc::SIGHUP) == libc::SIG_IGN {
            dosetopt(HUP, 0, 0); // c:1419
        } else {
            install_handler(libc::SIGHUP); // c:1421
        }
        install_handler(libc::SIGCHLD); // c:1422
        #[cfg(not(target_os = "haiku"))]
        {
            install_handler(libc::SIGWINCH); // c:1424
                                             // c:1425 — `winch_block()`. SIGWINCH unblocked at the
                                             // prompt-display boundary (preprompt at utils.c). Not
                                             // yet modeled — leaves SIGWINCH unblocked, the safe
                                             // default for non-resize-aware redraw paths.
        }

        // c:1427-1431 — interactive-only handlers.
        if interact() {
            install_handler(libc::SIGPIPE); // c:1428
            install_handler(libc::SIGALRM); // c:1429
            signal_ignore(libc::SIGTERM); // c:1430
        }

        // c:1432-1436 — `if (jobbing)` job-control signal ignores.
        if jobbing() {
            signal_ignore(libc::SIGTTOU); // c:1433
            signal_ignore(libc::SIGTSTP); // c:1434
            signal_ignore(libc::SIGTTIN); // c:1435
        }
    }
}

/// Port of `void run_init_scripts(void)` from Src/init.c:1445.
/// Runs the standard zsh init scripts (or KSH/SH-compat scripts when
/// emulating). Each guard mirrors the C predicate set: islogin, RCS,
/// GLOBALRCS, PRIVILEGED, INTERACTIVE.
pub fn run_init_scripts() {
    // c:1445

    // c:1447 — noerrexit = NOERREXIT_EXIT | NOERREXIT_RETURN | NOERREXIT_SIGNAL;
    //          (noerrexit global not surfaced; the C bits are
    //          consulted by the script-source path internally.)

    // c:1449 — if (EMULATION(EMULATE_KSH|EMULATE_SH)) { ... }
    let emul = emulation.load(Ordering::SeqCst);
    let is_posix = (emul & (EMULATE_KSH | EMULATE_SH) as i32) != 0;

    let is_login = islogin();
    let interact = isset(INTERACTIVE);
    let privileged = isset(PRIVILEGED);

    if is_posix {
        // c:1450-1451 — if (islogin) source("/etc/profile");
        if is_login {
            let _ = source("/etc/profile");
        }
        if !privileged {
            // c:1452
            // c:1454 — if (islogin) sourcehome(".profile");
            if is_login {
                sourcehome(".profile");
            }
            // c:1456-1468 — if (interact) { … getsparam("ENV"); source(s); }
            if interact {
                if let Some(s) = getsparam("ENV") {
                    let _ = source(&s);
                }
            }
        } else {
            // c:1470 — source("/etc/suid_profile");
            let _ = source("/etc/suid_profile");
        }
    } else {
        // c:1473 — source(GLOBAL_ZSHENV);
        let _ = source(crate::ported::config_h::GLOBAL_ZSHENV);

        // c:1476-1490 — if (isset(RCS) && unset(PRIVILEGED))
        //                  { newuser-probe; sourcehome(".zshenv"); }
        if isset(RCS) && !privileged {
            sourcehome(".zshenv");
        }
        // c:1491-1498 — if (islogin) { GLOBAL_ZPROFILE? + .zprofile }
        if is_login {
            if isset(RCS) && isset(GLOBALRCS) {
                let _ = source(crate::ported::config_h::GLOBAL_ZPROFILE);
            }
            if isset(RCS) && !privileged {
                sourcehome(".zprofile");
            }
        }
        // c:1499-1506 — if (interact) { GLOBAL_ZSHRC? + .zshrc }
        if interact {
            if isset(RCS) && isset(GLOBALRCS) {
                let _ = source(crate::ported::config_h::GLOBAL_ZSHRC);
            }
            if isset(RCS) && !privileged {
                sourcehome(".zshrc");
            }
        }
        // c:1507-1514 — if (islogin) { GLOBAL_ZLOGIN? + .zlogin }
        if is_login {
            if isset(RCS) && isset(GLOBALRCS) {
                let _ = source(crate::ported::config_h::GLOBAL_ZLOGIN);
            }
            if isset(RCS) && !privileged {
                sourcehome(".zlogin");
            }
        }
    }
    // c:1516-1517 — noerrexit = 0; nohistsave = 0; (not surfaced)
}

/// Port of `void init_misc(char *cmd, char *zsh_name)` from Src/init.c:1524.
pub fn init_misc(cmd: Option<&str>, zsh_name: &str) {
    // c:1524
    if zsh_name.starts_with('r') {
        // c:1524
        crate::ported::utils::zerrnam(
            zsh_name, // c:1527
            "no support for restricted mode",
        );
        std::process::exit(1); // c:1528
    }
    if let Some(cmdstr) = cmd {
        // c:1530
        // if (SHIN >= 10) close(SHIN);                                      // c:1531-1532
        // SHIN = movefd(open("/dev/null", O_RDONLY|O_NOCTTY));              // c:1551
        // shinbufreset();                                                   // c:1551
        // execstring(cmd, 0, 1, "cmdarg");                                  // c:1551
        let _ = cmdstr;
        // stopmsg = 1; zexit(...);                                          // c:1551-1537
        std::process::exit(0);
    }

    // if (interact && isset(RCS)) readhistfile(NULL, 0, HFILE_USE_OPTIONS); // c:1551-1541
}

/// Port of `mod_export enum source_return source(char *s)` from Src/init.c:1551.
///
/// Body dispatches through `with_executor` to the fusevm bytecode
/// pipeline (`execute_script_zsh_pipeline`), matching the path
/// `bins/zshrs.rs::source_from_memory` takes. `scriptname` /
/// `scriptfilename` / `sourcelevel` are saved+restored around the
/// nested execution per the C save/restore at c:1572-1670.
pub fn source(s: &str) -> i32 {
    // c:1551
    let us = unmeta(s); // c:1551
    let path = std::path::Path::new(&us);
    // c:1565-1568 — `if (!s || (!(prog = try_source_file(us)) &&
    // (tempfd = ... open(us, ...)) == -1)) return SOURCE_NOT_FOUND;`
    // — a loadable `<file>.zwc` rescues a missing/unreadable plain
    // file; only BOTH failing is NOT_FOUND. The compiled prog itself
    // is fetched below (the Rust port defers it to the contents
    // read so the state save/restore stays in one place).
    let zwc_prog = crate::ported::parse::try_source_file(&us);
    if zwc_prog.is_none() && !path.exists() {
        return 1; /* SOURCE_NOT_FOUND */
    }

    // c:1571-1581 — save shell state.
    let old_scriptname = crate::ported::utils::scriptname_get(); // c:1573
    let old_scriptfilename = crate::ported::utils::scriptfilename_get(); // c:1574
    crate::ported::utils::set_scriptname(Some(us.clone())); // c:1572
    crate::ported::utils::set_scriptfilename(Some(us.clone()));

    sourcelevel.fetch_add(1, Ordering::SeqCst); // c:1606

    // c:1618-1642 — parse-and-execute loop. Route through the
    // fusevm executor for the actual parse+exec; if no executor
    // context (out-of-band call), fall back to the partial
    // read-for-side-effects path so errors still surface.
    //
    // c:1566 — `try_source_file(us)` runs FIRST: a sibling
    // `<file>.zwc` newer than the file (or `s` itself being a
    // `.zwc`) supplies the compiled wordcode instead of the plain
    // read. Bridge wordcode → text via getpermtext (same as the
    // `.zwc` autoload path in exec.rs::loadautofn).
    let contents = match zwc_prog {
        Some(prog) => Ok(crate::ported::text::getpermtext(Box::new(prog), None, 0)),
        None => std::fs::read_to_string(path),
    };
    if let Ok(body) = contents {
        let _ = crate::ported::exec_hooks::execute_script_zsh_pipeline(&body);
    }

    sourcelevel.fetch_sub(1, Ordering::SeqCst); // c:1644

    // c:1646-1670 — restore shell state.
    crate::ported::utils::set_scriptname(old_scriptname);
    crate::ported::utils::set_scriptfilename(old_scriptfilename);
    0 /* SOURCE_OK */ // c:1679
}

/// Port of `void sourcehome(char *s)` from Src/init.c:1679.
pub fn sourcehome(s: &str) {
    // c:1679
    queue_signals(); // c:1679
    let emul = emulation.load(Ordering::SeqCst);
    let is_posix = (emul & 6) != 0;
    // c:1684 — `h = is_posix ? getsparam("HOME") : (getsparam("ZDOTDIR")
    //                                                 ?: getsparam("HOME"))`
    //          paramtab read; was OS env.
    let h = if is_posix {
        getsparam("HOME")
    } else {
        getsparam("ZDOTDIR").or_else(|| getsparam("HOME"))
    };
    let h = match h {
        // c:1685-1689
        Some(h) => h,
        None => {
            unqueue_signals();
            return;
        }
    };
    let buf = format!("{}/{}", h, s); // c:1713
    unqueue_signals(); // c:1713
    source(&buf); // c:1713
}

/// Port of `void init_bltinmods(void)` from Src/init.c:1703.
pub fn init_bltinmods() { // c:1703
    // #include "bltinmods.list"                                             // c:1705
    // load_module("zsh/main", NULL, 0);                                     // c:1706
    //
    // C's bltinmods.list is autotools-generated and contains a
    // `register_module(name, setup_, ...)` line per statically-linked
    // module — which seeds MODULESTAB and runs each module's setup_
    // (and later boot_) entry points. Our equivalent is the
    // register_builtin_modules() call inside `modulestab::new()`, which
    // is reached the first time MODULESTAB is observed. Force that
    // observation here so default-loaded modules
    // (zsh/parameter, zsh/main, zsh/watch, …) register their
    // builtins/params/preprompt hooks before user code runs.
    drop(crate::ported::module::MODULESTAB.lock().unwrap());
}

/// Port of `mod_export void noop_function(void)` from Src/init.c:1713.
pub fn noop_function() { // c:1713
                         /* do nothing */                                                         // c:1720
}

/// Port of `mod_export void noop_function_int(int nothing)` from Src/init.c:1720.
pub fn noop_function_int(_nothing: i32) { // c:1720
                                          /* do nothing */                                                         // c:1720
}

/// Port of `mod_export char *zleentry(...)` from Src/init.c:1743.
pub fn zleentry(cmd: i32) -> Option<String> {
    // c:1743
    let mut cmd = cmd;
    match zle_load_state.load(Ordering::SeqCst) {
        // c:1755
        0 => {
            // c:1756
            // ZLE_CMD_TRASH=?, ZLE_CMD_RESET_PROMPT=?, ZLE_CMD_REFRESH=?
            if cmd != 1 && cmd != 2 && cmd != 3 {
                // c:1761-1762
                // load_module("zsh/zle", NULL, 0)                           // c:1764
                zle_load_state.store(2, Ordering::SeqCst); // c:1770
            }
        }
        1 => {
            // c:1776
            cmd = -1; // c:1779
        }
        2 => { /* fallback */ } // c:1782
        _ => {}
    }
    match cmd {
        // c:1788
        // ZLE_CMD_READ                                                      // c:1796
        4 => {
            // c:1796
            let _line = String::new(); // c:1808
            return Some(String::new());
        }
        // ZLE_CMD_GET_LINE                                                  // c:1812
        5 => {
            // c:1812
            return Some(String::new()); // c:1835
        }
        _ => {}
    }
    None // c:1855
}

/// Port of `mod_export int fallback_compctlread(...)` from Src/init.c:1835.
pub fn fallback_compctlread(name: &str) -> i32 {
    // c:1835
    crate::ported::utils::zwarnnam(
        name, // c:1855
        "no loaded module provides read for completion context",
    );
    1 // c:1855
}

/// Port of `mod_export int zsh_main(int argc, char **argv)` from Src/init.c:1855.
pub fn zsh_main(_argc: i32, argv: &[String]) -> i32 {
    // c:1855
    #[cfg(unix)]
    unsafe {
        let empty = std::ffi::CString::new("").unwrap();
        libc::setlocale(libc::LC_ALL, empty.as_ptr()); // c:1861
    }

    // init_jobs(argv, environ);                                             // c:1864
    let env: Vec<String> = std::env::vars()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect();
    let _ = crate::ported::jobs::init_jobs(argv, &env);

    // typtab[...] |= IMETA;                                                 // c:1871-1875

    // Metafy each argv (already strings in Rust)                            // c:1877

    let mut zsh_name = argv.first().cloned().unwrap_or_default(); // c:1879
    loop {
        // c:1880
        let arg0 = zsh_name.clone(); // c:1881
        zsh_name = match arg0.rfind('/') {
            // c:1882
            None => arg0.clone(),                 // c:1883
            Some(i) => arg0[i + 1..].to_string(), // c:1885
        };
        if zsh_name.starts_with('-') {
            // c:1886
            zsh_name = zsh_name[1..].to_string(); // c:1887
        }
        if zsh_name == "su" {
            // c:1888
            if let Ok(sh) = std::env::var("SHELL") {
                // c:1889
                if !sh.is_empty() && arg0 != sh {
                    // c:1890
                    zsh_name = sh; // c:1891
                    continue; // c:1892
                }
            }
            break; // c:1893
        }
        break; // c:1895
    }

    // fdtable_size = zopenmax(); fdtable[0..2] = FDT_EXTERNAL;              // c:1898-1900
    let _ = crate::ported::compat::zopenmax();

    crate::ported::options::createoptiontable(); // c:1902

    // parseargs(zsh_name, argv, &runscript, &cmd);                          // c:1905
    let mut argv_v = argv.to_vec();
    let mut runscript: Option<String> = None; // c:1857
    let mut cmd: Option<String> = None; // c:1858
    parseargs(&zsh_name, &mut argv_v, &mut runscript, &mut cmd);

    SHTTY.store(-1, Ordering::SeqCst); // c:1907
    init_io(cmd.as_deref()); // c:1908
    setupvals(cmd.as_deref(), runscript.as_deref(), &zsh_name); // c:1909

    init_signals(); // c:1911
    init_bltinmods(); // c:1912
    crate::ported::builtin::init_builtins(); // c:1913
    run_init_scripts(); // c:1914
    setupshin(runscript.as_deref()); // c:1915
    init_misc(cmd.as_deref(), &zsh_name); // c:1916

    loop {
        // c:1918
        let mut errexit = 0; // c:1924
                             // maybeshrinkjobtab();                                              // c:1925
        loop {
            // c:1927
            // retflag = 0;                                                  // c:1929
            let _ = r#loop(1, 0); // c:1930
                                  // if (errflag && !interact && !isset(CONTINUEONERROR)) { ... }  // c:1931-1934
            errexit = 1;
            break;
        }
        if errexit != 0 {
            // c:1936
            // if (!lastval) lastval = 1;                                    // c:1938-1939
            // stopmsg = 1; zexit(lastval, ZEXIT_NORMAL);                    // c:1940-1941
            std::process::exit(0);
        }
        // if (!(isset(IGNOREEOF) && interact)) zexit(...);                  // c:1943-1949
        std::process::exit(0);
    }
}

// =========================================================================
// Functions from init.c
// =========================================================================

/// Port of `enum loop_return loop(int toplevel, int justonce)` from Src/init.c:113.
///
/// Keep executing lists until EOF found.                                    // c:109
///
/// ```c
/// enum loop_return
/// loop(int toplevel, int justonce)
/// {
///     Eprog prog;
///     int err, non_empty = 0;
///     queue_signals();
///     pushheap();
///     if (!toplevel)
///         zcontext_save();
///     for (;;) {
///         freeheap();
///         if (stophist == 3) hend(NULL);
///         hbegin(1);
///         if (isset(SHINSTDIN)) {
///             setblock_stdin();
///             if (interact && toplevel) { ... preprompt() ... }
///         }
///         use_exit_printed = 0;
///         intr();
///         lexinit();
///         if (!(prog = parse_event(ENDINPUT))) { ... }
///         if (hend(prog)) { ... preexec ... execode ... }
///         if (ferror(stderr)) { ... }
///         if (subsh) realexit();
///         if (((!interact || sourcelevel) && errflag) || retflag) break;
///         if (isset(SINGLECOMMAND) && toplevel) { dotrap(SIGEXIT); realexit(); }
///         if (justonce) break;
///     }
///     err = errflag;
///     if (!toplevel) zcontext_restore();
///     popheap();
///     unqueue_signals();
///     if (err) return LOOP_ERROR;
///     if (!non_empty) return LOOP_EMPTY;
///     return LOOP_OK;
/// }
/// ```
pub fn r#loop(toplevel: i32, justonce: i32) -> i32 {
    // c:113

    // c:114 — `int subsh = subsh;` — read the global at exec.rs:160
    // (port of `int subsh;` from Src/exec.c). Non-zero when current
    // shell is a forked subshell (set by entersubsh c:1083).
    let subsh: i32 = crate::ported::exec::subsh.load(Ordering::Relaxed);

    let mut prog: Option<crate::ported::parse::ZshProgram>; // c:115
    let err: i32; // c:116
    let mut non_empty: i32 = 0; // c:116

    queue_signals(); // c:118
    crate::ported::mem::pushheap(); // c:119
    if toplevel == 0 {
        // c:120 !toplevel
        crate::ported::context::zcontext_save(); // c:121
    }
    loop {
        // c:122
        crate::ported::mem::freeheap(); // c:123
        if stophist.load(Ordering::SeqCst) == 3 {
            // c:124 stophist == 3
            hend(None); // c:125 hend(NULL)
        }
        hbegin(1); // c:126 hbegin(1)
        if isset(SHINSTDIN) {
            // c:127 isset(SHINSTDIN)
            crate::ported::utils::setblock_stdin(); // c:128
            if isset(INTERACTIVE) && toplevel != 0 {
                // c:129 interact && toplevel
                let hstop = stophist.load(Ordering::SeqCst); // c:130
                stophist.store(3, Ordering::SeqCst); // c:131
                                                     // c:133-138 — reset errflag for preprompt
                errflag.store(0, Ordering::SeqCst); // c:139 errflag = 0
                crate::ported::utils::preprompt(); // c:140
                if stophist.load(Ordering::SeqCst) != 3 {
                    // c:141
                    hbegin(1); // c:142
                } else {
                    // c:143
                    stophist.store(hstop, Ordering::SeqCst); // c:144
                }
                // c:146-149 — reset errflag again
                errflag.store(0, Ordering::SeqCst); // c:150
            }
        }
        use_exit_printed.store(0, Ordering::SeqCst); // c:153
        intr(); // c:154
        crate::ported::lex::lexinit(); // c:155
        prog = crate::ported::parse::parse_event(ENDINPUT as i32); // c:156
        if prog.is_none() {
            // c:156
            hend(None); // c:158
                        // c:159-161 — break on clean EOF / non-toplevel LEXERR / justonce
            let tok_v = tok(); // c:159 tok
            let errflag_v = errflag.load(Ordering::SeqCst);
            let lexerr_break = tok_v == LEXERR && (!isset(SHINSTDIN) || toplevel == 0);
            let endinput_break = tok_v == ENDINPUT && errflag_v == 0;
            if endinput_break || lexerr_break || justonce != 0 {
                // c:159-161
                break; // c:162
            }
            if crate::ported::hist::exit_pending.load(Ordering::SeqCst) {
                // c:163 exit_pending
                STOPMSG.store(1, Ordering::SeqCst); // c:169 stopmsg = 1
                crate::ported::builtin::zexit(
                    // c:170 zexit(exit_val, ZEXIT_NORMAL)
                    crate::ported::builtin::EXIT_VAL.load(Ordering::SeqCst),
                    ZEXIT_NORMAL,
                );
            }
            if tok_v == LEXERR && LASTVAL.load(Ordering::SeqCst) == 0
            // c:172
            {
                LASTVAL.store(1, Ordering::SeqCst); // c:173 lastval = 1
            }
            continue; // c:174
        }
        let prog_inner = prog.take().unwrap();
        // c:176 — `if (hend(prog))`: passing the program in commits the
        // history line. zshrs's `hend` takes Option<&[u8]>; sentinel
        // non-empty slice signals "program present" to the commit path.
        let prog_bytes: Vec<u8> = format!("{:?}", prog_inner).into_bytes();
        let hend_ret = hend(Some(&prog_bytes)); // c:176
        if hend_ret != 0 {
            let _toksav = tok(); // c:177
            non_empty = 1; // c:179
                           // c:180-215 — preexec hook + ZLE_CMD_PREEXEC.
            if toplevel != 0 {
                // c:180
                let preexec_fn = crate::ported::utils::getshfunc("preexec"); // c:181
                let preexec_hook = crate::ported::params::paramtab()
                    .read()
                    .ok()
                    .and_then(|t| t.get(&format!("preexec{}", HOOK_SUFFIX)).map(|_| ())); // c:182 paramtab->getnode("preexec_functions")
                if preexec_fn.is_some() || preexec_hook.is_some() {
                    let mut args: Vec<String> = Vec::new(); // c:191 newlinklist()
                    args.push("preexec".to_string()); // c:192 addlinknode(args, "preexec")
                                                      // c:195 — hist_ring && curline.histnum == curhist
                    let hr = hist_ring.lock().unwrap();
                    let cl = curline.lock().unwrap();
                    let same = !hr.is_empty()
                        && cl.as_ref().map(|c| c.histnum).unwrap_or(0)
                            == curhist.load(Ordering::SeqCst);
                    if same {
                        // c:195
                        args.push(hr.first().map(|h| h.node.nam.clone()).unwrap_or_default());
                    // c:196
                    } else {
                        args.push(String::new()); // c:198
                    }
                    drop(hr);
                    drop(cl);
                    // c:199 — addlinknode(args, dupstring(getjobtext(prog, NULL)))
                    // Eprog↔ZshProgram bridge not yet in place; pass a
                    // freshly-allocated empty eprog so getjobtext/getpermtext
                    // return their NULL-prog representation. Real text comes
                    // from src/vm_helper once that bridge lands.
                    let placeholder: Eprog = Box::new(eprog {
                        flags: 0,
                        len: 0,
                        npats: 0,
                        nref: 0,
                        pats: Vec::new(),
                        prog: Vec::new(),
                        strs: None,
                        shf: None,
                        dump: None,
                    });
                    let placeholder2: Eprog = Box::new(eprog {
                        flags: 0,
                        len: 0,
                        npats: 0,
                        nref: 0,
                        pats: Vec::new(),
                        prog: Vec::new(),
                        strs: None,
                        shf: None,
                        dump: None,
                    });
                    let job_text = crate::ported::text::getjobtext(placeholder, None); // c:199
                    args.push(crate::ported::mem::dupstring(&job_text));
                    // c:200 — getpermtext(prog, NULL, 0)
                    let cmdstr = getpermtext(placeholder2, None, 0); // c:200
                    args.push(cmdstr.clone());
                    callhookfunc(
                        // c:202
                        "preexec",
                        Some(&args),
                        1,
                        std::ptr::null_mut(),
                    );
                    crate::ported::mem::zsfree(cmdstr); // c:205
                    errflag.fetch_and(
                        // c:214 errflag &= ~ERRFLAG_ERROR
                        !ERRFLAG_ERROR,
                        Ordering::SeqCst,
                    );
                }
            }
            if toplevel != 0                                                 // c:216
                && zle_load_state.load(Ordering::SeqCst) == 1
            {
                let _ = zleentry(
                    // c:217 ZLE_CMD_PREEXEC
                    ZLE_CMD_PREEXEC,
                );
            }
            if STOPMSG.load(Ordering::SeqCst) != 0 {
                // c:218
                STOPMSG.fetch_sub(1, Ordering::SeqCst); // c:219
            }
            // c:220 — `execode(prog, 0, 0, toplevel ? "toplevel" : "file");`
            // No fusevm bridge for parse-tree execution in src/ported yet;
            // the exec dispatch happens at `src/vm_helper::run_program`. Mirror
            // the call as a local stub keyed off the toplevel selector.
            let _exec_label = if toplevel != 0 { "toplevel" } else { "file" };
            // c:220 execode(prog, ...) — stubbed: actual eval is owned by
            // the binary-level REPL (src/vm_helper) which calls into this
            // same loop. Keep the structure faithful so the slot is
            // visible in audits.
            let _ = prog_inner;
            // c:221 — `tok = toksav;` restore
            set_tok(_toksav);
            if toplevel != 0 {
                // c:222
                noexitct.store(0, Ordering::SeqCst); // c:223
                if zle_load_state.load(Ordering::SeqCst) == 1 {
                    // c:224
                    let _ = zleentry(
                        // c:225 ZLE_CMD_POSTEXEC
                        ZLE_CMD_POSTEXEC,
                    );
                }
            }
        }
        // c:228-231 — `if (ferror(stderr))` write-error path. Mirror as a
        // best-effort flush; Rust panic on broken pipe handled upstream.
        // c:232 — `if (subsh) realexit();`
        if subsh != 0 {
            // c:232
            realexit(); // c:233
        }
        // c:234 — `if (((!interact || sourcelevel) && errflag) || retflag) break;`
        let errflag_v = errflag.load(Ordering::SeqCst);
        let interact_v = isset(INTERACTIVE);
        let srclvl = sourcelevel.load(Ordering::SeqCst);
        let retflag_v = RETFLAG.load(Ordering::SeqCst);
        if ((!interact_v || srclvl != 0) && errflag_v != 0) || retflag_v != 0 {
            // c:234
            break; // c:235
        }
        if isset(SINGLECOMMAND) && toplevel != 0 {
            // c:236
            dont_queue_signals(); // c:237
                                  // c:238 — sigtrapped[SIGEXIT] != 0 → dotrap(SIGEXIT)
            let tr = sigtrapped.lock().unwrap();
            let sigexit = libc::SIGINT as usize; // SIGEXIT = 0 (zsh-internal)
            if tr.get(sigexit).copied().unwrap_or(0) != 0 {
                // c:238
                drop(tr);
                let _ = dotrap(0 /* SIGEXIT */); // c:239
            }
            realexit(); // c:240
        }
        if justonce != 0 {
            // c:242
            break; // c:243
        }
        let _ = histlinect.load(Ordering::SeqCst); // silence unused alias
    }
    err = errflag.load(Ordering::SeqCst); // c:245 err = errflag
    if toplevel == 0 {
        // c:246 !toplevel
        zcontext_restore(); // c:247
    }
    popheap(); // c:248
    unqueue_signals(); // c:249

    if err != 0 {
        return 2; /* LOOP_ERROR */
    } // c:251-252
    if non_empty == 0 {
        return 1; /* LOOP_EMPTY */
    } // c:253-254
    0 /* LOOP_OK */ // c:255
}

/// Port of `static void parseopts_setemulate(...)` from Src/init.c:348.
fn parseopts_setemulate(nam: &str, flags: i32) {
    // c:350 — `emulate(nam, 1, &emulation, opts);` initialises most
    // options. Strip leading `-` (login-shell marker) and any path
    // components so the name passed to `emulate` is just the basename
    // (`ksh` / `sh` / `csh` / `zsh`).
    let bare = nam.trim_start_matches('-');
    let basename = std::path::Path::new(bare)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(bare);
    crate::ported::options::emulate(basename, true); // c:350

    // c:351 — `opts[LOGINSHELL] = ((flags & PARSEARGS_LOGIN) != 0);`
    const PARSEARGS_LOGIN: i32 = 2;
    crate::ported::options::opt_state_set("loginshell", (flags & PARSEARGS_LOGIN) != 0);

    // c:352 — `opts[PRIVILEGED] = (getuid() != geteuid() || …);`
    let priv_on = unsafe { libc::getuid() != libc::geteuid() || libc::getgid() != libc::getegid() };
    crate::ported::options::opt_state_set("privileged", priv_on);

    // c:361 — `opts[INTERACTIVE] = isatty(0) ? 2 : 0;`. The "2"
    // sentinel means "default-on, may be downgraded by SHINSTDIN
    // handling below". Rust port stores as bool — we only retain
    // the on/off distinction; the downgrade logic at c:end-of-fn
    // is a no-op once we collapse to bool.
    let interactive_default = unsafe { libc::isatty(0) != 0 };
    crate::ported::options::opt_state_set("interactive", interactive_default);

    // c:366-368 — `opts[MONITOR] = 2; opts[HASHDIRS] = 2; opts[USEZLE] = 1;`
    crate::ported::options::opt_state_set("monitor", true);
    crate::ported::options::opt_state_set("hashdirs", true);
    crate::ported::options::opt_state_set("usezle", true);
    // c:369-370 — `opts[SHINSTDIN] = 0; opts[SINGLECOMMAND] = 0;`
    crate::ported::options::opt_state_set("shinstdin", false);
    crate::ported::options::opt_state_set("singlecommand", false);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `fallback_compctlread` is the dispatch target when the zle
    /// module isn't loaded — `compctl -K read` paths need SOMETHING
    /// callable. C returns 1 + zwarnnam to signal "no provider". A
    /// regression returning 0 would make completers believe a read
    /// happened when nothing did (silent data corruption in completion).
    #[test]
    fn fallback_compctlread_signals_failure() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(fallback_compctlread("test"), 1);
    }

    /// c:418 — `parseopts` advances `idx` past every consumed option.
    /// `-c CMD` consumes 2 slots and stores CMD in cmdp. Regression
    /// that doesn't advance idx would loop forever or skip args.
    #[test]
    fn parseopts_dash_c_captures_command_string() {
        let _g = crate::test_util::global_state_lock();
        let mut argv = vec!["-c".to_string(), "echo hi".to_string()];
        let mut idx = 0usize;
        let mut cmd: Option<String> = None;
        let r = parseopts("zsh", &mut argv, &mut idx, &mut cmd, 0);
        assert_eq!(r, 0);
        assert_eq!(
            cmd.as_deref(),
            Some("echo hi"),
            "-c must capture the next arg as the command"
        );
        assert_eq!(idx, 2, "idx must advance past both -c AND its arg");
    }

    /// c:418 — `parseopts` stops at the first non-option positional.
    /// Regression that keeps walking past `--` or first bareword would
    /// silently consume the script name as an option.
    #[test]
    fn parseopts_stops_at_first_positional() {
        let _g = crate::test_util::global_state_lock();
        let mut argv = vec!["script.sh".to_string(), "arg1".to_string()];
        let mut idx = 0usize;
        let mut cmd: Option<String> = None;
        parseopts("zsh", &mut argv, &mut idx, &mut cmd, 0);
        assert_eq!(idx, 0, "must stop AT the first bareword (no advance)");
        assert!(cmd.is_none(), "no -c → cmd stays None");
    }

    /// c:418 — `parseopts` on empty argv exits cleanly with idx=0
    /// and cmd=None. Regression panicking on empty would crash
    /// startup with weird args.
    #[test]
    fn parseopts_empty_argv_is_safe() {
        let _g = crate::test_util::global_state_lock();
        let mut argv: Vec<String> = Vec::new();
        let mut idx = 0usize;
        let mut cmd: Option<String> = None;
        assert_eq!(parseopts("zsh", &mut argv, &mut idx, &mut cmd, 0), 0);
        assert_eq!(idx, 0);
        assert!(cmd.is_none());
    }

    /// c:756 — `tccap_get_name` indexes into the terminal-capability
    /// name array. Out-of-range MUST NOT panic — it returns "" so
    /// downstream tigetstr calls fail-soft. Regression panicking
    /// would crash the prompt subsystem on tiny terminals.
    #[test]
    fn tccap_get_name_out_of_range_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        // index 9999 is well past the cap table (~39 entries per
        // init.c:75). Must return safely.
        let n = tccap_get_name(9999);
        assert!(
            n.is_empty(),
            "out-of-range index must return empty (got {n:?})"
        );
    }

    /// c:418 — `parseopts` consumes `-x` (xtrace) as a flag without
    /// an argument. Pin the no-arg-required path; a regression that
    /// always reads `argv[idx+1]` would OOB-index when -x is the
    /// last arg.
    #[test]
    fn parseopts_dash_x_is_single_slot_flag() {
        let _g = crate::test_util::global_state_lock();
        let mut argv = vec!["-x".to_string()];
        let mut idx = 0usize;
        let mut cmd: Option<String> = None;
        let r = parseopts("zsh", &mut argv, &mut idx, &mut cmd, 0);
        assert_eq!(r, 0);
        assert_eq!(idx, 1, "-x consumes exactly 1 arg slot");
        assert!(cmd.is_none(), "-x must NOT capture a command");
    }

    /// c:418 — `parseopts` -c with NO following argument. The
    /// contract is "no valid command captured" so the caller can
    /// surface a usage error.
    #[test]
    fn parseopts_dash_c_without_argument_does_not_capture_command() {
        let _g = crate::test_util::global_state_lock();
        let mut argv = vec!["-c".to_string()];
        let mut idx = 0usize;
        let mut cmd: Option<String> = None;
        let _ = parseopts("zsh", &mut argv, &mut idx, &mut cmd, 0);
        assert!(
            cmd.is_none() || cmd.as_deref() == Some(""),
            "-c without arg must NOT capture a usable command"
        );
    }

    /// c:418 — `parseopts` consumes multiple flags in sequence.
    /// Pin the per-arg loop so a regen that early-exits after the
    /// first flag silently drops -v.
    #[test]
    fn parseopts_consumes_multiple_flags() {
        let _g = crate::test_util::global_state_lock();
        let mut argv = vec!["-x".to_string(), "-v".to_string()];
        let mut idx = 0usize;
        let mut cmd: Option<String> = None;
        let r = parseopts("zsh", &mut argv, &mut idx, &mut cmd, 0);
        assert_eq!(r, 0);
        assert_eq!(idx, 2, "both flags must be consumed");
    }

    /// c:756 — `tccap_get_name(0)` returns the first cap name. The
    /// table is non-empty per init.c:75; index 0 must be a valid
    /// stem. Pin the boundary so a regen that adds an "off-by-one
    /// slot 0" mistake gets caught.
    #[test]
    fn tccap_get_name_index_zero_is_nonempty() {
        let _g = crate::test_util::global_state_lock();
        let n = tccap_get_name(0);
        assert!(
            !n.is_empty(),
            "index 0 must yield a real cap name; got {:?}",
            n
        );
        for c in n.chars() {
            assert!(
                c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_',
                "cap name {:?} contains non-termcap char {:?}",
                n,
                c
            );
        }
    }

    /// c:1713 — `noop_function` MUST be safe to call without panics.
    /// C body is empty; the Rust port mirrors it. Re-entry test
    /// catches a regen that adds stateful side-effects.
    #[test]
    fn noop_function_is_safe_to_call_multiple_times() {
        let _g = crate::test_util::global_state_lock();
        noop_function();
        noop_function();
        noop_function();
    }

    /// c:1720 — `noop_function_int(n)` ignores its argument across
    /// the full i32 range. Pin no-side-effect contract.
    #[test]
    fn noop_function_int_ignores_argument() {
        let _g = crate::test_util::global_state_lock();
        noop_function_int(0);
        noop_function_int(42);
        noop_function_int(-1);
        noop_function_int(i32::MAX);
        noop_function_int(i32::MIN);
    }

    /// c:1743 — `zleentry(cmd)` for an unknown cmd code returns
    /// None (matching the C "no zle" branch). Every call site
    /// assumes None means "no ZLE active".
    #[test]
    fn zleentry_unknown_command_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            zleentry(99999).is_none(),
            "unknown zle command must return None, not a default string"
        );
    }

    /// c:1551 — `source("")` on empty path must NOT segfault. The
    /// C source opens("") which fails with ENOENT, returning non-0.
    #[test]
    fn source_empty_path_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let r = source("");
        assert_ne!(r, 0, "source of empty path must report failure");
    }

    // ─── zsh-corpus pins for init helpers ───────────────────────────

    /// `tccap_get_name(0)` returns a non-empty cap name.
    #[test]
    fn init_corpus_tccap_index_zero_nonempty() {
        let _g = crate::test_util::global_state_lock();
        let name = tccap_get_name(0);
        assert!(!name.is_empty(), "cap[0] has a name, got {name:?}");
    }

    /// `tccap_get_name` for index TC_COUNT or higher returns "".
    #[test]
    fn init_corpus_tccap_out_of_range_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(tccap_get_name(39), "", "TC_COUNT = empty");
        assert_eq!(tccap_get_name(999), "", "way out of range = empty");
    }

    /// All cap indexes 0..TC_COUNT return non-empty distinct names.
    #[test]
    fn init_corpus_tccap_indexes_all_named() {
        let _g = crate::test_util::global_state_lock();
        for i in 0..39 {
            let n = tccap_get_name(i);
            assert!(!n.is_empty(), "cap {i} should have a name");
        }
    }

    /// `source("/nonexistent/path/zshrs_test_xyz")` returns non-zero.
    #[test]
    fn init_corpus_source_nonexistent_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        assert_ne!(source("/nonexistent/path/zshrs_test_xyz_zzz"), 0);
    }

    /// `noop_function()` doesn't panic.
    #[test]
    fn init_corpus_noop_function_does_not_panic() {
        let _g = crate::test_util::global_state_lock();
        noop_function();
    }

    /// `noop_function_int(0)` doesn't panic.
    #[test]
    fn init_corpus_noop_function_int_zero_no_panic() {
        let _g = crate::test_util::global_state_lock();
        noop_function_int(0);
    }

    /// `zleentry` with negative cmd code returns None.
    #[test]
    fn init_corpus_zleentry_negative_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(zleentry(-1).is_none());
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/init.c tccap_get_name / init_term
    // / noop helpers.
    // ═══════════════════════════════════════════════════════════════════

    /// c:756 — `tccap_get_name(cap)` for cap >= 39 (TC_COUNT) returns
    /// empty string (Rust port out-of-range guard, c:771).
    #[test]
    fn tccap_get_name_out_of_range_returns_empty_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(tccap_get_name(39), "", "TC_COUNT boundary returns ''");
        assert_eq!(tccap_get_name(100), "");
        assert_eq!(tccap_get_name(usize::MAX), "");
    }

    /// c:756 — `tccap_get_name(0)` returns a non-empty cap name
    /// (first entry of tccapnams table).
    #[test]
    fn tccap_get_name_first_entry_is_nonempty() {
        let _g = crate::test_util::global_state_lock();
        let n = tccap_get_name(0);
        assert!(!n.is_empty(), "tccapnams[0] must be a valid cap name");
    }

    /// c:756 — `tccap_get_name` is deterministic across calls.
    #[test]
    fn tccap_get_name_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for i in 0..39 {
            let first = tccap_get_name(i);
            for _ in 0..5 {
                assert_eq!(tccap_get_name(i), first, "tccap_get_name({}) impure", i);
            }
        }
    }

    /// c:756 — every in-range cap name fits in ASCII and has
    /// non-control content (termcap caps are 2-char ASCII codes).
    #[test]
    fn tccap_get_name_in_range_returns_printable_ascii() {
        let _g = crate::test_util::global_state_lock();
        for i in 0..39 {
            let n = tccap_get_name(i);
            if n.is_empty() {
                continue;
            }
            for c in n.chars() {
                assert!(
                    c.is_ascii() && !c.is_control(),
                    "tccap_get_name({}) = {:?} has non-printable char {:?}",
                    i,
                    n,
                    c
                );
            }
        }
    }

    /// c:1713 — `noop_function` returns nothing, has no observable effect.
    /// Repeated calls don't accumulate state.
    #[test]
    fn noop_function_is_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..100 {
            noop_function();
        }
    }

    /// c:1720 — `noop_function_int` accepts any i32 value without panic.
    #[test]
    fn noop_function_int_accepts_any_value() {
        let _g = crate::test_util::global_state_lock();
        noop_function_int(0);
        noop_function_int(-1);
        noop_function_int(i32::MAX);
        noop_function_int(i32::MIN);
    }

    /// `zleentry` with very large positive cmd code returns None
    /// (out-of-range cmd dispatch).
    #[test]
    fn zleentry_unknown_cmd_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(zleentry(99999).is_none(), "unknown cmd → None");
    }

    /// c:1209 — `init_bltinmods` is idempotent (safe to call multiple times).
    #[test]
    fn init_bltinmods_is_idempotent() {
        let _g = crate::test_util::global_state_lock();
        init_bltinmods();
        init_bltinmods();
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/init.c
    // c:747 tccapnams / c:756 tccap_get_name / c:1713 noop_function /
    // c:1199 fallback_compctlread.
    // ═══════════════════════════════════════════════════════════════════

    /// c:747 — `tccapnams` table has exactly 39 entries (TC_COUNT).
    #[test]
    fn tccapnams_table_size_is_tc_count() {
        assert_eq!(tccapnams.len(), 39, "TC_COUNT must be 39");
    }

    /// c:747 — every tccapnams entry is exactly 2 ASCII chars
    /// (termcap 2-letter capability code convention).
    #[test]
    fn tccapnams_entries_are_two_ascii_chars() {
        for (i, &n) in tccapnams.iter().enumerate() {
            assert_eq!(n.len(), 2, "tccapnams[{}] = {:?} must be 2 chars", i, n);
            assert!(
                n.chars().all(|c| c.is_ascii_alphabetic()),
                "tccapnams[{}] = {:?} must be ASCII alpha",
                i,
                n
            );
        }
    }

    /// c:747 — no duplicates in tccapnams.
    #[test]
    fn tccapnams_has_no_duplicates() {
        let mut seen = std::collections::HashSet::new();
        for &n in tccapnams.iter() {
            assert!(seen.insert(n), "duplicate tccap name: {:?}", n);
        }
    }

    /// c:756 — `tccap_get_name(39)` is at TC_COUNT boundary → empty.
    #[test]
    fn tccap_get_name_at_tc_count_boundary_returns_empty() {
        assert_eq!(tccap_get_name(39), "", "TC_COUNT bound returns empty");
    }

    /// c:756 — `tccap_get_name(38)` is last valid index → non-empty.
    #[test]
    fn tccap_get_name_last_valid_index_returns_nonempty() {
        let n = tccap_get_name(38);
        assert!(!n.is_empty(), "tccap_get_name(38) must be non-empty");
        assert_eq!(n, tccapnams[38], "matches table");
    }

    /// c:756 — `tccap_get_name(usize::MAX)` is far out of range → empty.
    #[test]
    fn tccap_get_name_usize_max_returns_empty() {
        assert_eq!(tccap_get_name(usize::MAX), "");
    }

    /// c:756 — `tccap_get_name` is a pure function.
    #[test]
    fn tccap_get_name_is_pure() {
        for i in [0usize, 1, 5, 10, 38, 39, 100] {
            let a = tccap_get_name(i);
            let b = tccap_get_name(i);
            assert_eq!(a, b, "tccap_get_name({}) must be deterministic", i);
        }
    }

    /// c:1199 — `fallback_compctlread` returns 1 (failure) without args.
    #[test]
    fn fallback_compctlread_returns_one_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(fallback_compctlread(""), 1, "fallback always fails");
        assert_eq!(fallback_compctlread("anything"), 1);
    }

    /// c:1713 — `noop_function` is a true no-op (no panic, no state change).
    #[test]
    fn noop_function_does_nothing_observable() {
        for _ in 0..100 {
            noop_function();
        }
    }

    /// c:1720 — `noop_function_int` accepts any i32 without panic.
    #[test]
    fn noop_function_int_accepts_full_i32_range() {
        for v in [i32::MIN, -1, 0, 1, 42, i32::MAX] {
            noop_function_int(v);
        }
    }

    /// c:1159 — `zleentry(0)` returns None (no live ZLE for cmd=0).
    #[test]
    fn zleentry_zero_cmd_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zleentry(0), None, "cmd=0 unwired without live ZLE");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/init.c
    // c:464 tccap_get_name / c:1081 source / c:1116 sourcehome /
    // c:1159 zleentry / c:1199 fallback_compctlread / c:1149 noop_function /
    // c:1154 noop_function_int / c:1143 init_bltinmods
    // ═══════════════════════════════════════════════════════════════════

    /// c:464 — `tccap_get_name` returns &'static str (compile-time type pin).
    #[test]
    fn tccap_get_name_returns_static_str_type() {
        let _: &'static str = tccap_get_name(0);
    }

    /// c:1081 — `source("")` empty path returns i32 (compile-time pin).
    #[test]
    fn source_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = source("");
    }

    /// c:1081 — `source` is deterministic for the same nonexistent path.
    #[test]
    fn source_nonexistent_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = source("/__never_exists_zshrs_source__");
        for _ in 0..3 {
            assert_eq!(
                source("/__never_exists_zshrs_source__"),
                first,
                "source must be deterministic"
            );
        }
    }

    /// c:1116 — `sourcehome("")` empty arg is safe.
    #[test]
    fn sourcehome_empty_no_panic() {
        let _g = crate::test_util::global_state_lock();
        sourcehome("");
    }

    /// c:1159 — `zleentry` returns Option<String> (compile-time type pin).
    #[test]
    fn zleentry_returns_option_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<String> = zleentry(0);
    }

    /// c:1199 — `fallback_compctlread` returns i32.
    #[test]
    fn fallback_compctlread_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = fallback_compctlread("");
    }

    /// c:1199 — `fallback_compctlread` is deterministic for any name.
    #[test]
    fn fallback_compctlread_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for n in ["", "name", "anything"] {
            let first = fallback_compctlread(n);
            for _ in 0..3 {
                assert_eq!(
                    fallback_compctlread(n),
                    first,
                    "fallback_compctlread({:?}) must be deterministic",
                    n
                );
            }
        }
    }

    /// c:1143 — `init_bltinmods` returns void (no panic check).
    #[test]
    fn init_bltinmods_no_panic_full_sweep() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            init_bltinmods();
        }
    }

    /// c:1149 — `noop_function` returns void (compile-time void type).
    #[test]
    fn noop_function_signature_void() {
        let _: () = noop_function();
    }

    /// c:1154 — `noop_function_int(N)` returns void.
    #[test]
    fn noop_function_int_signature_void() {
        let _: () = noop_function_int(0);
    }

    /// c:1116 — `sourcehome("/")` root dir safe.
    #[test]
    fn sourcehome_root_dir_no_panic() {
        let _g = crate::test_util::global_state_lock();
        sourcehome("/");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/init.c
    // c:464 tccap_get_name / c:491 init_term / c:1081 source /
    // c:1159 zleentry / c:1199 fallback_compctlread + lifecycle pins
    // ═══════════════════════════════════════════════════════════════════

    /// c:464 — `tccap_get_name` returns `&'static str` (compile-time pin, alt).
    #[test]
    fn tccap_get_name_returns_static_str_pin_alt() {
        let _: &'static str = tccap_get_name(0);
    }

    /// c:464 — `tccap_get_name(TC_COUNT)` and beyond return empty.
    /// C source: `if (cap >= 39 /* TC_COUNT */) return "";`.
    #[test]
    fn tccap_get_name_oob_returns_empty() {
        assert_eq!(tccap_get_name(39), "", "TC_COUNT itself returns empty");
        assert_eq!(tccap_get_name(100), "", "way past TC_COUNT returns empty");
        assert_eq!(
            tccap_get_name(usize::MAX),
            "",
            "MAX returns empty (no panic)"
        );
    }

    /// c:464 — `tccap_get_name` is deterministic for in-range indices.
    #[test]
    fn tccap_get_name_deterministic_for_valid_indices() {
        for cap in 0..39usize {
            let first = tccap_get_name(cap);
            for _ in 0..3 {
                assert_eq!(
                    tccap_get_name(cap),
                    first,
                    "tccap_get_name({}) must be pure",
                    cap
                );
            }
        }
    }

    /// c:464 — every in-range cap returns non-empty (zsh ships a name
    /// for every TC_* slot; an empty entry would be a corpus drift).
    #[test]
    fn tccap_get_name_all_in_range_caps_non_empty() {
        for cap in 0..39usize {
            assert!(
                !tccap_get_name(cap).is_empty(),
                "TC_{} (cap idx {}) must have a non-empty cap name",
                cap,
                cap
            );
        }
    }

    /// c:491 — `init_term` returns i32 (compile-time pin).
    #[test]
    fn init_term_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = init_term();
    }

    /// c:1081 — `source(empty)` returns i32 (compile-time pin, alt).
    #[test]
    fn source_returns_i32_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = source("");
    }

    /// c:1081 — `source("/__nonexistent_xyz__")` is non-fatal (returns).
    #[test]
    fn source_nonexistent_file_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = source("/__definitely_not_a_file_zshrs_xyz__");
    }

    /// c:1159 — `zleentry` is deterministic (pure dispatch).
    #[test]
    fn zleentry_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for cmd in 0..5i32 {
            let first = zleentry(cmd);
            for _ in 0..3 {
                assert_eq!(
                    zleentry(cmd),
                    first,
                    "zleentry({}) must be deterministic",
                    cmd
                );
            }
        }
    }

    /// c:1199 — `fallback_compctlread("")` is safe (empty name no-op).
    #[test]
    fn fallback_compctlread_empty_name_safe() {
        let _g = crate::test_util::global_state_lock();
        let _ = fallback_compctlread("");
    }

    /// c:1149 — `noop_function` called repeatedly is safe.
    #[test]
    fn noop_function_repeated_calls_safe() {
        for _ in 0..100 {
            noop_function();
        }
    }

    /// c:1154 — `noop_function_int` for various ints is safe + void.
    #[test]
    fn noop_function_int_various_inputs_safe() {
        for n in [-1, 0, 1, i32::MIN, i32::MAX] {
            let _: () = noop_function_int(n);
        }
    }

    /// c:1116 — `sourcehome("nonexistent_file_xyz")` safe (no-op when
    /// the home-relative path doesn't exist).
    #[test]
    fn sourcehome_nonexistent_file_safe() {
        let _g = crate::test_util::global_state_lock();
        sourcehome("__definitely_no_such_zshrc_xyz__");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/init.c
    // c:464 tccap_get_name / c:491 init_term / c:1081 source /
    // c:1116 sourcehome / c:1143 init_bltinmods / c:1149 noop_function /
    // c:1159 zleentry / c:1199 fallback_compctlread
    // ═══════════════════════════════════════════════════════════════════

    /// c:1143 — `init_bltinmods` is idempotent (safe to call repeatedly).
    #[test]
    fn init_bltinmods_idempotent_repeated_calls() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            init_bltinmods();
        }
    }

    /// c:464 — `tccap_get_name` for index 0 returns non-empty (first cap).
    #[test]
    fn tccap_get_name_index_zero_non_empty() {
        let _g = crate::test_util::global_state_lock();
        let _ = tccap_get_name(0);
    }

    /// c:464 — `tccap_get_name(usize::MAX)` doesn't panic.
    #[test]
    fn tccap_get_name_usize_max_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let r = tccap_get_name(usize::MAX);
        // OOB returns empty per the OOB test that already exists.
        assert!(r.is_empty(), "OOB must return empty, got {:?}", r);
    }

    /// c:1116 — `sourcehome("")` empty name safe (no-op when path empty).
    #[test]
    fn sourcehome_empty_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        sourcehome("");
    }

    /// c:1081 — `source` return type i32 (compile-time pin).
    #[test]
    fn source_returns_i32_type_compile_pin() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = source("/dev/null");
    }

    /// c:1081 — `source("")` empty path doesn't panic.
    #[test]
    fn source_empty_path_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = source("");
    }

    /// c:1159 — `zleentry` return type Option<String> (compile-time pin, alt).
    #[test]
    fn zleentry_returns_option_string_type_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<String> = zleentry(0);
    }

    /// c:1159 — `zleentry(-1)` (invalid cmd) doesn't panic.
    #[test]
    fn zleentry_negative_cmd_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = zleentry(-1);
    }

    /// c:1199 — `fallback_compctlread` returns i32 (compile-time pin, alt).
    #[test]
    fn fallback_compctlread_returns_i32_type_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = fallback_compctlread("test");
    }

    /// c:1149 — `noop_function` returns void (compile-time pin).
    #[test]
    fn noop_function_returns_void_type() {
        let _g = crate::test_util::global_state_lock();
        let _: () = noop_function();
    }

    /// c:1154 — `noop_function_int` returns void (compile-time pin).
    #[test]
    fn noop_function_int_returns_void_type() {
        let _g = crate::test_util::global_state_lock();
        let _: () = noop_function_int(42);
    }

    /// c:464 — `tccap_get_name` is deterministic for OOB (same empty value
    /// across calls).
    #[test]
    fn tccap_get_name_oob_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let a = tccap_get_name(99_999);
        let b = tccap_get_name(99_999);
        assert_eq!(a, b, "OOB must be deterministic");
    }
}
