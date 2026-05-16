//! Direct port of `src/zsh/config.h` (1276 lines, 271 #defines,
//! 135 #undefs at autoconf-generation time on darwin23.6.0 arm).
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
pub const DEFAULT_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

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
pub const MACHTYPE: &str = "arm";

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
pub const OSTYPE: &str = "darwin23.6.0";

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
pub const PATH_DEV_FD: &str = "/dev/fd";

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
pub const VENDOR: &str = "apple";

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
        assert_eq!(JOB_CONTROL,   1);
        assert_eq!(USE_SUSPENDED, 1);
    }

    /// PASSWD_FILE is consulted by init.c when populating $USERNAME /
    /// $HOME from /etc/passwd. Hard-pins the POSIX-standard location.
    #[test]
    fn passwd_file_is_posix_standard_location() {
        assert_eq!(PASSWD_FILE, "/etc/passwd");
    }

    /// `configure.ac:2978-2984` + `config.h:13-19` — canonical zsh
    /// config defaults. Pin all three (HISTSIZE, FCEDIT, TMPPREFIX)
    /// against the upstream `config.h` values; drift on any silently
    /// changes user-facing behavior on a first-run shell.
    #[test]
    fn default_config_values_match_upstream_config_h() {
        // config.h:13 — DEFAULT_HISTSIZE 30.
        assert_eq!(DEFAULT_HISTSIZE, 30,
            "configure.ac:2978 / config.h:13 — DEFAULT_HISTSIZE = 30");
        // config.h:16 — DEFAULT_FCEDIT "vi".
        assert_eq!(DEFAULT_FCEDIT, "vi",
            "configure.ac:2981 / config.h:16 — DEFAULT_FCEDIT = \"vi\"");
        // config.h:19 — DEFAULT_TMPPREFIX "/tmp/zsh".
        assert_eq!(DEFAULT_TMPPREFIX, "/tmp/zsh",
            "configure.ac:2984 / config.h:19 — DEFAULT_TMPPREFIX = \"/tmp/zsh\"");
    }
}
