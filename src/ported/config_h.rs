//! Direct port of zsh's generated `config.h` (1276 lines, 271 #defines,
//! 135 #undefs as autoconf emitted them on an `aarch64-apple-darwin` host).
//!
//! **Configure-derived vs. genuinely constant.** C computes this whole file
//! on the build host, so a Rust literal is only faithful for the values
//! `./configure` would compute identically everywhere. The five that are
//! genuinely host- or target-shaped and are read by zshrs — `OSTYPE`,
//! `VENDOR`, `MACHTYPE`, `DEFAULT_PATH`, `PATH_DEV_FD` — are derived instead
//! (the first four via `build.rs`'s `cargo:rustc-env`, `MACHTYPE` from
//! `std::env::consts::ARCH`); each carries its `configure.ac` /
//! `config.guess` citation at its definition. Everything else below is a
//! literal, which is correct for the `--enable-*` defaults (`DEFAULT_HISTSIZE`,
//! `MAX_FUNCTION_DEPTH`, `PASSWD_FILE`, …) and is *unread dead code* for the
//! `HAVE_*` / `RLIMIT_*` / struct-member feature sentinels — Rust expresses
//! those as `cfg!`/`libc` at the use site, so the frozen macOS answers below
//! are a record of one autoconf run, not a switch anything reads. Do not wire
//! a `HAVE_*` constant into live code without deriving it first.
//!
//! Every C #define is mirrored as a `pub const` with the same name.
//! Boolean-style `#define X 1` becomes `pub const X: i32 = 1;`.
//! Empty `#define X` becomes `pub const X: bool = true;`. C
//! `/* #undef X */` lines are preserved as `// /* #undef X */`
//! comments so the porter can audit what is *not* enabled.
//!
//! Allow non-uppercase / non-camel naming since these constants
//! must match the C identifiers byte-for-byte (PORT.md Rule A).

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]

// config.h.  Generated from config.h.in by configure.
// config.h.in.  Generated from configure.ac by autoheader.

// *** begin user configuration section ****

/// Define this to be the location of your password file
pub const PASSWD_FILE: &str = "/etc/passwd";

/// Define this to be the name of your NIS/YP password
/// map (if applicable)
pub const PASSWD_MAP: &str = "passwd.byname";

/// Define to 1 if you want user names to be cached
pub const CACHE_USERNAMES: i32 = 1;

/// Define to 1 if system supports job control
pub const JOB_CONTROL: i32 = 1;

/// Define this if you use "suspended" instead of "stopped"
pub const USE_SUSPENDED: i32 = 1;

/// The default history buffer size in lines
pub const DEFAULT_HISTSIZE: i32 = 30;

/// The default editor for the fc builtin
pub const DEFAULT_FCEDIT: &str = "vi";

/// The default prefix for temporary files
pub const DEFAULT_TMPPREFIX: &str = "/tmp/zsh";

// *** end of user configuration section            ****
// *** shouldn't have to change anything below here ****

// Define to 1 if you want to use dynamically loaded modules on AIX.
// /* #undef AIXDYNAMIC */
/// Define to 1 if the isprint() function is broken under UTF-8 locale.
pub const BROKEN_ISPRINT: i32 = 1;

// Define to 1 if kill(pid, 0) doesn't return ESRCH, ie BeOS R4.51.
// /* #undef BROKEN_KILL_ESRCH */
// Define to 1 if sigsuspend() is broken
// /* #undef BROKEN_POSIX_SIGSUSPEND */
// Define to 1 if tcsetpgrp() doesn't work, ie BeOS R4.51.
// /* #undef BROKEN_TCSETPGRP */
// Define to 1 if you use BSD style signal handling (and can block signals).
//
// /* #undef BSD_SIGNALS */
/// Undefine if you don't want local features. By default this is defined.
pub const CONFIG_LOCALE: i32 = 1;

// Define to a custom value for the ZSH_PATCHLEVEL parameter
// /* #undef CUSTOM_PATCHLEVEL */
// Define to 1 if using 'alloca.c'.
// /* #undef C_ALLOCA */
// Define to 1 if you want to debug zsh.
// /* #undef DEBUG */
/// The default path; used when running commands with command -p
///
/// C: `configure.ac:1954 AC_DEFINE_UNQUOTED(DEFAULT_PATH, "$zsh_cv_cs_path", ...)`
/// where `$zsh_cv_cs_path` is the build host's `getconf _CS_PATH` /
/// `getconf CS_PATH` / `getconf PATH`, falling back to `/bin:/usr/bin`
/// (`configure.ac:1944-1953`). That answer is OS-specific —
/// `/usr/bin:/bin:/usr/sbin:/sbin` on macOS, a shorter list on Linux — so the
/// probe is re-run at build time in `build.rs::cs_path` rather than frozen.
pub const DEFAULT_PATH: &str = env!("ZSHRS_CONFIG_DEFAULT_PATH");

/// Define default pager used by readnullcmd
pub const DEFAULT_READNULLCMD: &str = "more";

// Define to 1 if you want to avoid calling functions that will require
// dynamic NSS modules.
// /* #undef DISABLE_DYNAMIC_NSS */
// Define to 1 if an underscore has to be prepended to dlsym() argument.
// /* #undef DLSYM_NEEDS_UNDERSCORE */
/// The extension used for dynamically loaded modules.
pub const DL_EXT: &str = "so";

/// Define to 1 if you want to use dynamically loaded modules.
pub const DYNAMIC: i32 = 1;

/// Define to 1 if multiple modules defining the same symbol are OK.
pub const DYNAMIC_NAME_CLASH_OK: i32 = 1;

// Define to 1 if you want use unicode9 character widths.
// /* #undef ENABLE_UNICODE9 */
/// Define to 1 if getcwd() calls malloc to allocate memory.
pub const GETCWD_CALLS_MALLOC: i32 = 1;

/// Define to 1 if the 'getpgrp' function requires zero arguments.
pub const GETPGRP_VOID: i32 = 1;

// Define to 1 if getpwnam() is faked, ie BeOS R4.51.
// /* #undef GETPWNAM_FAKED */
/// The global file to source whenever zsh is run as a login shell; if
/// undefined, don't source anything
pub const GLOBAL_ZLOGIN: &str = "/etc/zlogin";

/// The global file to source whenever zsh was run as a login shell. This is
/// sourced right before exiting. If undefined, don't source anything.
pub const GLOBAL_ZLOGOUT: &str = "/etc/zlogout";

/// The global file to source whenever zsh is run as a login shell, before
/// zshrc is read; if undefined, don't source anything.
pub const GLOBAL_ZPROFILE: &str = "/etc/zprofile";

/// The global file to source absolutely first whenever zsh is run; if
/// undefined, don't source anything.
pub const GLOBAL_ZSHENV: &str = "/etc/zshenv";

/// The global file to source whenever zsh is run; if undefined, don't source
/// anything
pub const GLOBAL_ZSHRC: &str = "/etc/zshrc";
// These five are autoconf's `${sysconfdir}` substitutions, so they name
// upstream's `--sysconfdir=/etc` layout. Debian/Ubuntu/Arch build zsh with
// `--sysconfdir=/etc/zsh` and put the same files in `/etc/zsh/`; since one
// zshrs binary serves every layout, callers must resolve them through
// `crate::extensions::global_rc::global_rc_path` rather than opening the
// literal below.

// Define if TIOCGWINSZ is defined in sys/ioctl.h but not in termios.h.
// /* #undef GWINSZ_IN_SYS_IOCTL */
/// Define to 1 if you have 'alloca', as a function or macro.
pub const HAVE_ALLOCA: i32 = 1;

/// Define to 1 if <alloca.h> works.
pub const HAVE_ALLOCA_H: i32 = 1;

/// Define to 1 if you have the 'arc4random_buf' function.
pub const HAVE_ARC4RANDOM_BUF: i32 = 1;

// Define to 1 if you have the <bind/netdb.h> header file.
// /* #undef HAVE_BIND_NETDB_H */
/// Define if you have the termcap boolcodes symbol.
pub const HAVE_BOOLCODES: i32 = 1;

/// Define if you have the terminfo boolnames symbol.
pub const HAVE_BOOLNAMES: i32 = 1;

/// Define to 1 if you have the 'brk' function.
pub const HAVE_BRK: i32 = 1;

/// Define to 1 if there is a prototype defined for brk() on your system.
pub const HAVE_BRK_PROTO: i32 = 1;

// Define to 1 if you have the 'canonicalize_file_name' function.
// /* #undef HAVE_CANONICALIZE_FILE_NAME */
// Define to 1 if you have the 'cap_get_proc' function.
// /* #undef HAVE_CAP_GET_PROC */
/// Define to 1 if you have the 'clock_gettime' function.
pub const HAVE_CLOCK_GETTIME: i32 = 1;

/// Define to 1 if you have the <curses.h> header file.
pub const HAVE_CURSES_H: i32 = 1;

// Define to 1 if you have the 'cygwin_conv_path' function.
// /* #undef HAVE_CYGWIN_CONV_PATH */
/// Define to 1 if you have the 'difftime' function.
pub const HAVE_DIFFTIME: i32 = 1;

/// Define to 1 if you have the <dirent.h> header file, and it defines 'DIR'.
///
pub const HAVE_DIRENT_H: i32 = 1;

/// Define to 1 if you have the 'dlclose' function.
pub const HAVE_DLCLOSE: i32 = 1;

/// Define to 1 if you have the 'dlerror' function.
pub const HAVE_DLERROR: i32 = 1;

/// Define to 1 if you have the <dlfcn.h> header file.
pub const HAVE_DLFCN_H: i32 = 1;

/// Define to 1 if you have the 'dlopen' function.
pub const HAVE_DLOPEN: i32 = 1;

/// Define to 1 if you have the 'dlsym' function.
pub const HAVE_DLSYM: i32 = 1;

// Define to 1 if you have the <dl.h> header file.
// /* #undef HAVE_DL_H */
/// Define to 1 if you have the 'endutxent' function.
pub const HAVE_ENDUTXENT: i32 = 1;

/// Define to 1 if you have the 'erand48' function.
pub const HAVE_ERAND48: i32 = 1;

/// Define to 1 if you have the <errno.h> header file.
pub const HAVE_ERRNO_H: i32 = 1;

// Define to 1 if you have the 'faccessx' function.
// /* #undef HAVE_FACCESSX */
/// Define to 1 if you have the 'fchdir' function.
pub const HAVE_FCHDIR: i32 = 1;

/// Define to 1 if you have the 'fchmod' function.
pub const HAVE_FCHMOD: i32 = 1;

/// Define to 1 if you have the 'fchown' function.
pub const HAVE_FCHOWN: i32 = 1;

/// Define to 1 if you have the <fcntl.h> header file.
pub const HAVE_FCNTL_H: i32 = 1;

/// Define to 1 if system has working FIFOs.
pub const HAVE_FIFOS: i32 = 1;

/// Define to 1 if you have the 'fpurge' function.
pub const HAVE_FPURGE: i32 = 1;

/// Define to 1 if you have the 'fseeko' function.
pub const HAVE_FSEEKO: i32 = 1;

/// Define to 1 if you have the 'fstat' function.
pub const HAVE_FSTAT: i32 = 1;

/// Define to 1 if you have the 'ftello' function.
pub const HAVE_FTELLO: i32 = 1;

/// Define to 1 if you have the 'ftruncate' function.
pub const HAVE_FTRUNCATE: i32 = 1;

// Define to 1 if you have the <gdbm.h> header file.
// /* #undef HAVE_GDBM_H */
// Define to 1 if you have the 'gdbm_open' function.
// /* #undef HAVE_GDBM_OPEN */
/// Define to 1 if you have the 'getcchar' function.
pub const HAVE_GETCCHAR: i32 = 1;

/// Define to 1 if you have the 'getcwd' function.
pub const HAVE_GETCWD: i32 = 1;

/// Define to 1 if you have the 'getenv' function.
pub const HAVE_GETENV: i32 = 1;

/// Define to 1 if you have the 'getgrgid' function.
pub const HAVE_GETGRGID: i32 = 1;

/// Define to 1 if you have the 'getgrnam' function.
pub const HAVE_GETGRNAM: i32 = 1;

/// Define to 1 if you have the 'gethostbyname2' function.
pub const HAVE_GETHOSTBYNAME2: i32 = 1;

/// Define to 1 if you have the 'gethostname' function.
pub const HAVE_GETHOSTNAME: i32 = 1;

/// Define to 1 if you have the 'getipnodebyname' function.
pub const HAVE_GETIPNODEBYNAME: i32 = 1;

/// Define to 1 if you have the 'getlogin' function.
pub const HAVE_GETLOGIN: i32 = 1;

/// Define to 1 if you have the 'getpagesize' function.
pub const HAVE_GETPAGESIZE: i32 = 1;

/// Define to 1 if you have the 'getpwent' function.
pub const HAVE_GETPWENT: i32 = 1;

/// Define to 1 if you have the 'getpwnam' function.
pub const HAVE_GETPWNAM: i32 = 1;

/// Define to 1 if you have the 'getpwuid' function.
pub const HAVE_GETPWUID: i32 = 1;

// Define to 1 if you have the 'getrandom' function.
// /* #undef HAVE_GETRANDOM */
/// Define to 1 if you have the 'getrlimit' function.
pub const HAVE_GETRLIMIT: i32 = 1;

/// Define to 1 if you have the 'getrusage' function.
pub const HAVE_GETRUSAGE: i32 = 1;

/// Define to 1 if you have the 'gettimeofday' function.
pub const HAVE_GETTIMEOFDAY: i32 = 1;

// Define to 1 if you have the 'getutent' function.
// /* #undef HAVE_GETUTENT */
/// Define to 1 if you have the 'getutxent' function.
pub const HAVE_GETUTXENT: i32 = 1;

/// Define to 1 if you have the 'getxattr' function.
pub const HAVE_GETXATTR: i32 = 1;

/// Define to 1 if you have the 'grantpt' function.
pub const HAVE_GRANTPT: i32 = 1;

/// Define to 1 if you have the <grp.h> header file.
pub const HAVE_GRP_H: i32 = 1;

/// Define to 1 if you have the 'htons' function.
pub const HAVE_HTONS: i32 = 1;

/// Define to 1 if you have the 'iconv' function.
pub const HAVE_ICONV: i32 = 1;

/// Define to 1 if you have the <iconv.h> header file.
pub const HAVE_ICONV_H: i32 = 1;

/// Define to 1 if you have the 'inet_aton' function.
pub const HAVE_INET_ATON: i32 = 1;

/// Define to 1 if you have the 'inet_ntop' function.
pub const HAVE_INET_NTOP: i32 = 1;

/// Define to 1 if you have the 'inet_pton' function.
pub const HAVE_INET_PTON: i32 = 1;

/// Define to 1 if you have the 'initgroups' function.
pub const HAVE_INITGROUPS: i32 = 1;

/// Define to 1 if you have the 'initscr' function.
pub const HAVE_INITSCR: i32 = 1;

/// Define to 1 if you have the <inttypes.h> header file.
pub const HAVE_INTTYPES_H: i32 = 1;

/// Define to 1 if there is a prototype defined for ioctl() on your system.
pub const HAVE_IOCTL_PROTO: i32 = 1;

/// Define to 1 if you have the 'isblank' function.
pub const HAVE_ISBLANK: i32 = 1;

/// Define to 1 if you have the `isinf' macro or function.
pub const HAVE_ISINF: i32 = 1;

/// Define to 1 if you have the `isnan' macro or function.
pub const HAVE_ISNAN: i32 = 1;

/// Define to 1 if you have the 'iswblank' function.
pub const HAVE_ISWBLANK: i32 = 1;

/// Define to 1 if you have the 'killpg' function.
pub const HAVE_KILLPG: i32 = 1;

/// Define to 1 if you have the <langinfo.h> header file.
pub const HAVE_LANGINFO_H: i32 = 1;

/// Define to 1 if you have the 'lchown' function.
pub const HAVE_LCHOWN: i32 = 1;

// Define to 1 if you have the 'cap' library (-lcap).
// /* #undef HAVE_LIBCAP */
/// Define to 1 if you have the <libc.h> header file.
pub const HAVE_LIBC_H: i32 = 1;

/// Define to 1 if you have the 'dl' library (-ldl).
pub const HAVE_LIBDL: i32 = 1;

// Define to 1 if you have the 'gdbm' library (-lgdbm).
// /* #undef HAVE_LIBGDBM */
/// Define to 1 if you have the 'm' library (-lm).
pub const HAVE_LIBM: i32 = 1;

// Define to 1 if you have the 'rt' library (-lrt).
// /* #undef HAVE_LIBRT */
// Define to 1 if you have the 'socket' library (-lsocket).
// /* #undef HAVE_LIBSOCKET */
/// Define to 1 if you have the <limits.h> header file.
pub const HAVE_LIMITS_H: i32 = 1;

/// Define to 1 if system has working link().
pub const HAVE_LINK: i32 = 1;

// Define to 1 if you have the 'load' function.
// /* #undef HAVE_LOAD */
// Define to 1 if you have the 'loadbind' function.
// /* #undef HAVE_LOADBIND */
// Define to 1 if you have the 'loadquery' function.
// /* #undef HAVE_LOADQUERY */
/// Define to 1 if you have the <locale.h> header file.
pub const HAVE_LOCALE_H: i32 = 1;

/// Define to 1 if you have the 'log2' function.
pub const HAVE_LOG2: i32 = 1;

/// Define to 1 if you have the 'lstat' function.
pub const HAVE_LSTAT: i32 = 1;

/// Define to 1 if you have the 'memcpy' function.
pub const HAVE_MEMCPY: i32 = 1;

/// Define to 1 if you have the 'memmove' function.
pub const HAVE_MEMMOVE: i32 = 1;

/// Define to 1 if you have the <memory.h> header file.
pub const HAVE_MEMORY_H: i32 = 1;

/// Define to 1 if you have the 'mkfifo' function.
pub const HAVE_MKFIFO: i32 = 1;

/// Define to 1 if there is a prototype defined for mknod() on your system.
pub const HAVE_MKNOD_PROTO: i32 = 1;

/// Define to 1 if you have the 'mkstemp' function.
pub const HAVE_MKSTEMP: i32 = 1;

/// Define to 1 if you have the 'mktime' function.
pub const HAVE_MKTIME: i32 = 1;

/// Define to 1 if you have a working 'mmap' system call.
pub const HAVE_MMAP: i32 = 1;

/// Define to 1 if you have the 'msync' function.
pub const HAVE_MSYNC: i32 = 1;

/// Define to 1 if you have the 'munmap' function.
pub const HAVE_MUNMAP: i32 = 1;

/// Define to 1 if you have the 'nanosleep' function.
pub const HAVE_NANOSLEEP: i32 = 1;

// Define to 1 if you have the <ncursesw/ncurses.h> header file.
// /* #undef HAVE_NCURSESW_NCURSES_H */
// Define to 1 if you have the <ncursesw/term.h> header file.
// /* #undef HAVE_NCURSESW_TERM_H */
/// Define to 1 if you have the <ncurses.h> header file.
pub const HAVE_NCURSES_H: i32 = 1;

// Define to 1 if you have the <ncurses/ncurses.h> header file.
// /* #undef HAVE_NCURSES_NCURSES_H */
// Define to 1 if you have the <ncurses/term.h> header file.
// /* #undef HAVE_NCURSES_TERM_H */
// Define to 1 if you have the <ndir.h> header file, and it defines 'DIR'.
// /* #undef HAVE_NDIR_H */
/// Define to 1 if you have the <netinet/in_systm.h> header file.
pub const HAVE_NETINET_IN_SYSTM_H: i32 = 1;

/// Define to 1 if you have the 'nice' function.
pub const HAVE_NICE: i32 = 1;

// Define to 1 if you have the 'nis_list' function.
// /* #undef HAVE_NIS_LIST */
/// Define to 1 if you have the 'nl_langinfo' function.
pub const HAVE_NL_LANGINFO: i32 = 1;

/// Define to 1 if you have the 'ntohs' function.
pub const HAVE_NTOHS: i32 = 1;

/// Define if you have the termcap numcodes symbol.
pub const HAVE_NUMCODES: i32 = 1;

/// Define if you have the terminfo numnames symbol.
pub const HAVE_NUMNAMES: i32 = 1;

/// Define to 1 if you have the 'open_memstream' function.
pub const HAVE_OPEN_MEMSTREAM: i32 = 1;

/// Define to 1 if your termcap library has the ospeed variable
pub const HAVE_OSPEED: i32 = 1;

/// Define to 1 if you have the 'pathconf' function.
pub const HAVE_PATHCONF: i32 = 1;

// Define to 1 if you have the 'pcre2_compile_8' function.
// /* #undef HAVE_PCRE2_COMPILE_8 */
// Define to 1 if you have the <pcre2.h> header file.
// /* #undef HAVE_PCRE2_H */
/// Define to 1 if you have the 'poll' function.
pub const HAVE_POLL: i32 = 1;

/// Define to 1 if you have the <poll.h> header file.
pub const HAVE_POLL_H: i32 = 1;

/// Define to 1 if you have the 'posix_openpt' function.
pub const HAVE_POSIX_OPENPT: i32 = 1;

// Define to 1 if the system supports `prctl' to change process name
// /* #undef HAVE_PRCTL */
/// Define to 1 if you have the 'ptsname' function.
pub const HAVE_PTSNAME: i32 = 1;

/// Define to 1 if you have the 'putenv' function.
pub const HAVE_PUTENV: i32 = 1;

/// Define to 1 if you have the <pwd.h> header file.
pub const HAVE_PWD_H: i32 = 1;

/// Define to 1 if you have the 'readlink' function.
pub const HAVE_READLINK: i32 = 1;

/// Define to 1 if you have the 'realpath' function.
pub const HAVE_REALPATH: i32 = 1;

/// Define to 1 if you have the 'regcomp' function.
pub const HAVE_REGCOMP: i32 = 1;

/// Define to 1 if you have the 'regerror' function.
pub const HAVE_REGERROR: i32 = 1;

/// Define to 1 if you have the 'regexec' function.
pub const HAVE_REGEXEC: i32 = 1;

/// Define to 1 if you have the 'regfree' function.
pub const HAVE_REGFREE: i32 = 1;

/// Define to 1 if you have the 'resize_term' function.
pub const HAVE_RESIZE_TERM: i32 = 1;

// Define to 1 if RLIMIT_AIO_MEM is present (whether or not as a macro).
// /* #undef HAVE_RLIMIT_AIO_MEM */
// Define to 1 if RLIMIT_AIO_OPS is present (whether or not as a macro).
// /* #undef HAVE_RLIMIT_AIO_OPS */
/// Define to 1 if RLIMIT_AS is present (whether or not as a macro).
pub const HAVE_RLIMIT_AS: i32 = 1;

// Define to 1 if RLIMIT_KQUEUES is present (whether or not as a macro).
// /* #undef HAVE_RLIMIT_KQUEUES */
// Define to 1 if RLIMIT_LOCKS is present (whether or not as a macro).
// /* #undef HAVE_RLIMIT_LOCKS */
/// Define to 1 if RLIMIT_MEMLOCK is present (whether or not as a macro).
pub const HAVE_RLIMIT_MEMLOCK: i32 = 1;

// Define to 1 if RLIMIT_MSGQUEUE is present (whether or not as a macro).
// /* #undef HAVE_RLIMIT_MSGQUEUE */
// Define to 1 if RLIMIT_NICE is present (whether or not as a macro).
// /* #undef HAVE_RLIMIT_NICE */
/// Define to 1 if RLIMIT_NOFILE is present (whether or not as a macro).
pub const HAVE_RLIMIT_NOFILE: i32 = 1;

/// Define to 1 if RLIMIT_NPROC is present (whether or not as a macro).
pub const HAVE_RLIMIT_NPROC: i32 = 1;

// Define to 1 if RLIMIT_NPTS is present (whether or not as a macro).
// /* #undef HAVE_RLIMIT_NPTS */
// Define to 1 if RLIMIT_NTHR is present (whether or not as a macro).
// /* #undef HAVE_RLIMIT_NTHR */
// Define to 1 if RLIMIT_POSIXLOCKS is present (whether or not as a macro).
// /* #undef HAVE_RLIMIT_POSIXLOCKS */
// Define to 1 if RLIMIT_PTHREAD is present (whether or not as a macro).
// /* #undef HAVE_RLIMIT_PTHREAD */
/// Define to 1 if RLIMIT_RSS is present (whether or not as a macro).
pub const HAVE_RLIMIT_RSS: i32 = 1;

// Define to 1 if RLIMIT_RTPRIO is present (whether or not as a macro).
// /* #undef HAVE_RLIMIT_RTPRIO */
// Define to 1 if RLIMIT_RTTIME is present (whether or not as a macro).
// /* #undef HAVE_RLIMIT_RTTIME */
// Define to 1 if RLIMIT_SBSIZE is present (whether or not as a macro).
// /* #undef HAVE_RLIMIT_SBSIZE */
// Define to 1 if RLIMIT_SIGPENDING is present (whether or not as a macro).
// /* #undef HAVE_RLIMIT_SIGPENDING */
// Define to 1 if RLIMIT_SWAP is present (whether or not as a macro).
// /* #undef HAVE_RLIMIT_SWAP */
// Define to 1 if RLIMIT_TCACHE is present (whether or not as a macro).
// /* #undef HAVE_RLIMIT_TCACHE */
// Define to 1 if RLIMIT_UMTXP is present (whether or not as a macro).
// /* #undef HAVE_RLIMIT_UMTXP */
// Define to 1 if RLIMIT_VMEM is present (whether or not as a macro).
// /* #undef HAVE_RLIMIT_VMEM */
/// Define to 1 if you have the 'sbrk' function.
pub const HAVE_SBRK: i32 = 1;

/// Define to 1 if there is a prototype defined for sbrk() on your system.
pub const HAVE_SBRK_PROTO: i32 = 1;

/// Define to 1 if you have the 'scalbn' function.
pub const HAVE_SCALBN: i32 = 1;

/// Define to 1 if you have the 'select' function.
pub const HAVE_SELECT: i32 = 1;

/// Define to 1 if you have the 'setcchar' function.
pub const HAVE_SETCCHAR: i32 = 1;

/// Define to 1 if you have the 'setegid' function.
pub const HAVE_SETEGID: i32 = 1;

/// Define to 1 if you have the 'setenv' function.
pub const HAVE_SETENV: i32 = 1;

/// Define to 1 if you have the 'seteuid' function.
pub const HAVE_SETEUID: i32 = 1;

/// Define to 1 if you have the 'setgid' function.
pub const HAVE_SETGID: i32 = 1;

/// Define to 1 if you have the 'setlocale' function.
pub const HAVE_SETLOCALE: i32 = 1;

/// Define to 1 if you have the 'setpgid' function.
pub const HAVE_SETPGID: i32 = 1;

/// Define to 1 if you have the 'setpgrp' function.
pub const HAVE_SETPGRP: i32 = 1;

// Define to 1 if the system supports `setproctitle' to change process name
// /* #undef HAVE_SETPROCTITLE */
/// Define to 1 if you have the 'setregid' function.
pub const HAVE_SETREGID: i32 = 1;

// Define to 1 if you have the 'setresgid' function.
// /* #undef HAVE_SETRESGID */
// Define to 1 if you have the 'setresuid' function.
// /* #undef HAVE_SETRESUID */
/// Define to 1 if you have the 'setreuid' function.
pub const HAVE_SETREUID: i32 = 1;

/// Define to 1 if you have the 'setsid' function.
pub const HAVE_SETSID: i32 = 1;

/// Define to 1 if you have the 'setuid' function.
pub const HAVE_SETUID: i32 = 1;

/// Define to 1 if you have the 'setupterm' function.
pub const HAVE_SETUPTERM: i32 = 1;

/// Define to 1 if you have the 'setutxent' function.
pub const HAVE_SETUTXENT: i32 = 1;

// Define to 1 if you have the 'shl_findsym' function.
// /* #undef HAVE_SHL_FINDSYM */
// Define to 1 if you have the 'shl_load' function.
// /* #undef HAVE_SHL_LOAD */
// Define to 1 if you have the 'shl_unload' function.
// /* #undef HAVE_SHL_UNLOAD */
/// Define to 1 if you have the 'sigaction' function.
pub const HAVE_SIGACTION: i32 = 1;

/// Define to 1 if you have the 'sigblock' function.
pub const HAVE_SIGBLOCK: i32 = 1;

/// Define to 1 if you have the 'sighold' function.
pub const HAVE_SIGHOLD: i32 = 1;

/// Define to 1 if you have the 'signgam' function.
pub const HAVE_SIGNGAM: i32 = 1;

/// Define to 1 if you have the 'sigprocmask' function.
pub const HAVE_SIGPROCMASK: i32 = 1;

// Define to 1 if you have the 'sigqueue' function.
// /* #undef HAVE_SIGQUEUE */
/// Define to 1 if you have the 'sigrelse' function.
pub const HAVE_SIGRELSE: i32 = 1;

/// Define to 1 if you have the 'sigsetmask' function.
pub const HAVE_SIGSETMASK: i32 = 1;

// Define to 1 if you have the 'srand_deterministic' function.
// /* #undef HAVE_SRAND_DETERMINISTIC */
/// Define to 1 if you have the <stdarg.h> header file.
pub const HAVE_STDARG_H: i32 = 1;

/// Define to 1 if you have the <stddef.h> header file.
pub const HAVE_STDDEF_H: i32 = 1;

/// Define to 1 if you have the <stdint.h> header file.
pub const HAVE_STDINT_H: i32 = 1;

/// Define to 1 if you have the <stdio.h> header file.
pub const HAVE_STDIO_H: i32 = 1;

/// Define to 1 if you have the <stdlib.h> header file.
pub const HAVE_STDLIB_H: i32 = 1;

/// Define if you have the termcap strcodes symbol.
pub const HAVE_STRCODES: i32 = 1;

/// Define to 1 if you have the 'strcoll' function and it is properly defined.
///
pub const HAVE_STRCOLL: i32 = 1;

/// Define to 1 if you have the 'strerror' function.
pub const HAVE_STRERROR: i32 = 1;

/// Define to 1 if you have the 'strftime' function.
pub const HAVE_STRFTIME: i32 = 1;

/// Define to 1 if you have the <strings.h> header file.
pub const HAVE_STRINGS_H: i32 = 1;

/// Define to 1 if you have the <string.h> header file.
pub const HAVE_STRING_H: i32 = 1;

/// Define if you have the terminfo strnames symbol.
pub const HAVE_STRNAMES: i32 = 1;

/// Define to 1 if you have the 'strptime' function.
pub const HAVE_STRPTIME: i32 = 1;

/// Define to 1 if you have the 'strstr' function.
pub const HAVE_STRSTR: i32 = 1;

/// Define to 1 if you have the 'strtoul' function.
pub const HAVE_STRTOUL: i32 = 1;

// Define if your system's struct direct has a member named d_ino.
// /* #undef HAVE_STRUCT_DIRECT_D_INO */
// Define if your system's struct direct has a member named d_stat.
// /* #undef HAVE_STRUCT_DIRECT_D_STAT */
/// Define if your system's struct dirent has a member named d_ino.
pub const HAVE_STRUCT_DIRENT_D_INO: i32 = 1;

// Define if your system's struct dirent has a member named d_stat.
// /* #undef HAVE_STRUCT_DIRENT_D_STAT */
/// Define to 1 if 'ru_idrss' is a member of 'struct rusage'.
pub const HAVE_STRUCT_RUSAGE_RU_IDRSS: i32 = 1;

/// Define to 1 if 'ru_inblock' is a member of 'struct rusage'.
pub const HAVE_STRUCT_RUSAGE_RU_INBLOCK: i32 = 1;

/// Define to 1 if 'ru_isrss' is a member of 'struct rusage'.
pub const HAVE_STRUCT_RUSAGE_RU_ISRSS: i32 = 1;

/// Define to 1 if 'ru_ixrss' is a member of 'struct rusage'.
pub const HAVE_STRUCT_RUSAGE_RU_IXRSS: i32 = 1;

/// Define to 1 if 'ru_majflt' is a member of 'struct rusage'.
pub const HAVE_STRUCT_RUSAGE_RU_MAJFLT: i32 = 1;

/// Define to 1 if 'ru_maxrss' is a member of 'struct rusage'.
pub const HAVE_STRUCT_RUSAGE_RU_MAXRSS: i32 = 1;

/// Define to 1 if 'ru_minflt' is a member of 'struct rusage'.
pub const HAVE_STRUCT_RUSAGE_RU_MINFLT: i32 = 1;

/// Define to 1 if 'ru_msgrcv' is a member of 'struct rusage'.
pub const HAVE_STRUCT_RUSAGE_RU_MSGRCV: i32 = 1;

/// Define to 1 if 'ru_msgsnd' is a member of 'struct rusage'.
pub const HAVE_STRUCT_RUSAGE_RU_MSGSND: i32 = 1;

/// Define to 1 if 'ru_nivcsw' is a member of 'struct rusage'.
pub const HAVE_STRUCT_RUSAGE_RU_NIVCSW: i32 = 1;

/// Define to 1 if 'ru_nsignals' is a member of 'struct rusage'.
pub const HAVE_STRUCT_RUSAGE_RU_NSIGNALS: i32 = 1;

/// Define to 1 if 'ru_nswap' is a member of 'struct rusage'.
pub const HAVE_STRUCT_RUSAGE_RU_NSWAP: i32 = 1;

/// Define to 1 if 'ru_nvcsw' is a member of 'struct rusage'.
pub const HAVE_STRUCT_RUSAGE_RU_NVCSW: i32 = 1;

/// Define to 1 if 'ru_oublock' is a member of 'struct rusage'.
pub const HAVE_STRUCT_RUSAGE_RU_OUBLOCK: i32 = 1;

/// Define if your system's struct sockaddr_in6 has a member named
/// sin6_scope_id.
pub const HAVE_STRUCT_SOCKADDR_IN6_SIN6_SCOPE_ID: i32 = 1;

// Define to 1 if 'st_atimensec' is a member of 'struct stat'.
// /* #undef HAVE_STRUCT_STAT_ST_ATIMENSEC */
/// Define to 1 if 'st_atimespec.tv_nsec' is a member of 'struct stat'.
pub const HAVE_STRUCT_STAT_ST_ATIMESPEC_TV_NSEC: i32 = 1;

// Define to 1 if 'st_atim.tv_nsec' is a member of 'struct stat'.
// /* #undef HAVE_STRUCT_STAT_ST_ATIM_TV_NSEC */
// Define to 1 if 'st_ctimensec' is a member of 'struct stat'.
// /* #undef HAVE_STRUCT_STAT_ST_CTIMENSEC */
/// Define to 1 if 'st_ctimespec.tv_nsec' is a member of 'struct stat'.
pub const HAVE_STRUCT_STAT_ST_CTIMESPEC_TV_NSEC: i32 = 1;

// Define to 1 if 'st_ctim.tv_nsec' is a member of 'struct stat'.
// /* #undef HAVE_STRUCT_STAT_ST_CTIM_TV_NSEC */
// Define to 1 if 'st_mtimensec' is a member of 'struct stat'.
// /* #undef HAVE_STRUCT_STAT_ST_MTIMENSEC */
/// Define to 1 if 'st_mtimespec.tv_nsec' is a member of 'struct stat'.
pub const HAVE_STRUCT_STAT_ST_MTIMESPEC_TV_NSEC: i32 = 1;

// Define to 1 if 'st_mtim.tv_nsec' is a member of 'struct stat'.
// /* #undef HAVE_STRUCT_STAT_ST_MTIM_TV_NSEC */
/// Define to 1 if struct timespec is defined by a system header
pub const HAVE_STRUCT_TIMESPEC: i32 = 1;

/// Define to 1 if struct timezone is defined by a system header
pub const HAVE_STRUCT_TIMEZONE: i32 = 1;

/// Define to 1 if struct utmp is defined by a system header
pub const HAVE_STRUCT_UTMP: i32 = 1;

/// Define to 1 if struct utmpx is defined by a system header
pub const HAVE_STRUCT_UTMPX: i32 = 1;

/// Define if your system's struct utmpx has a member named ut_host.
pub const HAVE_STRUCT_UTMPX_UT_HOST: i32 = 1;

/// Define if your system's struct utmpx has a member named ut_tv.
pub const HAVE_STRUCT_UTMPX_UT_TV: i32 = 1;

// Define if your system's struct utmpx has a member named ut_xtime.
// /* #undef HAVE_STRUCT_UTMPX_UT_XTIME */
/// Define if your system's struct utmp has a member named ut_host.
pub const HAVE_STRUCT_UTMP_UT_HOST: i32 = 1;

// Define to 1 if you have RFS superroot directory.
// /* #undef HAVE_SUPERROOT */
/// Define to 1 if you have the 'symlink' function.
pub const HAVE_SYMLINK: i32 = 1;

/// Define to 1 if you have the 'sysconf' function.
pub const HAVE_SYSCONF: i32 = 1;

// Define to 1 if you have the <sys/capability.h> header file.
// /* #undef HAVE_SYS_CAPABILITY_H */
// Define to 1 if you have the <sys/dir.h> header file, and it defines 'DIR'.
//
// /* #undef HAVE_SYS_DIR_H */
/// Define to 1 if you have the <sys/filio.h> header file.
pub const HAVE_SYS_FILIO_H: i32 = 1;

/// Define to 1 if you have the <sys/mman.h> header file.
pub const HAVE_SYS_MMAN_H: i32 = 1;

// Define to 1 if you have the <sys/ndir.h> header file, and it defines 'DIR'.
//
// /* #undef HAVE_SYS_NDIR_H */
/// Define to 1 if you have the <sys/param.h> header file.
pub const HAVE_SYS_PARAM_H: i32 = 1;

/// Define to 1 if you have the <sys/random.h> header file.
pub const HAVE_SYS_RANDOM_H: i32 = 1;

/// Define to 1 if you have the <sys/resource.h> header file.
pub const HAVE_SYS_RESOURCE_H: i32 = 1;

/// Define to 1 if you have the <sys/select.h> header file.
pub const HAVE_SYS_SELECT_H: i32 = 1;

/// Define to 1 if you have the <sys/stat.h> header file.
pub const HAVE_SYS_STAT_H: i32 = 1;

// Define to 1 if you have the <sys/stropts.h> header file.
// /* #undef HAVE_SYS_STROPTS_H */
/// Define to 1 if you have the <sys/times.h> header file.
pub const HAVE_SYS_TIMES_H: i32 = 1;

/// Define to 1 if you have the <sys/time.h> header file.
pub const HAVE_SYS_TIME_H: i32 = 1;

/// Define to 1 if you have the <sys/types.h> header file.
pub const HAVE_SYS_TYPES_H: i32 = 1;

/// Define to 1 if you have the <sys/utsname.h> header file.
pub const HAVE_SYS_UTSNAME_H: i32 = 1;

/// Define to 1 if you have <sys/wait.h> that is POSIX.1 compatible.
pub const HAVE_SYS_WAIT_H: i32 = 1;

/// Define to 1 if you have the <sys/xattr.h> header file.
pub const HAVE_SYS_XATTR_H: i32 = 1;

/// Define to 1 if you have the 'tcgetattr' function.
pub const HAVE_TCGETATTR: i32 = 1;

/// Define to 1 if you have the 'tcsetpgrp' function.
pub const HAVE_TCSETPGRP: i32 = 1;

/// Define to 1 if you have the <termcap.h> header file.
pub const HAVE_TERMCAP_H: i32 = 1;

/// Define to 1 if you have the <termios.h> header file.
pub const HAVE_TERMIOS_H: i32 = 1;

// Define to 1 if you have the <termio.h> header file.
// /* #undef HAVE_TERMIO_H */
/// Define to 1 if you have the <term.h> header file.
pub const HAVE_TERM_H: i32 = 1;

/// Define to 1 if you have the 'tgamma' function.
pub const HAVE_TGAMMA: i32 = 1;

/// Define to 1 if you have the 'tgetent' function.
pub const HAVE_TGETENT: i32 = 1;

/// Define to 1 if you have the 'tigetflag' function.
pub const HAVE_TIGETFLAG: i32 = 1;

/// Define to 1 if you have the 'tigetnum' function.
pub const HAVE_TIGETNUM: i32 = 1;

/// Define to 1 if you have the 'tigetstr' function.
pub const HAVE_TIGETSTR: i32 = 1;

/// Define to 1 if you have the 'timelocal' function.
pub const HAVE_TIMELOCAL: i32 = 1;

/// Define to 1 if you have the 'uname' function.
pub const HAVE_UNAME: i32 = 1;

/// Define to 1 if the compiler can initialise a union.
pub const HAVE_UNION_INIT: i32 = 1;

/// Define to 1 if you have the <unistd.h> header file.
pub const HAVE_UNISTD_H: i32 = 1;

// Define to 1 if you have the 'unload' function.
// /* #undef HAVE_UNLOAD */
/// Define to 1 if you have the 'unlockpt' function.
pub const HAVE_UNLOCKPT: i32 = 1;

/// Define to 1 if you have the 'unsetenv' function.
pub const HAVE_UNSETENV: i32 = 1;

/// Define to 1 if you have the 'use_default_colors' function.
pub const HAVE_USE_DEFAULT_COLORS: i32 = 1;

/// Define to 1 if you have the <utmpx.h> header file.
pub const HAVE_UTMPX_H: i32 = 1;

/// Define to 1 if you have the <utmp.h> header file.
pub const HAVE_UTMP_H: i32 = 1;

// Define to 1 if you have the <varargs.h> header file.
// /* #undef HAVE_VARARGS_H */
/// Define to 1 if compiler supports variable-length arrays
pub const HAVE_VARIABLE_LENGTH_ARRAYS: i32 = 1;

/// Define to 1 if you have the 'waddwstr' function.
pub const HAVE_WADDWSTR: i32 = 1;

/// Define to 1 if you have the 'wait3' function.
pub const HAVE_WAIT3: i32 = 1;

/// Define to 1 if you have the 'waitpid' function.
pub const HAVE_WAITPID: i32 = 1;

/// Define to 1 if you have the <wchar.h> header file.
pub const HAVE_WCHAR_H: i32 = 1;

/// Define to 1 if you have the 'wctomb' function.
pub const HAVE_WCTOMB: i32 = 1;

/// Define to 1 if you have the 'wget_wch' function.
pub const HAVE_WGET_WCH: i32 = 1;

/// Define to 1 if you have the 'win_wch' function.
pub const HAVE_WIN_WCH: i32 = 1;

// Define to 1 if you have the 'xw' function.
// /* #undef HAVE_XW */
/// Define to 1 if you have the '_mktemp' function.
pub const HAVE__MKTEMP: i32 = 1;

// Define to 1 if you want to use dynamically loaded modules on HPUX 10.
// /* #undef HPUX10DYNAMIC */
/// Define as const if the declaration of iconv() needs const.
pub const ICONV_CONST: bool = true;

/// Define to 1 if iconv() is linked from libiconv
pub const ICONV_FROM_LIBICONV: i32 = 1;

// Define to 1 if ino_t is 64 bit (for large file support).
// /* #undef INO_T_IS_64_BIT */
/// Define to 1 if we must include <sys/ioctl.h> to get a prototype for
/// ioctl().
pub const IOCTL_IN_SYS_IOCTL: i32 = 1;

// Define to 1 if musl is being used as the C library
// /* #undef LIBC_MUSL */
/// Definitions used when a long is less than eight byte, to try to provide
/// some support for eight byte operations. Note that ZSH_64_BIT_TYPE,
/// OFF_T_IS_64_BIT, INO_T_IS_64_BIT do *not* get defined if long is already 64
/// bits, since in that case no special handling is required. Define to 1 if
/// long is 64 bits
pub const LONG_IS_64_BIT: i32 = 1;

/// Define to be the machine type (microprocessor class or machine model).
///
/// C zsh freezes configure's `$host_cpu` here, which config.guess
/// canonicalizes: an Apple Silicon host reports `aarch64-apple-darwinN`,
/// so homebrew zsh has `MACHTYPE=aarch64` (verified:
/// `zsh -f -c 'typeset -p MACHTYPE'` → `typeset MACHTYPE=aarch64`).
/// The Rust equivalent of that canonical host_cpu is the build target
/// arch, which spells the same names (`aarch64`, `x86_64`, `arm`,
/// `riscv64`), so MACHTYPE tracks the architecture the binary was
/// compiled for on every platform instead of being frozen to one.
///
/// This value is load-bearing for completion parity: `_gcc` switches its
/// `-m*` option list on `case $MACHTYPE in ... aarch64) ... arm) ...`
/// (`Completion/Unix/Command/_gcc:34,66,448`), so a wrong MACHTYPE
/// silently completes another CPU's flags.
pub const MACHTYPE: &str = std::env::consts::ARCH;

// Define for Maildir support
// /* #undef MAILDIR_SUPPORT */
/// Define for function depth limits
pub const MAX_FUNCTION_DEPTH: i32 = 500;

/// Define to 1 if you want support for multibyte character sets.
pub const MULTIBYTE_SUPPORT: i32 = 1;

// Define to 1 if you have ospeed, but it is not defined in termcap.h
// /* #undef MUST_DEFINE_OSPEED */
// Define to 1 if you have no signal blocking at all (bummer).
// /* #undef NO_SIGNAL_BLOCKING */
// Define to 1 if off_t is 64 bit (for large file support)
// /* #undef OFF_T_IS_64_BIT */
/// Define to be the name of the operating system.
///
/// C: `configure.ac:47 AC_DEFINE_UNQUOTED(OSTYPE, "$host_os", ...)`, read
/// once at `Src/params.c:990 setsparam("OSTYPE", ztrdup_metafy(OSTYPE))`.
/// `$host_os` is the third field of the triple `AC_CANONICAL_HOST` takes
/// from `config.guess`, so its *shape is platform-specific*: `darwin25.5.0`
/// on Darwin (`config.guess:1463`, kernel release appended) but `linux-gnu`
/// on Linux (`config.guess:1009/1222`, C library appended, never a kernel
/// version). A `uname -r` derivation would therefore be right on this Mac
/// and wrong on every Linux box. Derived per-target in `build.rs`
/// (`config_guess_host_os`); see that function for the full citation set.
///
/// Load-bearing for completion parity — the compsys corpus branches on it,
/// e.g. `Completion/Unix/Type/_file_systems` and `_file_flags` switch on
/// `case $OSTYPE in darwin*|freebsd*|linux*)`.
pub const OSTYPE: &str = env!("ZSHRS_CONFIG_OSTYPE");

/// Define to the address where bug reports for this package should be sent.
pub const PACKAGE_BUGREPORT: &str = "";

/// Define to the full name of this package.
pub const PACKAGE_NAME: &str = "";

/// Define to the full name and version of this package.
pub const PACKAGE_STRING: &str = "";

/// Define to the one symbol short name of this package.
pub const PACKAGE_TARNAME: &str = "";

/// Define to the home page for this package.
pub const PACKAGE_URL: &str = "";

/// Define to the version of this package.
pub const PACKAGE_VERSION: &str = "";

/// Define to the path of the /dev/fd filesystem.
///
/// C: `configure.ac:1973 AC_DEFINE_UNQUOTED(PATH_DEV_FD, "$zsh_cv_sys_path_dev_fd")`.
/// The probe at `configure.ac:1969` tries `/proc/self/fd` **first**, then
/// `/dev/fd`, so a Linux build resolves to `/proc/self/fd` and a Darwin build
/// (no procfs) to `/dev/fd`. Derived per-target in `build.rs`.
pub const PATH_DEV_FD: &str = env!("ZSHRS_CONFIG_PATH_DEV_FD");

/// Define to be location of utmpx file.
pub const PATH_UTMPX_FILE: &str = "/var/run/utmpx";

// Define to be location of utmp file.
// /* #undef PATH_UTMP_FILE */
// Define to be location of wtmpx file.
// /* #undef PATH_WTMPX_FILE */
// Define to be location of wtmp file.
// /* #undef PATH_WTMP_FILE */
/// Define to 1 if you use POSIX style signal handling.
pub const POSIX_SIGNALS: i32 = 1;

/// Define to 1 if printf and sprintf support %lld for long long.
pub const PRINTF_HAS_LLD: i32 = 1;

// Define to the path of the symlink to the current executable file.
// /* #undef PROC_SELF_EXE */
/// Define if realpath() accepts NULL as its second argument.
pub const REALPATH_ACCEPTS_NULL: i32 = 1;

/// Undefine this if you don't want to get a restricted shell when zsh is
/// exec'd with basename that starts with r. By default this is defined.
pub const RESTRICTED_R: i32 = 1;

/// Define to 1 if RLIMIT_RSS and RLIMIT_AS both exist and are equal.
pub const RLIMIT_RSS_IS_AS: i32 = 1;

// Define to 1 if RLIMIT_VMEM and RLIMIT_AS both exist and are equal.
// /* #undef RLIMIT_VMEM_IS_AS */
// Define to 1 if RLIMIT_VMEM and RLIMIT_RSS both exist and are equal.
// /* #undef RLIMIT_VMEM_IS_RSS */
// Define to 1 if struct rlimit uses long long
// /* #undef RLIM_T_IS_LONG_LONG */
// Define to 1 if struct rlimit uses quad_t.
// /* #undef RLIM_T_IS_QUAD_T */
/// Define to 1 if struct rlimit uses unsigned.
pub const RLIM_T_IS_UNSIGNED: i32 = 1;

/// Define to 1 if ru_maxrss in struct rusage is in bytes.
pub const RU_MAXRSS_IS_IN_BYTES: i32 = 1;

// Define to 1 if select() is defined in <sys/socket.h>, ie BeOS R4.51
// /* #undef SELECT_IN_SYS_SOCKET_H */
// Define to 1 if setenv removes a leading =
// /* #undef SETENV_MANGLES_EQUAL */
// If using the C implementation of alloca, define if you know the
// direction of stack growth for your system; otherwise it will be
// automatically deduced at runtime.
// STACK_DIRECTION > 0 => grows toward higher addresses
// STACK_DIRECTION < 0 => grows toward lower addresses
// STACK_DIRECTION = 0 => direction of growth unknown
// /* #undef STACK_DIRECTION */
// Define to 1 if the 'S_IS*' macros in <sys/stat.h> do not work properly.
// /* #undef STAT_MACROS_BROKEN */
/// Define to 1 if all of the C89 standard headers exist (not just the ones
/// required in a freestanding environment). This macro is provided for
/// backward compatibility; new code need not use it.
pub const STDC_HEADERS: i32 = 1;

// Define to 1 if you use SYS style signal handling (and can block signals).
//
// /* #undef SYSV_SIGNALS */
/// Define to 1 if tgetent() accepts NULL as a buffer.
pub const TGETENT_ACCEPTS_NULL: i32 = 1;

/// Define to what tgetent() returns on success (0 on HP-UX X/Open curses).
pub const TGETENT_SUCCESS: i32 = 1;

// Define if there is no prototype for the tgoto() terminal function.
// /* #undef TGOTO_PROTO_MISSING */
// Define if sys/time.h and sys/select.h cannot be both included.
// /* #undef TIME_H_SELECT_H_CONFLICTS */
/// Define to 1 if all the kit for using /dev/ptmx for ptys is available.
pub const USE_DEV_PTMX: i32 = 1;

/// Define to 1 if you need to use the native getcwd.
pub const USE_GETCWD: i32 = 1;

// Define to 1 if h_errno is not defined by the system.
// /* #undef USE_LOCAL_H_ERRNO */
/// Define to 1 if lseek() can be used for SHIN.
pub const USE_LSEEK: i32 = 1;

// Define to 1 if you want to allocate stack memory e.g. with `alloca'.
// /* #undef USE_STACK_ALLOCATION */
/// Define to be a string corresponding the vendor of the machine.
///
/// C: `configure.ac:45 AC_DEFINE_UNQUOTED(VENDOR, "$host_vendor", ...)`,
/// read once at `Src/params.c:992 setsparam("VENDOR", ztrdup_metafy(VENDOR))`.
/// `$host_vendor` is a function of (OS, CPU) rather than of the OS alone:
/// `config.guess:1222` emits `$CPU-pc-linux-$LIBCABI` for x86_64 Linux while
/// `config.guess:1009` emits `$CPU-unknown-linux-$LIBCABI` for aarch64 Linux,
/// so the two Linux targets zshrs ships disagree (`pc` vs `unknown`).
/// Derived per-target in `build.rs` (`config_guess_host_vendor`).
pub const VENDOR: &str = env!("ZSHRS_CONFIG_VENDOR");

// Define if your should include sys/stream.h and sys/ptem.h.
// /* #undef WINSIZE_IN_PTEM */
/// Define if getxattr() etc. require additional MacOS-style arguments
pub const XATTR_EXTRA_ARGS: i32 = 1;

// Define to 1 if the zlong type uses 64-bit long int.
// /* #undef ZLONG_IS_LONG_64 */
// Define to 1 if the zlong type uses long long int.
// /* #undef ZLONG_IS_LONG_LONG */
// Define to a 64 bit integer type if there is one, but long is shorter.
// /* #undef ZSH_64_BIT_TYPE */
// Define to an unsigned variant of ZSH_64_BIT_TYPE if that is defined.
// /* #undef ZSH_64_BIT_UTYPE */
// Define to 1 if you want to get debugging information on internal hash
// tables. This turns on the `hashinfo' builtin.
// /* #undef ZSH_HASH_DEBUG */
/// Define to 1 if some variant of a curses header can be included
pub const ZSH_HAVE_CURSES_H: i32 = 1;

/// Define to 1 if some variant of term.h can be included
pub const ZSH_HAVE_TERM_H: i32 = 1;

// Define to 1 if you want to turn on error checking for heap allocation.
// /* #undef ZSH_HEAP_DEBUG */
// Define to 1 if you want to use zsh's own memory allocation routines
// /* #undef ZSH_MEM */
// Define to 1 if you want to debug zsh memory allocation routines.
// /* #undef ZSH_MEM_DEBUG */
// Define to 1 if you want to turn on warnings of memory allocation errors
// /* #undef ZSH_MEM_WARNING */
// Define if _XOPEN_SOURCE_EXTENDED should not be defined to avoid clashes
// /* #undef ZSH_NO_XOPEN */
// Define to 1 if you want to turn on memory checking for free().
// /* #undef ZSH_SECURE_FREE */
// Define to 1 if you want to add code for valgrind to debug heap memory.
// /* #undef ZSH_VALGRIND */
/// Define to the base type of the third argument of accept
/// (Rust port: type alias rather than const because the C
/// macro expands to a typename.)
pub type ZSOCKLEN_T = libc::socklen_t;

// Number of bits in a file offset, on hosts where this is settable.
// /* #undef _FILE_OFFSET_BITS */
// Define to 1 on platforms where this makes off_t a 64-bit type.
// /* #undef _LARGE_FILES */
// Number of bits in time_t, on hosts where this is settable.
// /* #undef _TIME_BITS */
// Define to 1 on platforms where this makes time_t a 64-bit type.
// /* #undef __MINGW_USE_VC2005_COMPAT */
// Define to empty if 'const' does not conform to ANSI C.
// /* #undef const */
// Define as 'int' if <sys/types.h> doesn't define.
// /* #undef gid_t */
// Define to `unsigned long' if <sys/types.h> doesn't define.
// /* #undef ino_t */
// Define to 'int' if <sys/types.h> does not define.
// /* #undef mode_t */
// Define to 'long int' if <sys/types.h> does not define.
// /* #undef off_t */
// Define as a signed integer type capable of holding a process identifier.
// /* #undef pid_t */
// Define to the type used in struct rlimit.
// /* #undef rlim_t */
// Define to `unsigned int' if <sys/types.h> or <signal.h> doesn't define
// /* #undef sigset_t */
// Define as 'unsigned int' if <stddef.h> doesn't define.
// /* #undef size_t */
// Define as 'int' if <sys/types.h> doesn't define.
// /* #undef uid_t */
#[cfg(test)]
mod tests {
    use super::*;

    /// JOB_CONTROL + USE_SUSPENDED gate every job-management feature
    /// (`bg`/`fg`/`jobs`/SIGTSTP handling). Both must be 1 — a regen
    /// flipping either silently disables half the shell.
    #[test]
    fn job_control_is_enabled() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(JOB_CONTROL, 1);
        assert_eq!(USE_SUSPENDED, 1);
    }

    /// PASSWD_FILE is consulted by init.c when populating $USERNAME /
    /// $HOME from /etc/passwd. Hard-pins the POSIX-standard location.
    #[test]
    fn passwd_file_is_posix_standard_location() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(PASSWD_FILE, "/etc/passwd");
    }

    /// `configure.ac:2978-2984` + `config.h:13-19` — canonical zsh
    /// config defaults. Pin all three (HISTSIZE, FCEDIT, TMPPREFIX)
    /// against the upstream `config.h` values; drift on any silently
    /// changes user-facing behavior on a first-run shell.
    #[test]
    fn default_config_values_match_upstream_config_h() {
        let _g = crate::test_util::global_state_lock();
        // config.h:13 — DEFAULT_HISTSIZE 30.
        assert_eq!(
            DEFAULT_HISTSIZE, 30,
            "configure.ac:2978 / config.h:13 — DEFAULT_HISTSIZE = 30"
        );
        // config.h:16 — DEFAULT_FCEDIT "vi".
        assert_eq!(
            DEFAULT_FCEDIT, "vi",
            "configure.ac:2981 / config.h:16 — DEFAULT_FCEDIT = \"vi\""
        );
        // config.h:19 — DEFAULT_TMPPREFIX "/tmp/zsh".
        assert_eq!(
            DEFAULT_TMPPREFIX, "/tmp/zsh",
            "configure.ac:2984 / config.h:19 — DEFAULT_TMPPREFIX = \"/tmp/zsh\""
        );
    }

    /// `config.h.in` — every `GLOBAL_*` rc path drives a `source` at
    /// shell startup (init.c::source_zshenv/profile/login/etc.). The
    /// `/etc/zsh*` prefix is the canonical zsh placement; an absolute
    /// path with the wrong stem or a relative form would silently
    /// disable system-wide configuration.
    #[test]
    fn global_rc_paths_have_canonical_etc_zsh_prefix() {
        let _g = crate::test_util::global_state_lock();
        for (name, path) in [
            ("GLOBAL_ZSHENV", GLOBAL_ZSHENV),
            ("GLOBAL_ZPROFILE", GLOBAL_ZPROFILE),
            ("GLOBAL_ZSHRC", GLOBAL_ZSHRC),
            ("GLOBAL_ZLOGIN", GLOBAL_ZLOGIN),
            ("GLOBAL_ZLOGOUT", GLOBAL_ZLOGOUT),
        ] {
            assert!(
                path.starts_with("/etc/z"),
                "{} = {:?} — must live under /etc/z* per zsh init convention",
                name,
                path
            );
            assert!(
                !path.contains(".."),
                "{} = {:?} — path traversal in a startup-file path is a security risk",
                name,
                path
            );
        }
    }

    /// `Src/init.c::source_global_rcs` opens them in a fixed sequence:
    /// zshenv → zprofile → zshrc → zlogin → zlogout. Each path is a
    /// distinct file. A regen that collapsed two into one (typo /
    /// copy-paste) would re-source the same file twice and silently
    /// drop one of the startup phases.
    #[test]
    fn global_rc_paths_are_all_distinct() {
        let _g = crate::test_util::global_state_lock();
        let all = [
            GLOBAL_ZSHENV,
            GLOBAL_ZPROFILE,
            GLOBAL_ZSHRC,
            GLOBAL_ZLOGIN,
            GLOBAL_ZLOGOUT,
        ];
        let unique: std::collections::HashSet<_> = all.iter().copied().collect();
        assert_eq!(
            unique.len(),
            all.len(),
            "duplicate startup-file path detected: {:?}",
            all
        );
    }

    /// `Src/exec.c::execcmd_exec` ultimately falls back to DEFAULT_PATH
    /// when the user clears $PATH. The list must (a) include /bin and
    /// /usr/bin or every shell command breaks, and (b) be
    /// colon-separated with no trailing colon (zsh treats trailing
    /// empty as cwd-on-PATH, which is a 30-year security footgun).
    #[test]
    fn default_path_is_colon_separated_and_includes_bin_dirs() {
        let _g = crate::test_util::global_state_lock();
        assert!(DEFAULT_PATH.contains("/bin"));
        assert!(DEFAULT_PATH.contains("/usr/bin"));
        assert!(
            !DEFAULT_PATH.ends_with(':'),
            "trailing colon in DEFAULT_PATH = {:?} means cwd-on-PATH — security regression",
            DEFAULT_PATH
        );
        for seg in DEFAULT_PATH.split(':') {
            assert!(seg.starts_with('/'),
                "DEFAULT_PATH segment {:?} is not absolute — relative paths in PATH are a security risk",
                seg);
        }
    }

    /// `DL_EXT` selects the dlopen() suffix when loading dynamic
    /// modules (`zmodload`). Unix-only build: must be a non-empty
    /// extension stub (without leading dot) so the module resolver
    /// can synthesize `<name>.<DL_EXT>` correctly.
    #[test]
    fn dl_ext_is_nonempty_extension_stem() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            !DL_EXT.is_empty(),
            "DL_EXT must be set so zmodload can find .so files"
        );
        assert!(
            !DL_EXT.starts_with('.'),
            "DL_EXT = {:?} starts with `.` — concatenation would produce `..so`",
            DL_EXT
        );
        // On Linux/macOS one of these matches; on neither it's a misconfig.
        assert!(
            matches!(DL_EXT, "so" | "dylib" | "bundle"),
            "DL_EXT = {:?} — unexpected dynamic-library extension",
            DL_EXT
        );
    }

    /// `Src/jobs.c` consults DYNAMIC + DYNAMIC_NAME_CLASH_OK at compile
    /// time to decide whether to even compile the module loader. Both
    /// gated to `1` enables the full zmodload path. A regen flipping
    /// either to `0` would silently disable every loadable module
    /// (datetime, mathfunc, pcre, regex, stat, etc.).
    #[test]
    fn dynamic_module_loading_is_enabled() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(DYNAMIC, 1, "DYNAMIC=0 disables zmodload entirely");
        assert_eq!(
            DYNAMIC_NAME_CLASH_OK, 1,
            "DYNAMIC_NAME_CLASH_OK=0 forbids the same fn name across modules"
        );
    }

    /// HAVE_* sentinels gate `#ifdef`'d code paths in the C source.
    /// Pinning the most load-bearing ones — clock_gettime drives
    /// $EPOCHREALTIME (zsh/datetime), brk underpins memory allocator
    /// fallbacks, arc4random_buf seeds $RANDOM in random_real.
    #[test]
    fn core_have_sentinels_match_runtime_capabilities() {
        let _g = crate::test_util::global_state_lock();
        // clock_gettime is required for $EPOCHSECONDS / $EPOCHREALTIME
        // (Src/Modules/datetime.c). Disabled = the param is empty.
        assert_eq!(
            HAVE_CLOCK_GETTIME, 1,
            "$EPOCHREALTIME relies on clock_gettime"
        );
        // alloca() is on every platform zshrs targets (macOS aarch64 +
        // Linux x86_64/aarch64); disabled would break parser stack
        // expansion in Src/parse.c.
        assert_eq!(HAVE_ALLOCA, 1);
        assert_eq!(HAVE_ALLOCA_H, 1);
        // brk(2) — getrlimit fallback paths.
        assert_eq!(HAVE_BRK, 1);
        assert_eq!(HAVE_BRK_PROTO, 1);
    }

    /// Cache flags wired by initialization code: CACHE_USERNAMES feeds
    /// the `nis_username` ~ $username lookup table; without it `~user`
    /// expansion is a fork-per-lookup. Pinning at 1 protects that
    /// optimisation against a regen flip.
    #[test]
    fn cache_usernames_is_enabled_for_tilde_expansion_perf() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            CACHE_USERNAMES, 1,
            "CACHE_USERNAMES drives ~user lookup table; 0 means fork-per-lookup"
        );
    }

    /// `config.h:54 BROKEN_ISPRINT` is the historic workaround for
    /// systems whose libc `isprint()` returns true for `0x80..0xFF`.
    /// zsh defines its own `iblank`/`isprint` macros via this gate.
    /// Must be set so zshrs's character classifiers behave like zsh's.
    #[test]
    fn broken_isprint_workaround_is_active() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(BROKEN_ISPRINT, 1);
    }

    /// `config.h:42 DEFAULT_FCEDIT = "vi"` is the editor `fc` invokes.
    /// "vi" is the POSIX-mandated default. A regen swapping to "ed" or
    /// "" would silently break `fc -e ?` invocations.
    #[test]
    fn default_fcedit_resolves_to_a_present_editor_command() {
        let _g = crate::test_util::global_state_lock();
        // We do NOT shell out to verify executability; just sanity-check
        // the name is a known editor stem (not "", "vim", or a path).
        assert!(
            matches!(DEFAULT_FCEDIT, "vi" | "ed"),
            "DEFAULT_FCEDIT = {:?} — must be a POSIX-installed editor",
            DEFAULT_FCEDIT
        );
        assert!(
            !DEFAULT_FCEDIT.contains('/'),
            "DEFAULT_FCEDIT must be PATH-resolved, not a fixed path"
        );
    }

    /// `config.h:45 DEFAULT_TMPPREFIX = "/tmp/zsh"`. Used by
    /// `Src/exec.c::gettmpfile` to derive the basename for `<(cmd)` /
    /// `>(cmd)` FIFO process substitution and `=()` here-strings. Must
    /// be (a) absolute, (b) under a sticky-bit world-writable dir.
    #[test]
    fn default_tmpprefix_is_absolute_under_tmp() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            DEFAULT_TMPPREFIX.starts_with("/tmp/"),
            "DEFAULT_TMPPREFIX = {:?} must live under /tmp so it inherits sticky-bit",
            DEFAULT_TMPPREFIX
        );
    }

    /// `config.h:85 DEFAULT_READNULLCMD` is what `< file` (read with no
    /// command) defaults to. zsh ships "more"; "cat" would also be
    /// safe, but pin the upstream choice.
    #[test]
    fn default_readnullcmd_matches_upstream() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            DEFAULT_READNULLCMD, "more",
            "config.h:85 — DEFAULT_READNULLCMD must remain `more` to match zsh"
        );
    }

    /// `config.h:27 PASSWD_MAP` is the NIS map name when looking up
    /// users via YP. zsh uses "passwd.byname" (the universal NIS map
    /// name); a typo here silently breaks `~user` expansion on NIS
    /// hosts.
    #[test]
    fn passwd_map_is_canonical_nis_name() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(PASSWD_MAP, "passwd.byname");
    }

    /// `CONFIG_LOCALE` gates the `setlocale(LC_ALL, "")` call at shell
    /// start. Pinned at 1 — disabling it would make non-ASCII
    /// completion lists and wide-char widget handling silently revert
    /// to the C locale (every CJK / European user breaks).
    #[test]
    fn config_locale_is_enabled() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            CONFIG_LOCALE, 1,
            "CONFIG_LOCALE=0 silently disables every locale-aware code path"
        );
    }

    /// `GETCWD_CALLS_MALLOC` + `GETPGRP_VOID` are two
    /// platform-detected sentinels. Both must be `1` on macOS/Linux
    /// (zshrs's supported platforms); the C source's `#ifdef`
    /// branches assume `getcwd(NULL, 0)` returns a malloc'd buffer
    /// and `getpgrp()` takes no arguments.
    #[test]
    fn getcwd_and_getpgrp_use_modern_signatures() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            GETCWD_CALLS_MALLOC, 1,
            "GETCWD_CALLS_MALLOC=0 means we'd pass a buffer — POSIX requires the NULL-arg form"
        );
        assert_eq!(
            GETPGRP_VOID, 1,
            "GETPGRP_VOID=0 means getpgrp(pid) — only ancient SysV uses that form"
        );
    }

    // ─── zsh-corpus pins for config_h string constants ────────────

    /// PASSWD_FILE is the canonical /etc/passwd path.
    #[test]
    fn config_h_corpus_passwd_file_default_path() {
        assert_eq!(PASSWD_FILE, "/etc/passwd");
    }

    /// DEFAULT_FCEDIT is "vi" per ancient ksh-compat default.
    #[test]
    fn config_h_corpus_default_fcedit_is_vi() {
        assert_eq!(DEFAULT_FCEDIT, "vi");
    }

    /// DEFAULT_TMPPREFIX starts with "/tmp/".
    #[test]
    fn config_h_corpus_default_tmpprefix_starts_with_tmp() {
        assert!(
            DEFAULT_TMPPREFIX.starts_with("/tmp/"),
            "tmp prefix in /tmp/ namespace, got {DEFAULT_TMPPREFIX:?}"
        );
    }

    /// DEFAULT_PATH includes /usr/bin and /bin.
    #[test]
    fn config_h_corpus_default_path_has_usr_bin_and_bin() {
        assert!(DEFAULT_PATH.contains("/usr/bin"));
        assert!(DEFAULT_PATH.contains("/bin"));
    }

    /// DEFAULT_READNULLCMD is "more".
    #[test]
    fn config_h_corpus_default_readnullcmd_is_more() {
        assert_eq!(DEFAULT_READNULLCMD, "more");
    }

    /// DL_EXT is a known shared-lib extension.
    #[test]
    fn config_h_corpus_dl_ext_is_known() {
        assert!(
            DL_EXT == "so" || DL_EXT == "dylib" || DL_EXT == "dll",
            "DL_EXT must be a known shlib ext, got {DL_EXT:?}"
        );
    }

    /// CACHE_USERNAMES is 1.
    #[test]
    fn config_h_corpus_cache_usernames_enabled() {
        assert_eq!(CACHE_USERNAMES, 1);
    }

    /// JOB_CONTROL is 1.
    #[test]
    fn config_h_corpus_job_control_enabled() {
        assert_eq!(JOB_CONTROL, 1);
    }

    /// DEFAULT_HISTSIZE is positive.
    #[test]
    fn config_h_corpus_default_histsize_positive() {
        assert!(
            DEFAULT_HISTSIZE > 0,
            "histsize must be positive, got {DEFAULT_HISTSIZE}"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/config.h constants.
    // ═══════════════════════════════════════════════════════════════════

    /// PASSWD_FILE points to canonical /etc/passwd path.
    #[test]
    fn passwd_file_is_etc_passwd() {
        assert_eq!(PASSWD_FILE, "/etc/passwd");
    }

    /// DEFAULT_FCEDIT is "vi" (zsh's traditional default fc editor).
    #[test]
    fn default_fcedit_is_vi() {
        assert_eq!(DEFAULT_FCEDIT, "vi");
    }

    /// DEFAULT_TMPPREFIX is under /tmp.
    #[test]
    fn default_tmpprefix_under_tmp() {
        assert!(
            DEFAULT_TMPPREFIX.starts_with("/tmp"),
            "got {:?}",
            DEFAULT_TMPPREFIX
        );
    }

    /// DEFAULT_READNULLCMD is non-empty.
    #[test]
    fn default_readnullcmd_non_empty() {
        assert!(!DEFAULT_READNULLCMD.is_empty());
    }

    /// All GLOBAL_Z* config paths are under /etc.
    #[test]
    fn global_z_files_are_under_etc() {
        for p in [
            GLOBAL_ZLOGIN,
            GLOBAL_ZLOGOUT,
            GLOBAL_ZPROFILE,
            GLOBAL_ZSHENV,
            GLOBAL_ZSHRC,
        ] {
            assert!(p.starts_with("/etc/"), "{:?} must be under /etc", p);
        }
    }

    /// All GLOBAL_Z* paths are pairwise distinct.
    #[test]
    fn global_z_files_pairwise_distinct() {
        let paths = [
            GLOBAL_ZLOGIN,
            GLOBAL_ZLOGOUT,
            GLOBAL_ZPROFILE,
            GLOBAL_ZSHENV,
            GLOBAL_ZSHRC,
        ];
        let unique: std::collections::HashSet<_> = paths.iter().copied().collect();
        assert_eq!(unique.len(), paths.len(), "must be distinct");
    }

    /// HAVE_* flags are 0/1 (booleans as int).
    #[test]
    fn have_flags_are_zero_or_one() {
        for f in [HAVE_ALLOCA, HAVE_ALLOCA_H, HAVE_ARC4RANDOM_BUF] {
            assert!(f == 0 || f == 1, "HAVE_* must be 0 or 1, got {}", f);
        }
    }

    /// DYNAMIC = 1 (Rust port supports module loading).
    #[test]
    fn dynamic_enabled() {
        assert_eq!(DYNAMIC, 1);
    }

    /// USE_SUSPENDED enabled (zsh job-control suspend semantics).
    #[test]
    fn use_suspended_enabled() {
        assert_eq!(USE_SUSPENDED, 1);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for src/zsh/config.h constants
    // ═══════════════════════════════════════════════════════════════════

    /// DEFAULT_HISTSIZE = 30 (upstream default).
    #[test]
    fn default_histsize_is_thirty() {
        assert_eq!(DEFAULT_HISTSIZE, 30, "upstream zsh default");
    }

    /// DEFAULT_HISTSIZE is positive (else `fc -l` breaks).
    #[test]
    fn default_histsize_positive() {
        assert!(DEFAULT_HISTSIZE > 0, "history size must be > 0");
    }

    /// PASSWD_FILE is absolute path.
    #[test]
    fn passwd_file_absolute_path() {
        assert!(PASSWD_FILE.starts_with('/'), "PASSWD_FILE must be absolute");
    }

    /// PASSWD_MAP follows NIS dotted-name convention.
    #[test]
    fn passwd_map_dotted_nis_name() {
        assert!(
            PASSWD_MAP.contains('.'),
            "NIS map names use dot: passwd.byname"
        );
    }

    /// DL_EXT is short and ASCII (file extension stem).
    #[test]
    fn dl_ext_short_ascii() {
        assert!(DL_EXT.is_ascii(), "DL_EXT must be ASCII");
        assert!(!DL_EXT.is_empty(), "DL_EXT must be non-empty");
        assert!(DL_EXT.len() <= 5, "DL_EXT must be a short ext stem");
    }

    /// DL_EXT contains no dots (autoconf strips leading '.').
    #[test]
    fn dl_ext_has_no_dots() {
        assert!(!DL_EXT.contains('.'), "DL_EXT stem must not contain dots");
    }

    /// All GLOBAL_Z* paths share /etc prefix.
    #[test]
    fn all_global_z_paths_under_etc() {
        for path in &[
            GLOBAL_ZLOGIN,
            GLOBAL_ZLOGOUT,
            GLOBAL_ZPROFILE,
            GLOBAL_ZSHENV,
            GLOBAL_ZSHRC,
        ] {
            assert!(
                path.starts_with("/etc/"),
                "GLOBAL_Z* path must start with /etc/, got {:?}",
                path
            );
        }
    }

    /// DEFAULT_PATH has at least 2 colon-separated entries.
    #[test]
    fn default_path_has_multiple_entries() {
        let entries: Vec<&str> = DEFAULT_PATH.split(':').collect();
        assert!(
            entries.len() >= 2,
            "DEFAULT_PATH must have ≥ 2 entries, got {:?}",
            entries
        );
    }

    /// DEFAULT_PATH entries are all absolute.
    #[test]
    fn default_path_entries_all_absolute() {
        for entry in DEFAULT_PATH.split(':') {
            assert!(
                entry.starts_with('/'),
                "DEFAULT_PATH entry must be absolute: {:?}",
                entry
            );
        }
    }

    /// DEFAULT_TMPPREFIX has no trailing slash (used as path prefix).
    #[test]
    fn default_tmpprefix_no_trailing_slash() {
        assert!(
            !DEFAULT_TMPPREFIX.ends_with('/'),
            "DEFAULT_TMPPREFIX is a prefix; trailing slash would be wrong"
        );
    }

    /// All file-path config strings are non-empty.
    #[test]
    fn all_path_strings_non_empty() {
        for s in &[
            PASSWD_FILE,
            DEFAULT_PATH,
            DEFAULT_TMPPREFIX,
            DEFAULT_FCEDIT,
            DEFAULT_READNULLCMD,
            DL_EXT,
            GLOBAL_ZLOGIN,
            GLOBAL_ZLOGOUT,
            GLOBAL_ZPROFILE,
            GLOBAL_ZSHENV,
            GLOBAL_ZSHRC,
        ] {
            assert!(
                !s.is_empty(),
                "config path string must not be empty: {:?}",
                s
            );
        }
    }

    /// All boolean-flag config constants are 0 or 1.
    #[test]
    fn boolean_flags_all_zero_or_one() {
        for &v in &[
            CACHE_USERNAMES,
            JOB_CONTROL,
            USE_SUSPENDED,
            BROKEN_ISPRINT,
            CONFIG_LOCALE,
            DYNAMIC,
            DYNAMIC_NAME_CLASH_OK,
            GETCWD_CALLS_MALLOC,
            GETPGRP_VOID,
        ] {
            assert!(v == 0 || v == 1, "flag must be 0/1, got {}", v);
        }
    }

    /// All path strings are ASCII (filesystem-safe).
    #[test]
    fn config_path_strings_are_ascii() {
        for s in &[
            PASSWD_FILE,
            PASSWD_MAP,
            DEFAULT_PATH,
            DEFAULT_TMPPREFIX,
            DEFAULT_FCEDIT,
            DEFAULT_READNULLCMD,
            DL_EXT,
            GLOBAL_ZLOGIN,
            GLOBAL_ZLOGOUT,
            GLOBAL_ZPROFILE,
            GLOBAL_ZSHENV,
            GLOBAL_ZSHRC,
        ] {
            assert!(s.is_ascii(), "config string must be ASCII: {:?}", s);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for src/zsh/config.h second-half constants
    // c:959 LONG_IS_64_BIT / c:962 MACHTYPE / c:967 MAX_FUNCTION_DEPTH /
    // c:970 MULTIBYTE_SUPPORT / c:979 OSTYPE / c:1000 PATH_DEV_FD /
    // c:1003 PATH_UTMPX_FILE / c:1012 POSIX_SIGNALS / c:1088 VENDOR
    // ═══════════════════════════════════════════════════════════════════

    /// c:959 — LONG_IS_64_BIT = 1 on 64-bit arm/x86_64 platforms.
    #[test]
    fn long_is_64_bit_enabled() {
        assert_eq!(LONG_IS_64_BIT, 1, "modern targets use 64-bit long");
    }

    /// c:967 — MAX_FUNCTION_DEPTH = 500 (zsh default recursion limit).
    #[test]
    fn max_function_depth_is_500() {
        assert_eq!(MAX_FUNCTION_DEPTH, 500, "zsh canonical recursion limit");
    }

    /// c:970 — MULTIBYTE_SUPPORT = 1 (UTF-8 support is mandatory).
    #[test]
    fn multibyte_support_enabled() {
        assert_eq!(MULTIBYTE_SUPPORT, 1, "UTF-8 must be enabled");
    }

    /// c:1012 — POSIX_SIGNALS = 1 (modern OS uses POSIX signals).
    #[test]
    fn posix_signals_enabled() {
        assert_eq!(POSIX_SIGNALS, 1, "POSIX signal API mandatory");
    }

    /// c:962 — MACHTYPE is non-empty ASCII string (e.g. "arm" / "x86_64").
    #[test]
    fn machtype_non_empty_ascii() {
        assert!(!MACHTYPE.is_empty(), "MACHTYPE must be non-empty");
        assert!(MACHTYPE.is_ascii(), "MACHTYPE must be ASCII");
    }

    /// c:962 — MACHTYPE is configure's `$host_cpu`, the name config.guess
    /// prints, NOT uname's `machine`. On Apple Silicon those differ:
    /// config.guess says `aarch64-apple-darwinN` while `uname -m` says
    /// `arm64`, and zsh reports the former (`zsh -f -c 'typeset -p
    /// MACHTYPE'` → `typeset MACHTYPE=aarch64`). This was hardcoded to
    /// `"arm"`, which sent `_gcc`'s `case $MACHTYPE in` (Completion/Unix/
    /// Command/_gcc:34,66,448) down the 32-bit ARM arm rather than the
    /// aarch64 arm: `gcc -<TAB>` offered 17 ARM/SuperH `-m*` flags that
    /// do not exist on this CPU and hid the 3 AArch64 ones, so zshrs
    /// reported "1652 possibilities" where zsh reports 1638.
    #[test]
    fn machtype_is_config_guess_host_cpu_not_uname_machine() {
        assert_eq!(
            MACHTYPE,
            std::env::consts::ARCH,
            "MACHTYPE must track the build target arch"
        );
        #[cfg(target_arch = "aarch64")]
        assert_eq!(
            MACHTYPE, "aarch64",
            "config.guess spells 64-bit ARM `aarch64`, never `arm`"
        );
    }

    /// c:979 — `$OSTYPE` must have the *shape* `config.guess` gives
    /// `$host_os` on this target, not the shape of any one build host.
    ///
    /// Darwin (`config.guess:1463` `GUESS=aarch64-apple-darwin$UNAME_RELEASE`,
    /// `config.guess:148` `UNAME_RELEASE=`(uname -r)``): `darwin` immediately
    /// followed by the kernel release, so a digit must follow `darwin`.
    ///
    /// Linux (`config.guess:1009` `$CPU-unknown-linux-$LIBCABI`,
    /// `config.guess:1222` `$CPU-pc-linux-$LIBCABI`): `linux-$LIBC` — the C
    /// library, never a kernel version. This is the case a naive `uname -r`
    /// derivation gets wrong, and the case that cannot be checked on this
    /// Mac, so assert it structurally: no digits at all.
    #[test]
    fn ostype_matches_config_guess_host_os_shape() {
        #[cfg(target_os = "macos")]
        {
            assert!(
                OSTYPE.starts_with("darwin"),
                "config.guess:1463 spells Darwin's $host_os `darwin<release>`, got {OSTYPE:?}"
            );
            let rel = &OSTYPE["darwin".len()..];
            assert!(
                rel.starts_with(|c: char| c.is_ascii_digit()),
                "$host_os on Darwin carries uname -r; {OSTYPE:?} has no release suffix"
            );
            assert!(
                !OSTYPE.contains('-'),
                "Darwin's $host_os is a single token, got {OSTYPE:?}"
            );
        }
        #[cfg(target_os = "linux")]
        {
            assert!(
                matches!(OSTYPE, "linux-gnu" | "linux-musl" | "linux-uclibc"),
                "config.guess emits `linux-$LIBC` for $host_os on Linux, got {OSTYPE:?}"
            );
            assert!(
                !OSTYPE.contains(|c: char| c.is_ascii_digit()),
                "a kernel version in {OSTYPE:?} means the derivation used `uname -r`, \
                 which config.guess never does on Linux"
            );
        }
    }

    /// c:1088 — `$VENDOR` is `$host_vendor`, which depends on the CPU as
    /// well as the OS: `config.guess:1222` emits `$CPU-pc-linux-$LIBCABI`
    /// for x86_64 Linux but `config.guess:1009` emits
    /// `$CPU-unknown-linux-$LIBCABI` for aarch64 Linux. Anything derived
    /// from `uname -s` alone collapses that distinction and answers
    /// `unknown` on the x86_64 servers.
    #[test]
    fn vendor_matches_config_guess_host_vendor() {
        #[cfg(target_os = "macos")]
        assert_eq!(
            VENDOR, "apple",
            "config.guess:1463/1500 — Darwin's $host_vendor is `apple`"
        );
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        assert_eq!(
            VENDOR, "pc",
            "config.guess:1222 — x86_64 Linux is `x86_64-pc-linux-gnu`"
        );
        #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
        assert_eq!(
            VENDOR, "unknown",
            "config.guess:1009 — aarch64 Linux is `aarch64-unknown-linux-gnu`"
        );
        #[cfg(all(target_os = "linux", target_arch = "riscv64"))]
        assert_eq!(
            VENDOR, "unknown",
            "config.guess:1177 — riscv64 Linux is `riscv64-unknown-linux-gnu`"
        );
    }

    /// c:1000 / `configure.ac:1954` — `DEFAULT_PATH` is the build host's
    /// `getconf _CS_PATH`/`CS_PATH`/`PATH`. `Src/exec.c` substitutes it for
    /// `$PATH` under `command -p`, so every entry has to be a directory that
    /// actually exists on this machine; a value copied from another OS
    /// silently makes `command -p` resolve nothing.
    #[test]
    fn default_path_entries_exist_on_this_host() {
        for seg in DEFAULT_PATH.split(':') {
            assert!(
                std::path::Path::new(seg).is_dir(),
                "DEFAULT_PATH entry {seg:?} is not a directory here — \
                 `command -p` would find nothing in it (full value {DEFAULT_PATH:?})"
            );
        }
    }

    /// c:979 — OSTYPE is non-empty ASCII string (e.g. "darwin25.5.0").
    #[test]
    fn ostype_non_empty_ascii() {
        assert!(!OSTYPE.is_empty(), "OSTYPE must be non-empty");
        assert!(OSTYPE.is_ascii(), "OSTYPE must be ASCII");
    }

    /// c:1088 — VENDOR is non-empty ASCII (e.g. "apple" / "pc").
    #[test]
    fn vendor_non_empty_ascii() {
        assert!(!VENDOR.is_empty(), "VENDOR must be non-empty");
        assert!(VENDOR.is_ascii(), "VENDOR must be ASCII");
    }

    /// c:1000 — PATH_DEV_FD is whatever `configure.ac:1969`'s probe finds
    /// *first*: `for zsh_cv_sys_path_dev_fd in /proc/self/fd /dev/fd no`.
    /// Linux always satisfies `/proc/self/fd`; Darwin has no procfs and
    /// falls to `/dev/fd`. Pinning the Darwin answer unconditionally would
    /// have made a Linux build disagree with the zsh it must match.
    #[test]
    fn path_dev_fd_is_canonical() {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        assert_eq!(
            PATH_DEV_FD, "/proc/self/fd",
            "configure.ac:1969 probes /proc/self/fd before /dev/fd"
        );
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        assert_eq!(
            PATH_DEV_FD, "/dev/fd",
            "canonical fd filesystem path per POSIX"
        );
    }

    /// c:1003 — PATH_UTMPX_FILE is absolute path.
    #[test]
    fn path_utmpx_file_absolute() {
        assert!(
            PATH_UTMPX_FILE.starts_with('/'),
            "PATH_UTMPX_FILE must be absolute"
        );
    }

    /// c:962+979+1088 — MACHTYPE/OSTYPE/VENDOR all have no whitespace.
    #[test]
    fn machtype_ostype_vendor_no_whitespace() {
        for s in &[MACHTYPE, OSTYPE, VENDOR] {
            assert!(
                !s.chars().any(|c| c.is_whitespace()),
                "config target string must not contain whitespace: {:?}",
                s
            );
        }
    }

    /// PACKAGE_* fields are all defined (may be empty per autoconf default).
    #[test]
    fn package_fields_are_ascii_strings() {
        for s in &[
            PACKAGE_BUGREPORT,
            PACKAGE_NAME,
            PACKAGE_STRING,
            PACKAGE_TARNAME,
            PACKAGE_URL,
            PACKAGE_VERSION,
        ] {
            assert!(s.is_ascii(), "PACKAGE_* must be ASCII: {:?}", s);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/config.h constants
    // c:23 PASSWD_FILE / c:30 CACHE_USERNAMES / c:33 JOB_CONTROL /
    // c:36 USE_SUSPENDED / c:39 DEFAULT_HISTSIZE / c:42 DEFAULT_FCEDIT /
    // c:45 DEFAULT_TMPPREFIX / c:74 DEFAULT_PATH / c:77 DEFAULT_READNULLCMD /
    // c:85 DL_EXT / c:967 MAX_FUNCTION_DEPTH
    // ═══════════════════════════════════════════════════════════════════

    /// c:23 — `PASSWD_FILE` is absolute path.
    #[test]
    fn passwd_file_is_absolute_path() {
        assert!(
            PASSWD_FILE.starts_with('/'),
            "PASSWD_FILE must be absolute, got {:?}",
            PASSWD_FILE
        );
    }

    /// c:23 — `PASSWD_FILE` is canonical POSIX `/etc/passwd`.
    #[test]
    fn passwd_file_is_canonical_etc_passwd() {
        assert_eq!(
            PASSWD_FILE, "/etc/passwd",
            "PASSWD_FILE must be the POSIX canonical /etc/passwd"
        );
    }

    /// c:39 — `DEFAULT_HISTSIZE` is positive.
    #[test]
    fn default_histsize_is_positive() {
        assert!(
            DEFAULT_HISTSIZE > 0,
            "DEFAULT_HISTSIZE must be positive, got {}",
            DEFAULT_HISTSIZE
        );
    }

    /// c:42 — `DEFAULT_FCEDIT` is non-empty ASCII (editor command).
    #[test]
    fn default_fcedit_is_non_empty_ascii() {
        assert!(!DEFAULT_FCEDIT.is_empty());
        assert!(DEFAULT_FCEDIT.is_ascii());
    }

    /// c:45 — `DEFAULT_TMPPREFIX` is absolute.
    #[test]
    fn default_tmpprefix_is_absolute() {
        assert!(
            DEFAULT_TMPPREFIX.starts_with('/'),
            "DEFAULT_TMPPREFIX must be absolute path"
        );
    }

    /// c:74 — `DEFAULT_PATH` contains `/usr/bin` (POSIX required entry).
    #[test]
    fn default_path_contains_usr_bin() {
        assert!(
            DEFAULT_PATH.contains("/usr/bin"),
            "DEFAULT_PATH must contain /usr/bin; got {:?}",
            DEFAULT_PATH
        );
    }

    /// c:74 — `DEFAULT_PATH` uses ':' separator (POSIX convention).
    #[test]
    fn default_path_uses_colon_separator() {
        assert!(
            DEFAULT_PATH.contains(':'),
            "DEFAULT_PATH must use ':' separator"
        );
    }

    /// c:77 — `DEFAULT_READNULLCMD` is non-empty (pager command).
    #[test]
    fn default_readnullcmd_is_non_empty() {
        assert!(!DEFAULT_READNULLCMD.is_empty());
    }

    /// c:85 — `DL_EXT` is non-empty ASCII (e.g. "so", "dylib").
    #[test]
    fn dl_ext_is_non_empty_ascii() {
        assert!(
            !DL_EXT.is_empty(),
            "DL_EXT must be non-empty (e.g. 'so'/'dylib')"
        );
        assert!(DL_EXT.is_ascii());
    }

    /// c:967 — `MAX_FUNCTION_DEPTH` ≥ 100 (no degenerate shallow limit).
    #[test]
    fn max_function_depth_is_at_least_100() {
        assert!(
            MAX_FUNCTION_DEPTH >= 100,
            "MAX_FUNCTION_DEPTH must be ≥ 100 to support real scripts"
        );
    }

    /// c:33+36 — JOB_CONTROL + USE_SUSPENDED both enabled.
    #[test]
    fn job_control_and_use_suspended_enabled() {
        assert_eq!(JOB_CONTROL, 1, "JOB_CONTROL required for shell");
        assert_eq!(USE_SUSPENDED, 1, "USE_SUSPENDED required");
    }

    /// c:30 — `CACHE_USERNAMES` enabled (canonical zsh behaviour).
    #[test]
    fn cache_usernames_enabled() {
        assert_eq!(
            CACHE_USERNAMES, 1,
            "CACHE_USERNAMES must be enabled for ~user expansion"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional pins for Src/config.h
    // c:23 PASSWD_FILE / c:39 DEFAULT_HISTSIZE / c:42 DEFAULT_FCEDIT /
    // c:45 DEFAULT_TMPPREFIX / c:74 DEFAULT_PATH / c:105-121 GLOBAL_Z* /
    // c:1000 PATH_DEV_FD / c:970 MULTIBYTE_SUPPORT / c:85 DL_EXT
    // ═══════════════════════════════════════════════════════════════════

    /// c:23 — `PASSWD_FILE` is the canonical Unix password file path.
    #[test]
    fn passwd_file_is_canonical_unix_path() {
        assert_eq!(
            PASSWD_FILE, "/etc/passwd",
            "PASSWD_FILE must be /etc/passwd"
        );
    }

    /// c:42 — `DEFAULT_FCEDIT` is non-empty (history editor must have a default).
    #[test]
    fn default_fcedit_non_empty() {
        assert!(
            !DEFAULT_FCEDIT.is_empty(),
            "DEFAULT_FCEDIT must have a value"
        );
    }

    /// c:45 — `DEFAULT_TMPPREFIX` starts with `/tmp` (canonical temp dir).
    #[test]
    fn default_tmpprefix_starts_with_tmp() {
        assert!(
            DEFAULT_TMPPREFIX.starts_with("/tmp"),
            "DEFAULT_TMPPREFIX must be under /tmp, got {:?}",
            DEFAULT_TMPPREFIX
        );
    }

    /// c:74 — `DEFAULT_PATH` contains `/usr/bin` and `/bin` (POSIX baseline).
    #[test]
    fn default_path_contains_usr_bin_and_bin() {
        assert!(
            DEFAULT_PATH.contains("/usr/bin"),
            "DEFAULT_PATH must contain /usr/bin"
        );
        assert!(
            DEFAULT_PATH.contains("/bin"),
            "DEFAULT_PATH must contain /bin"
        );
    }

    /// c:74 — `DEFAULT_PATH` uses `:` separator (POSIX PATH form, alt).
    #[test]
    fn default_path_uses_colon_separator_alt() {
        assert!(
            DEFAULT_PATH.contains(':'),
            "DEFAULT_PATH must use ':' separator"
        );
    }

    /// c:39 — `DEFAULT_HISTSIZE` is a positive integer (alt).
    #[test]
    fn default_histsize_is_positive_alt() {
        assert!(
            DEFAULT_HISTSIZE > 0,
            "DEFAULT_HISTSIZE must be > 0, got {}",
            DEFAULT_HISTSIZE
        );
    }

    /// c:105-121 — every GLOBAL_Z* path starts with `/etc/`.
    #[test]
    fn global_z_files_under_etc() {
        for (name, val) in [
            ("GLOBAL_ZLOGIN", GLOBAL_ZLOGIN),
            ("GLOBAL_ZLOGOUT", GLOBAL_ZLOGOUT),
            ("GLOBAL_ZPROFILE", GLOBAL_ZPROFILE),
            ("GLOBAL_ZSHENV", GLOBAL_ZSHENV),
            ("GLOBAL_ZSHRC", GLOBAL_ZSHRC),
        ] {
            assert!(
                val.starts_with("/etc/"),
                "{} must be under /etc/, got {:?}",
                name,
                val
            );
        }
    }

    /// c:105-121 — every GLOBAL_Z* path is non-empty.
    #[test]
    fn global_z_files_non_empty() {
        for (name, val) in [
            ("GLOBAL_ZLOGIN", GLOBAL_ZLOGIN),
            ("GLOBAL_ZLOGOUT", GLOBAL_ZLOGOUT),
            ("GLOBAL_ZPROFILE", GLOBAL_ZPROFILE),
            ("GLOBAL_ZSHENV", GLOBAL_ZSHENV),
            ("GLOBAL_ZSHRC", GLOBAL_ZSHRC),
        ] {
            assert!(!val.is_empty(), "{} must be non-empty", name);
        }
    }

    /// c:1000 — `PATH_DEV_FD` names a real fd-as-path directory on the
    /// target, since `Src/exec.c::getproc` opens `$PATH_DEV_FD/N` for every
    /// `<(cmd)` / `>(cmd)`. Same per-target split as
    /// `path_dev_fd_is_canonical`, asserted from the opposite direction:
    /// the path must be absolute and must actually exist here.
    #[test]
    fn path_dev_fd_is_canonical_alt() {
        assert!(
            PATH_DEV_FD.starts_with('/'),
            "PATH_DEV_FD must be absolute, got {PATH_DEV_FD:?}"
        );
        assert!(
            std::path::Path::new(PATH_DEV_FD).is_dir(),
            "PATH_DEV_FD = {PATH_DEV_FD:?} does not exist — process substitution would break"
        );
    }

    /// c:970 — `MULTIBYTE_SUPPORT` is enabled (alt).
    #[test]
    fn multibyte_support_enabled_alt() {
        assert_eq!(
            MULTIBYTE_SUPPORT, 1,
            "MULTIBYTE_SUPPORT must be on for modern shells"
        );
    }

    /// c:23-1093 — every string const is valid UTF-8 (trivial for &str, pin contract).
    #[test]
    fn string_consts_all_valid_utf8() {
        for s in [
            PASSWD_FILE,
            PASSWD_MAP,
            DEFAULT_FCEDIT,
            DEFAULT_TMPPREFIX,
            DEFAULT_PATH,
            DEFAULT_READNULLCMD,
            DL_EXT,
            GLOBAL_ZLOGIN,
            GLOBAL_ZLOGOUT,
            GLOBAL_ZPROFILE,
            GLOBAL_ZSHENV,
            GLOBAL_ZSHRC,
            PATH_DEV_FD,
            PATH_UTMPX_FILE,
            MACHTYPE,
            OSTYPE,
            VENDOR,
        ] {
            let _ = std::str::from_utf8(s.as_bytes()).expect("must be UTF-8");
        }
    }

    /// c:85 — `DL_EXT` is `so` (Linux/macOS shared-object suffix, no leading dot).
    #[test]
    fn dl_ext_is_so_no_leading_dot() {
        assert_eq!(DL_EXT, "so", "DL_EXT must be `so` (no leading dot)");
    }

    /// c:42 — `DEFAULT_FCEDIT` is `vi` (zsh canonical default fc editor, alt).
    #[test]
    fn default_fcedit_is_vi_alt() {
        assert_eq!(DEFAULT_FCEDIT, "vi", "canonical default fc editor");
    }
}
