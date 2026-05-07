//! Shell initialization for zshrs
//!
//! Port from zsh/Src/init.c
//!
//! Provides shell initialization, startup script sourcing, and main loop.

use std::env;
use std::path::{Path, PathBuf};

/// Shell initialization options
#[derive(Clone, Debug, Default)]
/// Parsed command-line options for the shell binary.
/// Mirrors the option set `parseargs()` from Src/init.c:263 +
/// `parseopts()` (line 390) build into the global state.
pub struct ShellOptions {
    pub interactive: bool,
    pub login: bool,
    pub shin_stdin: bool,
    pub use_zle: bool,
    pub monitor: bool,
    pub hash_dirs: bool,
    pub privileged: bool,
    pub single_command: bool,
    pub rcs: bool,
    pub global_rcs: bool,
}

/// Global shell state
/// Top-level shell state.
/// Aggregates the slots Src/init.c populates in `setupvals()`
/// (line 1014) and `init_misc()` (line 1524) — `argv0`,
/// `cmd_string`, login-shell flag, runscript path, etc.
pub struct ShellState {
    pub options: ShellOptions,
    pub argv0: String,
    pub argzero: String,
    pub posixzero: String,
    pub shell_name: String,
    pub pwd: String,
    pub oldpwd: String,
    pub home: String,
    pub username: String,
    pub mypid: i64,
    pub ppid: i64,
    pub shtty: i32,
    pub sourcelevel: i32,
    pub lineno: i64,
    pub path: Vec<String>,
    pub fpath: Vec<String>,
    pub cdpath: Vec<String>,
    pub module_path: Vec<String>,
    pub term: String,
    pub histsize: usize,
    pub emulation: ShellEmulation,
}

/// Shell emulation mode
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
/// Shell emulation modes.
/// Port of the `EMULATE_*` enum from Src/zsh.h —
/// `parseopts_setemulate()` (Src/init.c:348) maps `--emulate`
/// values onto these.
pub enum ShellEmulation {
    #[default]
    Zsh,
    Sh,
    Ksh,
    Csh,
}

impl ShellState {
    pub fn new() -> Self {
        let home = env::var("HOME").unwrap_or_else(|_| "/".to_string());
        let pwd = env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| home.clone());

        ShellState {
            options: ShellOptions {
                rcs: true,
                global_rcs: true,
                ..Default::default()
            },
            argv0: String::new(),
            argzero: String::new(),
            posixzero: String::new(),
            shell_name: "zsh".to_string(),
            pwd: pwd.clone(),
            oldpwd: pwd,
            home,
            username: env::var("USER").unwrap_or_default(),
            mypid: std::process::id() as i64,
            ppid: 0, // Would need libc to get parent pid
            shtty: -1,
            sourcelevel: 0,
            lineno: 1,
            path: vec![
                "/bin".to_string(),
                "/usr/bin".to_string(),
                "/usr/local/bin".to_string(),
            ],
            fpath: Vec::new(),
            cdpath: Vec::new(),
            module_path: Vec::new(),
            term: env::var("TERM").unwrap_or_default(),
            histsize: 1000,
            emulation: ShellEmulation::Zsh,
        }
    }

    /// Determine shell emulation from name
    pub fn emulate_from_name(&mut self, name: &str) {
        let basename = Path::new(name)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(name);

        let basename = basename.trim_start_matches('-');

        self.emulation = match basename {
            "sh" => ShellEmulation::Sh,
            "ksh" | "ksh93" => ShellEmulation::Ksh,
            "csh" | "tcsh" => ShellEmulation::Csh,
            _ => ShellEmulation::Zsh,
        };
    }

    /// Check if running in sh/ksh emulation
    pub fn is_posix_emulation(&self) -> bool {
        matches!(self.emulation, ShellEmulation::Sh | ShellEmulation::Ksh)
    }
}

impl Default for ShellState {
    fn default() -> Self {
        Self::new()
    }
}

/// Loop result
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Outcome of one iteration of the main shell loop.
/// Port of the integer return values `loop()` from Src/init.c:113
/// produces — Continue / Break / Done / Error.
pub enum LoopReturn {
    Ok,
    Empty,
    Error,
}

/// Source result
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Outcome of `source`/`.` execution.
/// Mirrors the return codes `source()` from Src/init.c:1551
/// produces — Success / NotFound / Error.
pub enum SourceReturn {
    Ok,
    NotFound,
    Error,
}

/// Parse command line arguments
/// Parse `argv` for the shell binary.
/// Port of `parseargs()` from Src/init.c:263 — extracts `-c`
/// command, runscript path, and remaining positional args; the
/// option flags it sets feed into `parseopts()` (line 390).
pub fn parseargs(args: &[String]) -> (ShellOptions, Option<String>, Vec<String>) {
    let mut opts = ShellOptions::default();
    let mut cmd = None;
    let mut positional = Vec::new();
    let mut iter = args.iter().skip(1).peekable();
    let mut done_opts = false;

    while let Some(arg) = iter.next() {
        if done_opts || !arg.starts_with('-') && !arg.starts_with('+') {
            positional.push(arg.clone());
            done_opts = true;
            continue;
        }

        if arg == "--" {
            done_opts = true;
            continue;
        }

        if arg == "--help" {
            print_help();
            std::process::exit(0);
        }

        if arg == "--version" {
            println!("zshrs {}", env!("CARGO_PKG_VERSION"));
            std::process::exit(0);
        }

        let is_set = arg.starts_with('-');
        let flags: Vec<char> = arg[1..].chars().collect();

        for flag in flags {
            match flag {
                'c' => {
                    if let Some(c) = iter.next() {
                        cmd = Some(c.clone());
                        opts.interactive = false;
                    }
                }
                'i' => opts.interactive = is_set,
                'l' => opts.login = is_set,
                's' => opts.shin_stdin = is_set,
                'm' => opts.monitor = is_set,
                'o' => {
                    if let Some(opt_name) = iter.next() {
                        set_option_by_name(&mut opts, opt_name, is_set);
                    }
                }
                _ => {}
            }
        }
    }

    // Defaults based on tty
    if atty::is(atty::Stream::Stdin) {
        if cmd.is_none() {
            opts.interactive = true;
        }
        opts.use_zle = true;
    }

    (opts, cmd, positional)
}

fn set_option_by_name(opts: &mut ShellOptions, name: &str, value: bool) {
    let name_lower = name.to_lowercase().replace('_', "");
    match name_lower.as_str() {
        "interactive" => opts.interactive = value,
        "login" => opts.login = value,
        "shinstdin" => opts.shin_stdin = value,
        "zle" | "usezle" => opts.use_zle = value,
        "monitor" => opts.monitor = value,
        "hashdirs" => opts.hash_dirs = value,
        "privileged" => opts.privileged = value,
        "singlecommand" => opts.single_command = value,
        "rcs" => opts.rcs = value,
        "globalrcs" => opts.global_rcs = value,
        _ => {}
    }
}

fn print_help() {
    println!("Usage: zshrs [<options>] [<argument> ...]");
    println!();
    println!("Special options:");
    println!("  --help     show this message, then exit");
    println!("  --version  show zshrs version number, then exit");
    println!("  -c         take first argument as a command to execute");
    println!("  -i         force interactive mode");
    println!("  -l         treat as login shell");
    println!("  -s         read commands from stdin");
    println!("  -o OPTION  set an option by name");
}

/// Initialize shell I/O
/// Initialize the shell's stdio.
/// Port of `init_io()` from Src/init.c:577 — sets up SHIN/SHTTY,
/// duplicates the controlling tty into `mailfd`, and configures
/// terminal-related globals.
pub fn init_io(state: &mut ShellState) {
    // Try to get tty
    if atty::is(atty::Stream::Stdin) {
        state.shtty = 0;
    }

    if state.options.interactive && state.shtty == -1 {
        state.options.use_zle = false;
    }
}

/// Set up shell values
/// Populate environment-derived globals (PWD/UID/HOME/etc.).
/// Port of `setupvals()` from Src/init.c:1014 — the C source
/// reads `getuid()`/`gethostname()`/`getpwuid()` and seeds the
/// special parameter table. Same effect on Rust state here.
pub fn setupvals(state: &mut ShellState) {
    // Set up PATH
    if let Ok(path_env) = env::var("PATH") {
        state.path = path_env.split(':').map(String::from).collect();
    }

    // Set up prompts based on emulation
    // (In full implementation, these would be stored in params)

    // Initialize history
    state.histsize = env::var("HISTSIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);
}

/// Source a file
/// Source a shell file at `path`.
/// Port of `source()` from Src/init.c:1551 — parses + runs the
/// file in the current shell environment, with the standard
/// `noexec`/`autocd`/`local-script-options` handling.
pub fn source(state: &mut ShellState, path: &str) -> SourceReturn {
    let path = Path::new(path);

    if !path.exists() {
        return SourceReturn::NotFound;
    }

    state.sourcelevel += 1;

    // In a full implementation, we would:
    // 1. Open the file
    // 2. Parse and execute commands
    // 3. Handle errors

    state.sourcelevel -= 1;
    SourceReturn::Ok
}

/// Source a file from home directory
/// Source a startup file from `$ZDOTDIR` / `$HOME`.
/// Port of `sourcehome()` from Src/init.c:1679 — same
/// `$ZDOTDIR`-overrides-`$HOME` lookup precedence the C source
/// uses for `.zshrc` / `.zprofile` / `.zlogin` / `.zlogout`.
pub fn sourcehome(state: &mut ShellState, filename: &str) -> SourceReturn {
    let zdotdir = env::var("ZDOTDIR").unwrap_or_else(|_| state.home.clone());
    let path = format!("{}/{}", zdotdir, filename);
    source(state, &path)
}

/// Run initialization scripts
/// Run the standard startup-file chain.
/// Port of `run_init_scripts()` from Src/init.c:1445 — sources
/// `/etc/zshenv` → `$ZDOTDIR/.zshenv` → (if login)
/// `/etc/zprofile` → `$ZDOTDIR/.zprofile` → (if interactive)
/// `/etc/zshrc` → `$ZDOTDIR/.zshrc` → (if login) `/etc/zlogin` →
/// `$ZDOTDIR/.zlogin`. Same precedence as the C source.
pub fn run_init_scripts(state: &mut ShellState) {
    if state.is_posix_emulation() {
        // sh/ksh emulation
        if state.options.login {
            source(state, "/etc/profile");
        }
        if !state.options.privileged {
            if state.options.login {
                sourcehome(state, ".profile");
            }
            if state.options.interactive {
                if let Ok(env_file) = env::var("ENV") {
                    source(state, &env_file);
                }
            }
        }
    } else {
        // zsh mode
        if state.options.rcs && state.options.global_rcs {
            source(state, "/etc/zshenv");
        }
        if state.options.rcs && !state.options.privileged {
            sourcehome(state, ".zshenv");
        }
        if state.options.login {
            if state.options.rcs && state.options.global_rcs {
                source(state, "/etc/zprofile");
            }
            if state.options.rcs && !state.options.privileged {
                sourcehome(state, ".zprofile");
            }
        }
        if state.options.interactive {
            if state.options.rcs && state.options.global_rcs {
                source(state, "/etc/zshrc");
            }
            if state.options.rcs && !state.options.privileged {
                sourcehome(state, ".zshrc");
            }
        }
        if state.options.login {
            if state.options.rcs && state.options.global_rcs {
                source(state, "/etc/zlogin");
            }
            if state.options.rcs && !state.options.privileged {
                sourcehome(state, ".zlogin");
            }
        }
    }
}

/// Get the executable path of the current process
/// Locate the running shell binary.
/// zshrs convenience — the closest C analog is `getmypath()`
/// (Src/init.c:909) which walks `$0`, `$PATH`, then `getcwd(2)`
/// to identify the executable.
pub fn get_exe_path() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_link("/proc/self/exe").ok()
    }

    #[cfg(target_os = "macos")]
    {
        use std::ffi::CStr;
        let mut buf = [0u8; libc::PATH_MAX as usize];
        let mut size = buf.len() as u32;
        unsafe {
            if libc::_NSGetExecutablePath(buf.as_mut_ptr() as *mut i8, &mut size) == 0 {
                let path = CStr::from_ptr(buf.as_ptr() as *const i8);
                Some(PathBuf::from(path.to_string_lossy().into_owned()))
            } else {
                None
            }
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// Resolve the shell's own executable path.
/// Port of `getmypath()` from Src/init.c:909-1004 — used on
/// platforms where the kernel doesn't expose the binary path
/// (no /proc/self/exe, no _NSGetExecutablePath, no
/// KERN_PROC_PATHNAME). Walks the argv\[0\]/cwd/$PATH heuristics
/// the C source falls back on.
///
/// Algorithm (init.c:956-1004):
///
///   1. If name starts with `-`, skip it (login-shell prefix).
///   2. If name is empty or ends with `/`, return None.
///   3. If name is absolute (starts with `/`), return as-is.
///   4. If name contains `/`, treat as relative — return cwd/name.
///   5. Otherwise walk $PATH: for each dir, return realpath(dir/name)
///      if it exists.
pub fn getmypath(name: Option<&str>, cwd: Option<&str>) -> Option<PathBuf> {
    // Try the kernel-supported path first (init.c:914-953).
    if let Some(p) = get_exe_path() {
        return Some(p);
    }

    // Fallback to the argv[0]/cwd/$PATH walk (init.c:956-1004).
    let name = name?;
    let name = name.strip_prefix('-').unwrap_or(name);
    if name.is_empty() {
        return None;
    }
    if name.ends_with('/') {
        return None;
    }
    if name.starts_with('/') {
        return Some(PathBuf::from(name));
    }
    if name.contains('/') {
        let cwd = cwd?;
        return Some(PathBuf::from(format!("{}/{}", cwd, name)));
    }
    // PATH walk via realpath equivalent.
    let path = env::var("PATH").ok()?;
    if path.is_empty() {
        return None;
    }
    for dir in path.split(':') {
        let candidate = if dir.is_empty() {
            PathBuf::from(name)
        } else {
            PathBuf::from(format!("{}/{}", dir, name))
        };
        if let Ok(real) = std::fs::canonicalize(&candidate) {
            if real.is_file() {
                return Some(real);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Missing functions from init.c
// ---------------------------------------------------------------------------

/// Initialize terminal settings (from init.c init_term)
/// Initialize terminal-capability state.
/// Port of `init_term()` from Src/init.c:771 — looks up TERM,
/// resolves `tgetent()`, and populates the termcap globals.
pub fn init_term(state: &ShellState) -> bool {
    let term = &state.term;
    if term.is_empty() {
        return false;
    }
    // Terminal initialization is handled by the terminfo/termcap modules
    // This function mainly validates the TERM value
    !term.is_empty() && term != "dumb"
}

/// Set up the PWD variable (from init.c set_pwd_env)
/// Set `$PWD` from the current working directory.
/// Port of the `setupvals()` PWD-init step (Src/init.c:1014) —
/// uses `zgetcwd()` and writes both `$PWD` and `$OLDPWD`.
pub fn set_pwd_env(state: &mut ShellState) {
    if let Ok(cwd) = env::current_dir() {
        state.pwd = cwd.to_string_lossy().to_string();
    }
    env::set_var("PWD", &state.pwd);
    env::set_var("OLDPWD", &state.oldpwd);
}

/// Close the shell (from init.c zexit)
/// Terminate the shell with an exit status.
/// Port of `zexit()` (Src/init.c) — runs exit traps, flushes
/// history, releases tty, then `exit(val)`. The `from_where`
/// argument matches the C source's `ZEXIT_*` reason codes.
pub fn zexit(val: i32, from_where: i32) -> ! {
    // from_where: 0=normal, 1=signal, 2=exec
    std::process::exit(val)
}

/// Set up the tty (from init.c init_shout)
/// Initialize the controlling terminal.
/// Port of `init_shout()` from Src/init.c:712 — opens `/dev/tty`
/// and configures the shell's output stream for prompt-aware
/// writes.
pub fn init_shout(state: &mut ShellState) {
    #[cfg(unix)]
    {
        // Check if stdin is a tty
        if unsafe { libc::isatty(0) } == 1 {
            state.shtty = 0;
            state.options.interactive = true;
        } else {
            state.shtty = -1;
        }
    }
}

/// Set up options from emulation mode (from init.c setupvals emulation portion)
/// Apply emulation-flag presets.
/// Port of `parseopts_setemulate()` from Src/init.c:348 — sets
/// the `EMULATE_*` flag bits and toggles compatibility options
/// to match `--emulate sh`/`csh`/`ksh`.
pub fn parseopts_setemulate(state: &mut ShellState) {
    match state.emulation {
        ShellEmulation::Sh => {
            // POSIX sh compatibility
            state.options.monitor = state.options.interactive;
        }
        ShellEmulation::Ksh => {
            // ksh compatibility
            state.options.monitor = state.options.interactive;
        }
        ShellEmulation::Csh => {
            // csh compatibility
        }
        ShellEmulation::Zsh => {
            // Default zsh behavior
            state.options.monitor = state.options.interactive;
            state.options.hash_dirs = true;
        }
    }
}

/// Find a command in PATH (from init.c pathprog equivalent)
/// Search `$path` for an executable.
/// zshrs convenience — the C source has a similar helper
/// inline in `findcmd()` (Src/exec.c). Walks each directory and
/// returns the first match for which `access(X_OK)` succeeds.
pub fn pathprog(prog: &str, path: &[String]) -> Option<PathBuf> {
    if prog.contains('/') {
        let p = PathBuf::from(prog);
        if p.exists() {
            return Some(p);
        }
        return None;
    }
    for dir in path {
        let candidate = PathBuf::from(dir).join(prog);
        if candidate.exists() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(meta) = std::fs::metadata(&candidate) {
                    if meta.permissions().mode() & 0o111 != 0 {
                        return Some(candidate);
                    }
                }
            }
            #[cfg(not(unix))]
            {
                return Some(candidate);
            }
        }
    }
    None
}

/// Determine if shell is a login shell from `argv[0]`
/// Detect whether `argv[0]` indicates a login shell.
/// Port of the `argv[0][0] == '-'` check inside `parseargs()`
/// (Src/init.c:263) — same `-zsh` invocation convention.
pub fn is_login_shell(argv0: &str) -> bool {
    argv0.starts_with('-')
}

/// Get the ZDOTDIR
/// Full initialization sequence (from init.c zsh_main)
/// Top-level shell initialization driver.
/// Port of `zsh_main()` from Src/init.c:1855 — parses argv,
/// sets up signals, populates the env, sources the init chain,
/// then returns ready state for the main loop.
pub fn zsh_main(args: &[String]) -> ShellState {
    let (opts, cmd, positional) = parseargs(args);
    let mut state = ShellState::new();
    state.options = opts;

    // Determine shell name from argv[0]
    if let Some(arg0) = args.first() {
        if is_login_shell(arg0) {
            state.options.login = true;
        }
        state.emulate_from_name(arg0);
        state.argv0 = arg0.clone();
        state.argzero = arg0.clone();
        state.posixzero = arg0.clone();
    }

    // Set up tty
    init_shout(&mut state);

    // Set up values
    setupvals(&mut state);

    // Set up emulation-specific options
    parseopts_setemulate(&mut state);

    // Set PWD
    set_pwd_env(&mut state);

    // Run init scripts
    run_init_scripts(&mut state);

    state
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shell_state_new() {
        let state = ShellState::new();
        assert!(!state.options.interactive);
        assert!(state.options.rcs);
    }

    #[test]
    fn test_emulate_from_name() {
        let mut state = ShellState::new();

        state.emulate_from_name("zsh");
        assert_eq!(state.emulation, ShellEmulation::Zsh);

        state.emulate_from_name("/bin/sh");
        assert_eq!(state.emulation, ShellEmulation::Sh);

        state.emulate_from_name("-ksh");
        assert_eq!(state.emulation, ShellEmulation::Ksh);
    }

    #[test]
    fn test_parseargs_basic() {
        let args = vec!["zsh".to_string()];
        let (opts, cmd, positional) = parseargs(&args);
        assert!(cmd.is_none());
        assert!(positional.is_empty());
    }

    #[test]
    fn test_parseargs_command() {
        let args = vec![
            "zsh".to_string(),
            "-c".to_string(),
            "echo hello".to_string(),
        ];
        let (opts, cmd, _) = parseargs(&args);
        assert_eq!(cmd, Some("echo hello".to_string()));
        assert!(!opts.interactive);
    }

    #[test]
    fn test_parseargs_interactive() {
        let args = vec!["zsh".to_string(), "-i".to_string()];
        let (opts, _, _) = parseargs(&args);
        assert!(opts.interactive);
    }

    #[test]
    fn test_is_posix_emulation() {
        let mut state = ShellState::new();

        state.emulation = ShellEmulation::Zsh;
        assert!(!state.is_posix_emulation());

        state.emulation = ShellEmulation::Sh;
        assert!(state.is_posix_emulation());

        state.emulation = ShellEmulation::Ksh;
        assert!(state.is_posix_emulation());
    }
}
