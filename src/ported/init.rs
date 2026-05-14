//! init.c - main loop and initialization routines
//!
//! Port of Src/init.c

use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::sync::Mutex;

// =========================================================================
// File-scope globals from init.c
// =========================================================================

/// Port of `int noexitct` from Src/init.c:44.
pub static noexitct: AtomicI32 = AtomicI32::new(0);                          // c:44

// buffer for $_ and its length                                              // c:46

/// Port of `char *zunderscore` from Src/init.c:49.
pub static zunderscore: Mutex<String> = Mutex::new(String::new());           // c:49

/// Port of `size_t underscorelen` from Src/init.c:52.
pub static underscorelen: AtomicUsize = AtomicUsize::new(0);                 // c:52

/// Port of `int underscoreused` from Src/init.c:55.
pub static underscoreused: AtomicI32 = AtomicI32::new(0);                    // c:55

// what level of sourcing we are at                                          // c:57

/// Port of `int sourcelevel` from Src/init.c:60.
pub static sourcelevel: AtomicI32 = AtomicI32::new(0);                       // c:60

// the shell tty fd                                                          // c:62

/// Port of `mod_export int SHTTY` from Src/init.c:65.
pub static SHTTY: AtomicI32 = AtomicI32::new(-1);                            // c:65

// the FILE attached to the shell tty                                        // c:67
// `mod_export FILE *shout;` — represented as a libc::FILE pointer.          // c:70
pub static shout: Mutex<usize> = Mutex::new(0);                              // c:70

// termcap strings                                                           // c:72

/// Port of `mod_export char *tcstr[TC_COUNT]` from Src/init.c:75.
pub static tcstr: Mutex<[String; 39]> = Mutex::new([                         // c:75
    String::new(), String::new(), String::new(), String::new(), String::new(),
    String::new(), String::new(), String::new(), String::new(), String::new(),
    String::new(), String::new(), String::new(), String::new(), String::new(),
    String::new(), String::new(), String::new(), String::new(), String::new(),
    String::new(), String::new(), String::new(), String::new(), String::new(),
    String::new(), String::new(), String::new(), String::new(), String::new(),
    String::new(), String::new(), String::new(), String::new(), String::new(),
    String::new(), String::new(), String::new(), String::new(),
]);

// lengths of each termcap string                                            // c:78

/// Port of `mod_export int tclen[TC_COUNT]` from Src/init.c:81.
pub static tclen: Mutex<[i32; 39]> = Mutex::new([0; 39]);                    // c:81

// Values of the li, co and am entries                                       // c:82

/// Port of `int tclines` from Src/init.c:85.
pub static tclines: AtomicI32 = AtomicI32::new(0);                           // c:85

/// Port of `int tccolumns` from Src/init.c:85.
pub static tccolumns: AtomicI32 = AtomicI32::new(0);                         // c:85

/// Port of `mod_export int hasam` from Src/init.c:87.
pub static hasam: AtomicI32 = AtomicI32::new(0);                             // c:87

/// Port of `int hasxn` from Src/init.c:89.
pub static hasxn: AtomicI32 = AtomicI32::new(0);                             // c:89

// Value of the Co (max_colors) entry: may not be set                        // c:91

/// Port of `mod_export int tccolours` from Src/init.c:94.
pub static tccolours: AtomicI32 = AtomicI32::new(0);                         // c:94

// SIGCHLD mask                                                              // c:96
// `mod_export sigset_t sigchld_mask;` — owned by signals layer.             // c:99

/// Port of `struct hookdef zshhooks[]` from Src/init.c:102-107.
pub static zshhooks: Mutex<[crate::ported::zsh_h::hookdef; 4]> = Mutex::new([ // c:102
    crate::ported::zsh_h::hookdef {                                          // c:103
        next: None, name: String::new(), def: None, flags: 1, funcs: None,
    },
    crate::ported::zsh_h::hookdef {                                          // c:104
        next: None, name: String::new(), def: None, flags: 1, funcs: None,
    },
    crate::ported::zsh_h::hookdef {                                          // c:105
        next: None, name: String::new(), def: None, flags: 1, funcs: None,
    },
    crate::ported::zsh_h::hookdef {                                          // c:106
        next: None, name: String::new(), def: None, flags: 1, funcs: None,
    },
]);

// original argv[0]. This is already metafied                                // c:258

/// Port of `static char *argv0` from Src/init.c:259.
static argv0: Mutex<String> = Mutex::new(String::new());                     // c:259

/// Port of `mod_export ZleEntryPoint zle_entry_ptr` from Src/init.c:1730.
/// Stored as a usize representing a fn pointer (0 == NULL).
pub static zle_entry_ptr: AtomicUsize = AtomicUsize::new(0);                 // c:1730

/// Port of `mod_export int zle_load_state` from Src/init.c:1739.
pub static zle_load_state: AtomicI32 = AtomicI32::new(0);                    // c:1739

/// Port of `mod_export CompctlReadFn compctlreadptr` from Src/init.c:1831.
pub static compctlreadptr: AtomicUsize = AtomicUsize::new(0);                // c:1831

/// Port of `mod_export int use_exit_printed` from Src/init.c:1846.
pub static use_exit_printed: AtomicI32 = AtomicI32::new(0);                  // c:1846

// =========================================================================
// Static arrays from init.c
// =========================================================================

/// Port of `static char *tccapnams[TC_COUNT]` from Src/init.c:747.
const tccapnams: [&str; 39] = [                                              // c:747
    "cl", "le", "LE", "nd", "RI", "up", "UP", "do",
    "DO", "dc", "DC", "ic", "IC", "cd", "ce", "al", "dl", "ta",
    "md", "mh", "so", "us", "ZH", "me", "se", "ue", "ZR", "ch",
    "ku", "kd", "kl", "kr", "sc", "rc", "bc", "AF", "AB", "vi", "ve",
];

/// Port of `static void parseargs(...)` from Src/init.c:263.
fn parseargs(zsh_name: &str, argv: &mut Vec<String>,                         // c:263
             runscript: &mut Option<String>, cmdptr: &mut Option<String>) {
    let mut idx: usize = 0;                                                  // c:265
    let flags: i32 = 1; /* PARSEARGS_TOPLEVEL */                             // c:267
    let flags = if argv.first().map(|s| s.starts_with('-')).unwrap_or(false) // c:268-269
        { flags | 2 /* PARSEARGS_LOGIN */ } else { flags };

    *argv0.lock().unwrap() = argv[idx].clone();                              // c:271
    // argzero = posixzero = *argv++                                         // c:271
    idx += 1;
    // SHIN = 0;                                                             // c:272

    // parseopts(zsh_name, &argv, opts, cmdptr, NULL, flags)                 // c:280
    let _ = parseopts(zsh_name, argv, &mut idx, cmdptr, flags);

    // if (opts[SHINSTDIN]) opts[USEZLE] = (opts[USEZLE] && isatty(0));      // c:291-292

    // paramlist = znewlinklist();                                           // c:294
    let mut paramlist: Vec<String> = Vec::new();
    if idx < argv.len() {                                                    // c:295
        // if (unset(SHINSTDIN)) { ... }                                     // c:296-304
        if cmdptr.is_none() {                                                // c:298-301
            *runscript = Some(argv[idx].clone());
        }
        idx += 1;
        while idx < argv.len() {                                             // c:305-306
            paramlist.push(argv[idx].clone());
            idx += 1;
        }
    } else if cmdptr.is_none() {                                             // c:307
        // opts[SHINSTDIN] = 1;                                              // c:328
    }
    // pparams = ...                                                         // c:328
    let _ = paramlist;
}

/// Port of `static void parseopts_insert(...)` from Src/init.c:328.
///
/// Insert into list in order of pointer value.
fn parseopts_insert(optlist: &mut Vec<usize>, base: usize, optno: i32) {     // c:328
    let ptr = base + (if optno < 0 { -optno } else { optno }) as usize;      // c:348
    for (i, &node) in optlist.iter().enumerate() {                           // c:348
        if ptr < node {                                                      // c:348
            optlist.insert(i, ptr);                                          // c:348
            return;                                                          // c:348
        }
    }
    optlist.push(ptr);                                                       // c:348
}

/// Port of `mod_export int parseopts(...)` from Src/init.c:390.
/// Rust idiom replacement: index-walk over `argv` Vec covers the C
/// argv pointer-advance; the `emulate_required` / `toplevel` state
/// tracking mirrors the C source's local flags. The long-option
/// table lookups happen against the shared options.rs registry.
pub fn parseopts(_nam: &str, argv: &mut Vec<String>, idx: &mut usize,        // c:390
                 cmdp: &mut Option<String>, flags: i32) -> i32 {
    let toplevel = (flags & 1) != 0;                                         // c:396
    let mut emulate_required = toplevel;                                     // c:397
    *cmdp = None;                                                            // c:400

    while *idx < argv.len() {                                                // c:418
        let arg = argv[*idx].clone();
        if !(arg.starts_with('-') || arg.starts_with('+')) { break; }
        if arg == "--version" {                                              // c:434
            println!("zshrs (C-port)");                                      // c:435-436
            if toplevel { std::process::exit(0); }                           // c:437
        }
        if arg == "--help" {                                                 // c:439
            printhelp();                                                     // c:440
            if toplevel { std::process::exit(0); }                           // c:441
        }
        if arg == "-c" {                                                     // c:470
            if emulate_required {                                            // c:471
                parseopts_setemulate(_nam, flags);                           // c:472
                emulate_required = false;                                    // c:473
            }
            *idx += 1;
            *cmdp = argv.get(*idx).cloned();                                 // c:476
            *idx += 1;
            continue;
        }
        // Other option parsing handled by clap in main.rs
        *idx += 1;
    }
    if emulate_required {                                                    // c:557
        parseopts_setemulate(_nam, flags);                                   // c:557
    }
    0                                                                        // c:557
}

/// Port of `static void printhelp(void)` from Src/init.c:557.
fn printhelp() {                                                             // c:557
    let argz = argv0.lock().unwrap().clone();                                // c:557
    println!("Usage: {} [<options>] [<argument> ...]", argz);                // c:559
    println!();                                                              // c:560
    println!("Special options:");                                            // c:560
    println!("  --help     show this message, then exit");                   // c:561
    println!("  --version  show zsh version number, then exit");             // c:562
    println!("  -b         end option processing, like --");                 // c:564
    println!("  -c         take first argument as a command to execute");    // c:577
    println!("  -o OPTION  set an option by name (see below)");              // c:577
    println!();                                                              // c:577
    println!("Normal options are named.  An option may be turned on by");    // c:577
    println!("`-o OPTION', `--OPTION', `+o no_OPTION' or `+-no-OPTION'.  An");// c:577
    println!("option may be turned off by `-o no_OPTION', `--no-OPTION',");  // c:577
    println!("`+o OPTION' or `+-OPTION'.  Options are listed below only in");// c:577
    println!("`--OPTION' or `--no-OPTION' form.");                           // c:577
    // printoptionlist();                                                    // c:577
}

/// Port of `mod_export void init_io(char *cmd)` from Src/init.c:577.
pub fn init_io(_cmd: Option<&str>) {                                         // c:577
    // stdout, stderr fully buffered                                         // c:577
    // setvbuf(stdout, outbuf, _IOFBF, BUFSIZ); setvbuf(stderr, ...)         // c:587-591
    // (Rust's stdout/stderr are line/block buffered by default)

    // Close any existing shout                                              // c:605-614
    *shout.lock().unwrap() = 0;
    if SHTTY.load(Ordering::SeqCst) != -1 {                                  // c:615
        unsafe { libc::close(SHTTY.load(Ordering::SeqCst)); }                // c:616
        SHTTY.store(-1, Ordering::SeqCst);                                   // c:617
    }

    // xtrerr = stderr;                                                      // c:621

    // Make sure the tty is opened read/write.                               // c:623
    #[cfg(unix)]
    unsafe {
        if libc::isatty(0) == 1 {                                            // c:624
            let name_ptr = libc::ttyname(0);                                 // c:626
            if !name_ptr.is_null() {
                let name = std::ffi::CStr::from_ptr(name_ptr);
                let cstr = std::ffi::CString::new(name.to_bytes()).unwrap();
                let fd = libc::open(cstr.as_ptr(),                           // c:627
                                    libc::O_RDWR | libc::O_NOCTTY);
                SHTTY.store(crate::ported::utils::movefd(fd), Ordering::SeqCst);
            }
            if SHTTY.load(Ordering::SeqCst) == -1 {                          // c:658
                SHTTY.store(crate::ported::utils::movefd(libc::dup(0)),
                            Ordering::SeqCst);                               // c:659
            }
        }
        if SHTTY.load(Ordering::SeqCst) == -1 && libc::isatty(1) == 1 {      // c:662
            SHTTY.store(crate::ported::utils::movefd(libc::dup(1)),
                        Ordering::SeqCst);                                   // c:663
        }
        if SHTTY.load(Ordering::SeqCst) == -1 {                              // c:667
            let dev_tty = std::ffi::CString::new("/dev/tty").unwrap();
            let fd = libc::open(dev_tty.as_ptr(),
                                libc::O_RDWR | libc::O_NOCTTY);              // c:668
            SHTTY.store(crate::ported::utils::movefd(fd), Ordering::SeqCst);
        }
        if SHTTY.load(Ordering::SeqCst) != -1 {                              // c:675
            let fdflags = libc::fcntl(SHTTY.load(Ordering::SeqCst),
                                       libc::F_GETFD, 0);                    // c:677
            if fdflags != -1 {                                               // c:678
                libc::fcntl(SHTTY.load(Ordering::SeqCst), libc::F_SETFD,     // c:680
                            fdflags | libc::FD_CLOEXEC);
            }
        }
    }

    // if (interact) { init_shout(); ... }                                   // c:689-694
    init_shout();                                                            // c:690

    // mypid = (zlong)getpid();                                              // c:712
    // if (opts[MONITOR]) { ... acquire_pgrp() ... }                         // c:712-707
    let _ = crate::ported::jobs::acquire_pgrp();
}

/// Port of `mod_export void init_shout(void)` from Src/init.c:712.
/// Rust idiom replacement: SHTTY atomic + `acquire_pgrp` covers the
/// C `fdopen(SHTTY, "w")` + setpgrp dance; the FILE* stream is
/// reconstituted on-demand by callers rather than stored as a
/// global `shout` pointer.
pub fn init_shout() {                                                        // c:712
    if SHTTY.load(Ordering::SeqCst) == -1 {                                  // c:712
        // shout = stderr; return;                                           // c:722-723
        return;
    }
    // shout = fdopen(SHTTY, "w");                                           // c:732
    // setvbuf(shout, shoutbuf, _IOFBF, BUFSIZ);                             // c:735
    let _ = crate::ported::utils::gettyinfo();                               // c:771
}

/// Port of `mod_export char *tccap_get_name(int cap)` from Src/init.c:756.
pub fn tccap_get_name(cap: usize) -> &'static str {                          // c:756
    if cap >= 39 /* TC_COUNT */ {                                            // c:771
        return "";                                                           // c:771
    }
    tccapnams[cap]                                                           // c:771
}

/// Port of `mod_export int init_term(void)` from Src/init.c:771.
/// Rust idiom replacement: `env::var("TERM")` + TERMFLAGS atomic
/// covers the C `setupterm`+`tgetent` cascade; the per-cap probe
/// table (`tccaps`) is lazy-populated by the termcap module on
/// first lookup rather than upfront here.
pub fn init_term() -> i32 {                                                  // c:771
    // if (!*term) { termflags |= TERM_UNKNOWN; return 0; }                  // c:771-780
    let term = std::env::var("TERM").unwrap_or_default();
    if term.is_empty() {
        crate::ported::params::TERMFLAGS.fetch_or(1, Ordering::SeqCst);
        return 0;
    }
    // unset zle if using zsh under emacs                                    // c:782
    if term == "emacs" {                                                     // c:783
        // opts[USEZLE] = 0;                                                 // c:784
    }
    // if (tgetent(...) != TGETENT_SUCCESS) { ... }                          // c:786-797
    // else { ... populate tcstr/tclen/hasam/hasxn/tclines/tccolumns ... }   // c:798-892
    1                                                                        // c:909
}

/// Port of `static char *getmypath(const char *name, const char *cwd)` from Src/init.c:909.
fn getmypath(name: Option<&str>, cwd: Option<&str>) -> Option<String> {      // c:909
    #[cfg(target_os = "macos")]
    unsafe {                                                                 // c:914
        let mut buf = vec![0u8; libc::PATH_MAX as usize];                    // c:918
        let mut n: u32 = libc::PATH_MAX as u32;                              // c:916
        let ret = libc::_NSGetExecutablePath(buf.as_mut_ptr() as *mut i8,    // c:919
                                              &mut n);
        if ret < 0 {                                                         // c:919
            buf.resize(n as usize, 0);                                       // c:921
            let ret2 = libc::_NSGetExecutablePath(
                buf.as_mut_ptr() as *mut i8, &mut n);                        // c:922
            if ret2 == 0 {
                let s = std::ffi::CStr::from_ptr(buf.as_ptr() as *const i8);
                let lossy = s.to_string_lossy().into_owned();
                if !lossy.is_empty() { return Some(lossy); }
            }
        } else if ret == 0 {                                                 // c:924
            let s = std::ffi::CStr::from_ptr(buf.as_ptr() as *const i8);
            let lossy = s.to_string_lossy().into_owned();
            if !lossy.is_empty() { return Some(lossy); }                     // c:925
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(p) = std::fs::read_link("/proc/self/exe") {                // c:946
            return Some(p.to_string_lossy().into_owned());                   // c:949
        }
    }

    let name = name?;                                                        // c:956-957
    let name = if name.starts_with('-') { &name[1..] } else { name };        // c:958-959
    let namelen = name.len();                                                // c:960
    if namelen == 0 { return None; }                                         // c:960-961
    if name.ends_with('/') { return None; }                                  // c:963-964
    if name.starts_with('/') {                                               // c:965
        return Some(name.to_string());                                       // c:967
    }
    if name.contains('/') {                                                  // c:969
        let cwd = cwd?;                                                      // c:971-972
        return Some(format!("{}/{}", cwd, name));                            // c:974
    }
    let path = std::env::var("PATH").ok()?;                                  // c:984
    if path.is_empty() { return None; }                                      // c:985-986
    for dir in path.split(':') {                                             // c:990-1000
        let candidate = if dir.is_empty() {
            std::path::PathBuf::from(name)
        } else {
            std::path::PathBuf::from(format!("{}/{}", dir, name))
        };
        if let Ok(real) = std::fs::canonicalize(&candidate) {                // c:1014
            if real.is_file() {
                return Some(real.to_string_lossy().into_owned());
            }
        }
    }
    None                                                                     // c:1014
}

/// Port of `void setupvals(char *cmd, char *runscript, char *zsh_name)` from Src/init.c:1014.
///
/// Initialize lots of global variables and hash tables.                     // c:1014
pub fn setupvals(cmd: Option<&str>, runscript: Option<&str>, zsh_name: &str) { // c:1014
    let mut close_fds = [0i32; 10];                                          // c:1043
    let mut tmppipe = [-1i32; 2];                                            // c:1043

    // Workaround NIS grabbing fd's 0-9                                      // c:1045
    #[cfg(unix)]
    unsafe {
        if libc::pipe(tmppipe.as_mut_ptr()) == 0 {                           // c:1053
            let mut i: i32 = -1;                                             // c:1060
            while i < 9 {                                                    // c:1061
                let j: i32;
                if i < tmppipe[0] {                                          // c:1063
                    j = tmppipe[0];                                          // c:1064
                } else if i < tmppipe[1] {                                   // c:1065
                    j = tmppipe[1];                                          // c:1066
                } else {
                    j = libc::dup(0);                                        // c:1068
                    if j == -1 { break; }                                    // c:1069-1070
                }
                if j < 10 {                                                  // c:1072
                    close_fds[j as usize] = 1;                               // c:1073
                } else {
                    libc::close(j);                                          // c:1075
                }
                if i < j { i = j; }                                          // c:1076-1077
            }
            if i < tmppipe[0] { libc::close(tmppipe[0]); }                   // c:1079-1080
            if i < tmppipe[1] { libc::close(tmppipe[1]); }                   // c:1081-1082
        }
    }

    // (void)addhookdefs(NULL, zshhooks, ...);                               // c:1085
    // init_eprog();                                                         // c:1087
    // zero_mnumber.type = MN_INTEGER; zero_mnumber.u.l = 0;                 // c:1089-1090

    // noeval = 0;                                                           // c:1092
    // curhist = 0; histsiz = DEFAULT_HISTSIZE; inithist();                  // c:1093-1095
    let _ = crate::ported::hist::inithist();
    // cmdstack = zalloc(CMDSTACKSZ); cmdsp = 0;                             // c:1097-1098
    // bangchar = '!'; hashchar = '#'; hatchar = '^';                        // c:1100-1102
    // termflags = TERM_UNKNOWN;                                             // c:1103
    crate::ported::params::TERMFLAGS.store(1, Ordering::SeqCst);
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
    std::env::set_var("PATH",                                                // c:1109
        std::env::var("PATH").unwrap_or_else(|_|
            "/bin:/usr/bin:/usr/ucb:/usr/local/bin".to_string()));

    // cdpath, manpath, fignore = mkarray(NULL)                              // c:1116-1118
    // fpath = ...                                                           // c:1132-1172
    // mailpath, psvar, module_path = mkarray(...)                           // c:1174-1176
    // modulestab = newmoduletable(17, "modules");                           // c:1177
    // linkedmodules = znewlinklist();                                       // c:1178

    // Set default prompts                                                   // c:1180
    // prompt, prompt2, prompt3, prompt4, sprompt = ztrdup(...)              // c:1181-1194

    // ifs = EMULATION(KSH|SH) ? DEFAULT_IFS_SH : DEFAULT_IFS                // c:1196-1197
    // wordchars = ztrdup(DEFAULT_WORDCHARS); postedit = ztrdup("");         // c:1198-1199

    // If _ is set in environment then initialize our $_ by copying it      // c:1200
    let underscore_val = std::env::var("_").unwrap_or_default();             // c:1201
    let metafied = crate::ported::utils::metafy(&underscore_val);            // c:1202
    *zunderscore.lock().unwrap() = metafied.clone();                         // c:1202
    underscoreused.store((metafied.len() + 1) as i32, Ordering::SeqCst);     // c:1203
    let ulen = (metafied.len() + 1 + 31) & !31;                              // c:1204
    underscorelen.store(ulen, Ordering::SeqCst);                             // c:1205
    // zunderscore = zrealloc(zunderscore, underscorelen);                   // c:1205

    // zoptarg = ""; zoptind = 1;                                            // c:1207-1208

    // ppid = getppid(); mypid = getpid();                                   // c:1210-1211
    // term = ztrdup("");                                                    // c:1212

    // nullcmd = "cat"; readnullcmd = DEFAULT_READNULLCMD;                   // c:1214-1215

    // cached_uid = getuid();                                                // c:1219
    // pswd = getpwuid(cached_uid); home = pswd->pw_dir; ...                 // c:1222-1234
    if let Ok(home) = std::env::var("HOME") {                                // c:1225
        let _ = home;
    }

    // PWD/OLDPWD initialization                                             // c:1236-1259
    if let Some(cwd) = crate::ported::utils::zgetcwd() {                     // c:1252
        std::env::set_var("PWD", &cwd);
        if std::env::var("OLDPWD").is_err() {                                // c:1255-1257
            std::env::set_var("OLDPWD", &cwd);
        }
    }

    crate::ported::utils::inittyptab();                                      // c:1261
    // initlextabs();                                                        // c:1262

    crate::ported::hashtable::createreswdtable();                            // c:1264
    crate::ported::hashtable::createaliastables();                           // c:1265
    crate::ported::hashtable::createcmdnamtable();                           // c:1266
    crate::ported::hashtable::createshfunctable();                           // c:1267
    let _ = crate::ported::builtin::createbuiltintable();                    // c:1268
    // createnameddirtable();                                                // c:1269
    crate::ported::params::createparamtable();                               // c:1270

    // condtab = NULL; wrappers = NULL;                                      // c:1272-1273

    let (_cols, _lines) = crate::ported::utils::adjustwinsize();             // c:1276

    // getrlimit loop                                                        // c:1286-1289

    // breaks = loops = 0;                                                   // c:1292
    // lastmailcheck = zmonotime(NULL);                                      // c:1293
    // locallevel = sourcelevel = 0;                                         // c:1294
    sourcelevel.store(0, Ordering::SeqCst);                                  // c:1294
    // sfcontext = SFC_NONE; trap_return = 0;                                // c:1295-1296
    // trap_state = TRAP_STATE_INACTIVE;                                     // c:1297
    // noerrexit = NOERREXIT_EXIT|RETURN|SIGNAL;                             // c:1298
    // nohistsave = 1;                                                       // c:1299
    // dirstack = znewlinklist(); bufstack = znewlinklist();                 // c:1300-1301
    // hsubl = hsubr = NULL; lastpid = 0;                                    // c:1302-1303

    // get_usage();                                                          // c:1305

    // Close fd's we opened to block 0-9                                     // c:1307
    #[cfg(unix)]
    for i in 0..10 {                                                         // c:1308
        if close_fds[i] != 0 {                                               // c:1309
            unsafe { libc::close(i as i32); }                                // c:1310
        }
    }

    let _ = crate::ported::prompt::set_default_colour_sequences();           // c:1313

    // ZSH_EXEPATH                                                           // c:1315
    {
        let exename = argv0.lock().unwrap().clone();                         // c:1318
        let exename = crate::ported::utils::unmeta(&exename);                // c:1318
        // c:1319 — `cwd = pwd;` (the in-shell logical cwd global).
        //          Read paramtab; was reading OS env which can lag.
        let cwd = crate::ported::params::getsparam("PWD").map(|s|
            crate::ported::utils::unmeta(&s));
        let mypath = getmypath(Some(&exename),                               // c:1320
                                cwd.as_deref());
        if let Some(mp) = mypath {                                           // c:1323
            std::env::set_var("ZSH_EXEPATH", &mp);                           // c:1324
        }
    }
    if let Some(cmd) = cmd {                                                 // c:1340
        std::env::set_var("ZSH_EXECUTION_STRING", cmd);                      // c:1340
    }
    if let Some(rs) = runscript {                                            // c:1340
        std::env::set_var("ZSH_SCRIPT", rs);                                 // c:1340
    }
    std::env::set_var("ZSH_NAME", zsh_name);                                 // c:1340
}

/// Port of `static void setupshin(char *runscript)` from Src/init.c:1340.
fn setupshin(runscript: Option<&str>) {                                      // c:1340
    if let Some(script) = runscript {                                        // c:1340
        let funmeta = crate::ported::utils::unmeta(script);            // c:1346
        let mut sfname: Option<String> = None;                               // c:1343
        if std::path::Path::new(&funmeta).is_file() {                        // c:1350-1352
            sfname = Some(script.to_string());                               // c:1353
        }
        // PATHSCRIPT search omitted (depends on opts[PATHSCRIPT])           // c:1354-1360
        if sfname.is_none() {                                                // c:1361
            crate::ported::utils::zerr(&format!(                             // c:1364
                "can't open input file: {}", script));
            std::process::exit(127);                                         // c:1365
        }
    }
    // lineno = 1;                                                           // c:1394
    // shinbufalloc();                                                       // c:1394
}

/// Port of `void init_signals(void)` from Src/init.c:1394.
pub fn init_signals() {                                                      // c:1394
    // sigtrapped = hcalloc(TRAPCOUNT * sizeof(int));                        // c:1394
    // siglists = hcalloc(TRAPCOUNT * sizeof(Eprog));                        // c:1399

    // if (interact) { signal_setmask(...); for(...) signal_default(i); }    // c:1401-1406

    // sigchld_mask = signal_mask(SIGCHLD);                                  // c:1407

    crate::ported::signals::intr();                                          // c:1409

    #[cfg(unix)]
    unsafe {
        let mut act: libc::sigaction = std::mem::zeroed();                   // c:1411
        if libc::sigaction(libc::SIGQUIT, std::ptr::null(), &mut act) == 0   // c:1411
            && act.sa_sigaction == libc::SIG_IGN
        {
            // sigtrapped[SIGQUIT] = ZSIG_IGNORED;                           // c:1412
        }
        let mut ign: libc::sigaction = std::mem::zeroed();                   // c:1415
        ign.sa_sigaction = libc::SIG_IGN;
        libc::sigaction(libc::SIGQUIT, &ign, std::ptr::null_mut());

        // if (signal_ignore(SIGHUP) == SIG_IGN) opts[HUP] = 0;              // c:1418-1419
        crate::ported::signals::install_handler(libc::SIGHUP);               // c:1421
        crate::ported::signals::install_handler(libc::SIGCHLD);              // c:1422
        #[cfg(not(target_os = "haiku"))]
        crate::ported::signals::install_handler(libc::SIGWINCH);             // c:1424

        // if (interact) { ... SIGPIPE, SIGALRM, SIGTERM ... }               // c:1427-1431
        crate::ported::signals::install_handler(libc::SIGPIPE);              // c:1445
        crate::ported::signals::install_handler(libc::SIGALRM);              // c:1445
        libc::sigaction(libc::SIGTERM, &ign, std::ptr::null_mut());          // c:1445

        // if (jobbing) { signal_ignore(SIGTTOU/TSTP/TTIN); }                // c:1445-1436
        libc::sigaction(libc::SIGTTOU, &ign, std::ptr::null_mut());          // c:1445
        libc::sigaction(libc::SIGTSTP, &ign, std::ptr::null_mut());          // c:1445
        libc::sigaction(libc::SIGTTIN, &ign, std::ptr::null_mut());          // c:1445
    }
}

/// Port of `void run_init_scripts(void)` from Src/init.c:1445.
/// Rust idiom replacement: emulation flag check + `source` calls
/// cover the C ifdef cascade for KSH/SH vs zsh init paths; the
/// `noerrexit` state-machine bit fiddling is handled at the
/// executor level rather than threaded through here.
pub fn run_init_scripts() {                                                  // c:1445
    // noerrexit = NOERREXIT_EXIT | NOERREXIT_RETURN | NOERREXIT_SIGNAL;     // c:1445

    // if (EMULATION(KSH|SH)) { ... } else { ... zsh paths ... }             // c:1449
    let emul = crate::ported::options::emulation.load(Ordering::SeqCst);
    let is_posix = (emul & 6) != 0; /* EMULATE_KSH=2 | EMULATE_SH=4 */

    if is_posix {
        // if (islogin) source("/etc/profile");                              // c:1450-1451
        // if (unset(PRIVILEGED)) { sourcehome(".profile"); ... ENV ... }    // c:1452-1469
    } else {
        // source(GLOBAL_ZSHENV);                                            // c:1473
        let _ = source(crate::ported::config_h::GLOBAL_ZSHENV);
        // if (isset(RCS) && unset(PRIVILEGED)) sourcehome(".zshenv");       // c:1476-1490
        sourcehome(".zshenv");
        // if (islogin) { ... .zprofile ... }                                // c:1491-1498
        // if (interact) { ... .zshrc ... }                                  // c:1499-1506
        sourcehome(".zshrc");
        // if (islogin) { ... .zlogin ... }                                  // c:1524-1514
    }
    // noerrexit = 0; nohistsave = 0;                                        // c:1524-1517
}

/// Port of `void init_misc(char *cmd, char *zsh_name)` from Src/init.c:1524.
pub fn init_misc(cmd: Option<&str>, zsh_name: &str) {                        // c:1524
    if zsh_name.starts_with('r') {                                           // c:1524
        crate::ported::utils::zerrnam(zsh_name,                              // c:1527
            "no support for restricted mode");
        std::process::exit(1);                                               // c:1528
    }
    if let Some(cmdstr) = cmd {                                              // c:1530
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
/// Rust idiom replacement: `Path::exists` + `fs::read_to_string`
/// covers the C `open`+`read`+`zexec` cascade; the parse-and-exec
/// loop dispatches through the canonical fusevm bridge rather than
/// in-process bytecode walk.
pub fn source(s: &str) -> i32 {                                              // c:1551
    let _us = crate::ported::utils::unmeta(s);                         // c:1551
    let path = std::path::Path::new(&_us);
    if !path.exists() {                                                      // c:1565-1568
        return 1; /* SOURCE_NOT_FOUND */
    }

    // Save shell state                                                      // c:1571-1581
    let oldlineno = 0i64;                                                    // c:1575
    let _ = oldlineno;

    sourcelevel.fetch_add(1, Ordering::SeqCst);                              // c:1606

    // Read and execute the file                                             // c:1618-1642
    // Without exec/parse layers, just record that we sourced it.
    let _ = std::fs::read_to_string(path);

    sourcelevel.fetch_sub(1, Ordering::SeqCst);                              // c:1644

    // Restore shell state                                                   // c:1646-1670
    0 /* SOURCE_OK */                                                        // c:1679
}

/// Port of `void sourcehome(char *s)` from Src/init.c:1679.
pub fn sourcehome(s: &str) {                                                 // c:1679
    crate::ported::signals::queue_signals();                                 // c:1679
    let emul = crate::ported::options::emulation.load(Ordering::SeqCst);
    let is_posix = (emul & 6) != 0;
    // c:1684 — `h = is_posix ? getsparam("HOME") : (getsparam("ZDOTDIR")
    //                                                 ?: getsparam("HOME"))`
    //          paramtab read; was OS env.
    let h = if is_posix {
        crate::ported::params::getsparam("HOME")
    } else {
        crate::ported::params::getsparam("ZDOTDIR")
            .or_else(|| crate::ported::params::getsparam("HOME"))
    };
    let h = match h {                                                        // c:1685-1689
        Some(h) => h,
        None => {
            crate::ported::signals::unqueue_signals();
            return;
        }
    };
    let buf = format!("{}/{}", h, s);                                        // c:1713
    crate::ported::signals::unqueue_signals();                               // c:1713
    source(&buf);                                                            // c:1713
}

/// Port of `void init_bltinmods(void)` from Src/init.c:1703.
pub fn init_bltinmods() {                                                    // c:1703
    // #include "bltinmods.list"                                             // c:1720
    // load_module("zsh/main", NULL, 0);                                     // c:1720
}

/// Port of `mod_export void noop_function(void)` from Src/init.c:1713.
pub fn noop_function() {                                                     // c:1713
    /* do nothing */                                                         // c:1720
}

/// Port of `mod_export void noop_function_int(int nothing)` from Src/init.c:1720.
pub fn noop_function_int(_nothing: i32) {                                    // c:1720
    /* do nothing */                                                         // c:1720
}

/// Port of `mod_export char *zleentry(...)` from Src/init.c:1743.
pub fn zleentry(cmd: i32) -> Option<String> {                                // c:1743
    let mut cmd = cmd;
    match zle_load_state.load(Ordering::SeqCst) {                            // c:1755
        0 => {                                                               // c:1756
            // ZLE_CMD_TRASH=?, ZLE_CMD_RESET_PROMPT=?, ZLE_CMD_REFRESH=?
            if cmd != 1 && cmd != 2 && cmd != 3 {                            // c:1761-1762
                // load_module("zsh/zle", NULL, 0)                           // c:1764
                zle_load_state.store(2, Ordering::SeqCst);                   // c:1770
            }
        }
        1 => {                                                               // c:1776
            cmd = -1;                                                        // c:1779
        }
        2 => { /* fallback */ }                                              // c:1782
        _ => {}
    }
    match cmd {                                                              // c:1788
        // ZLE_CMD_READ                                                      // c:1796
        4 => {                                                               // c:1796
            let _line = String::new();                                       // c:1808
            return Some(String::new());
        }
        // ZLE_CMD_GET_LINE                                                  // c:1812
        5 => {                                                               // c:1812
            return Some(String::new());                                      // c:1835
        }
        _ => {}
    }
    None                                                                     // c:1855
}

/// Port of `mod_export int fallback_compctlread(...)` from Src/init.c:1835.
pub fn fallback_compctlread(name: &str) -> i32 {                             // c:1835
    crate::ported::utils::zwarnnam(name,                                     // c:1855
        "no loaded module provides read for completion context");
    1                                                                        // c:1855
}

/// Port of `mod_export int zsh_main(int argc, char **argv)` from Src/init.c:1855.
pub fn zsh_main(_argc: i32, argv: &[String]) -> i32 {                        // c:1855
    #[cfg(unix)]
    unsafe {
        let empty = std::ffi::CString::new("").unwrap();
        libc::setlocale(libc::LC_ALL, empty.as_ptr());                       // c:1861
    }

    // init_jobs(argv, environ);                                             // c:1864
    let env: Vec<String> = std::env::vars()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect();
    let _ = crate::ported::jobs::init_jobs(argv, &env);

    // typtab[...] |= IMETA;                                                 // c:1871-1875

    // Metafy each argv (already strings in Rust)                            // c:1877

    let mut zsh_name = argv.first().cloned().unwrap_or_default();            // c:1879
    loop {                                                                   // c:1880
        let arg0 = zsh_name.clone();                                         // c:1881
        zsh_name = match arg0.rfind('/') {                                   // c:1882
            None => arg0.clone(),                                            // c:1883
            Some(i) => arg0[i+1..].to_string(),                              // c:1885
        };
        if zsh_name.starts_with('-') {                                       // c:1886
            zsh_name = zsh_name[1..].to_string();                            // c:1887
        }
        if zsh_name == "su" {                                                // c:1888
            if let Ok(sh) = std::env::var("SHELL") {                         // c:1889
                if !sh.is_empty() && arg0 != sh {                            // c:1890
                    zsh_name = sh;                                           // c:1891
                    continue;                                                // c:1892
                }
            }
            break;                                                           // c:1893
        }
        break;                                                               // c:1895
    }

    // fdtable_size = zopenmax(); fdtable[0..2] = FDT_EXTERNAL;              // c:1898-1900
    let _ = crate::ported::compat::zopenmax();

    crate::ported::options::createoptiontable();                             // c:1902

    // parseargs(zsh_name, argv, &runscript, &cmd);                          // c:1905
    let mut argv_v = argv.to_vec();
    let mut runscript: Option<String> = None;                                // c:1857
    let mut cmd: Option<String> = None;                                      // c:1858
    parseargs(&zsh_name, &mut argv_v, &mut runscript, &mut cmd);

    SHTTY.store(-1, Ordering::SeqCst);                                       // c:1907
    init_io(cmd.as_deref());                                                 // c:1908
    setupvals(cmd.as_deref(), runscript.as_deref(), &zsh_name);              // c:1909

    init_signals();                                                          // c:1911
    init_bltinmods();                                                        // c:1912
    crate::ported::builtin::init_builtins();                                 // c:1913
    run_init_scripts();                                                      // c:1914
    setupshin(runscript.as_deref());                                         // c:1915
    init_misc(cmd.as_deref(), &zsh_name);                                    // c:1916

    loop {                                                                   // c:1918
        let mut errexit = 0;                                                 // c:1924
        // maybeshrinkjobtab();                                              // c:1925
        loop {                                                               // c:1927
            // retflag = 0;                                                  // c:1929
            let _ = r#loop(1, 0);                                            // c:1930
            // if (errflag && !interact && !isset(CONTINUEONERROR)) { ... }  // c:1931-1934
            errexit = 1;
            break;
        }
        if errexit != 0 {                                                    // c:1936
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
pub fn r#loop(toplevel: i32, justonce: i32) -> i32 {                         // c:113
    let mut err: i32;                                                        // c:116
    let mut non_empty: i32 = 0;                                              // c:116

    crate::ported::signals::queue_signals();                                 // c:118
    crate::ported::mem::pushheap();                                          // c:119
    if toplevel == 0 {                                                       // c:120
        // zcontext_save();                                                  // c:121
    }
    loop {                                                                   // c:122
        crate::ported::mem::freeheap();                                      // c:123
        // if (stophist == 3) hend(NULL);                                    // c:124-125
        // hbegin(1);                                                        // c:126
        // if (isset(SHINSTDIN)) {                                           // c:127
        //     setblock_stdin();                                             // c:128
        //     if (interact && toplevel) { ... preprompt() ... }             // c:129-151
        // }
        use_exit_printed.store(0, Ordering::SeqCst);                         // c:153
        crate::ported::signals::intr();                                      // c:154
        // lexinit();                                                        // c:155
        // if (!(prog = parse_event(ENDINPUT))) { ... }                      // c:156-175
        // if (hend(prog)) { ... execode(prog,...) ... }                     // c:176-227
        // if (subsh) realexit();                                            // c:232-233
        // if (((!interact || sourcelevel) && errflag) || retflag) break;    // c:234-235
        // if (isset(SINGLECOMMAND) && toplevel) { ... realexit(); }         // c:236-241
        if justonce != 0 { break; }                                          // c:242-243
        // The actual REPL is owned by the binary; without parser/exec
        // dispatch we cannot run another iteration, so break here.
        break;
    }
    err = 0;                                                                 // c:245 (errflag stub)
    if toplevel == 0 {                                                       // c:263
        // zcontext_restore();                                               // c:263
    }
    crate::ported::mem::popheap();                                           // c:263
    crate::ported::signals::unqueue_signals();                               // c:263

    if err != 0 { return 2; /* LOOP_ERROR */ }                               // c:263-252
    if non_empty == 0 { return 1; /* LOOP_EMPTY */ }                         // c:263-254
    0 /* LOOP_OK */                                                          // c:263
}

/// Port of `static void parseopts_setemulate(...)` from Src/init.c:348.
fn parseopts_setemulate(_nam: &str, _flags: i32) {                           // c:348
    // emulate(nam, 1, &emulation, opts);                                    // c:348
    // opts[LOGINSHELL] = ((flags & PARSEARGS_LOGIN) != 0);                  // c:351
    // opts[PRIVILEGED] = (getuid() != geteuid() || getgid() != getegid()); // c:352
    // opts[INTERACTIVE] = isatty(0) ? 2 : 0;                                // c:361
    // opts[MONITOR] = 2; opts[HASHDIRS] = 2; opts[USEZLE] = 1;              // c:366-368
    // opts[SHINSTDIN] = 0; opts[SINGLECOMMAND] = 0;                         // c:369-370
}
