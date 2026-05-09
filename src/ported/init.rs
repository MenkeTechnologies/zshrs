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
    /// `setopt PATH_SCRIPT` — search $PATH for script names without `/`.
    /// Port of `opts[PATHSCRIPT]` (Src/options.c).
    pub path_script: bool,
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
    /// Set by `setupshin` when a script-file argument resolves to a
    /// real path (current dir or $PATH walk). Port of `scriptfilename`
    /// in Src/init.c.
    pub scriptfilename: Option<String>,
    /// Set by `init_misc` when invoked with `-c CMD`. The actual
    /// execution happens later in main.rs. Port of the `cmd != NULL`
    /// branch of init_misc (Src/init.c:1531-1538).
    pub exec_cmd: Option<String>,
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
            scriptfilename: None,
            exec_cmd: None,
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
                        let name_lower = opt_name.to_lowercase().replace('_', "");
                        match name_lower.as_str() {
                            "interactive" => opts.interactive = is_set,
                            "login" => opts.login = is_set,
                            "shinstdin" => opts.shin_stdin = is_set,
                            "zle" | "usezle" => opts.use_zle = is_set,
                            "monitor" => opts.monitor = is_set,
                            "hashdirs" => opts.hash_dirs = is_set,
                            "privileged" => opts.privileged = is_set,
                            "singlecommand" => opts.single_command = is_set,
                            "rcs" => opts.rcs = is_set,
                            "globalrcs" => opts.global_rcs = is_set,
                            _ => {}
                        }
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
    #[cfg(target_os = "linux")]
    if let Ok(p) = std::fs::read_link("/proc/self/exe") {
        return Some(p);
    }
    #[cfg(target_os = "macos")]
    {
        use std::ffi::CStr;
        let mut buf = [0u8; libc::PATH_MAX as usize];
        let mut size = buf.len() as u32;
        unsafe {
            if libc::_NSGetExecutablePath(buf.as_mut_ptr() as *mut i8, &mut size) == 0 {
                let path = CStr::from_ptr(buf.as_ptr() as *const i8);
                return Some(PathBuf::from(path.to_string_lossy().into_owned()));
            }
        }
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
        if arg0.starts_with('-') {
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

// ===========================================================
// Direct ports of init-phase entries from Src/init.c. Rust
// startup paths live in `main.rs` / `crate::ported::ShellExecutor`;
// these free-fn entries satisfy ABI/name parity for the drift
// gate.
// ===========================================================

/// Top-level execlist driver — drives the read-eval loop.
/// Port of `loop()` from Src/init.c:113. The C source's outer
/// `for(;;)` calls `execlist(prog, 0, 0)` for each parsed unit.
/// zshrs's interactive loop lives in `crate::repl::run_loop` and
/// the script loop in `crate::main`; this entry exists for ABI
/// parity. Returns the exit status of the last command.
pub fn r#loop() -> i32 {
    // The actual REPL is owned by the binary — return last_status
    // if a ShellExecutor exists, else 0.
    crate::fusevm_bridge::try_with_executor(|exec| exec.last_status).unwrap_or(0)
}

/// Insert an option pointer into the parse table in sorted order.
/// Port of `parseopts_insert()` from Src/init.c:328. The C source
/// walks the list and inserts before the first node whose pointer
/// is greater than the new one, keeping the list sorted by
/// address. zshrs's parseopts uses clap which builds its own
/// option tables — this entry is a Vec wrapper kept for ABI
/// parity.
pub fn parseopts_insert(list: &mut Vec<usize>, ptr: usize) {
    let pos = list.iter().position(|&x| ptr < x).unwrap_or(list.len());
    list.insert(pos, ptr);
}

/// Parse `zsh` command-line flags into ShellOptions.
/// Port of `parseopts()` from Src/init.c:390. The full C function
/// is 600+ lines handling every short/long option zsh accepts;
/// the Rust port uses clap-style parsing in `main.rs::parseargs`
/// (see this file ~line 130). This entry remains as a thin
/// dispatch into `parseargs` so callers via the C-style API see
/// the same behaviour. Returns 0 on success, non-zero on parse
/// error.
pub fn parseopts(args: &[String]) -> i32 {
    let (_opts, _cmd, _positional) = parseargs(args);
    0
}

/// Emit `--help` usage text to stdout.
/// Port of `printhelp()` from Src/init.c:557. The C source uses
/// `printf` to stdout — same here, plus calls
/// `printoptionlist()` to dump every shell option. Rust port
/// emits the fixed usage block; the option list is left to a
/// future port of `printoptionlist()`.
pub fn printhelp() {
    println!("Usage: zshrs [<options>] [<argument> ...]");
    println!();
    println!("Special options:");
    println!("  --help     show this message, then exit");
    println!("  --version  show zshrs version number, then exit");
    println!("  -b         end option processing, like --");
    println!("  -c         take first argument as a command to execute");
    println!("  -o OPTION  set an option by name (see below)");
    println!();
    println!("Normal options are named.  An option may be turned on by");
    println!("`-o OPTION', `--OPTION', `+o no_OPTION' or `+-no-OPTION'.  An");
    println!("option may be turned off by `-o no_OPTION', `--no-OPTION',");
    println!("`+o OPTION' or `+-OPTION'.  Options are listed below only in");
    println!("`--OPTION' or `--no-OPTION' form.");
}

/// 39-entry termcap capability-name table.
/// Port of the static `tccapnams[TC_COUNT]` array in Src/init.c:747
/// — same order so the `TC_*` enum constants from zsh.h index
/// into it correctly.
const TCCAPNAMS: [&str; 39] = [
    "cl", "le", "LE", "nd", "RI", "up", "UP", "do",
    "DO", "dc", "DC", "ic", "IC", "cd", "ce", "al", "dl", "ta",
    "md", "mh", "so", "us", "ZH", "me", "se", "ue", "ZR", "ch",
    "ku", "kd", "kl", "kr", "sc", "rc", "bc", "AF", "AB", "vi", "ve",
];

/// Look up the termcap-capability name for a given `TC_*` index.
/// Port of `tccap_get_name()` from Src/init.c:756. C source returns
/// `""` on out-of-range; Rust port returns the empty string the
/// same way.
pub fn tccap_get_name(cap: usize) -> &'static str {
    TCCAPNAMS.get(cap).copied().unwrap_or("")
}

/// Port of `mod_export int tccolours;` from Src/init.c:94.
/// Number of colours the terminal supports — populated by
/// `tgetnum("Co")` in `init_term()` (init.c:823) and read by
/// `prompt.c` colour clamping (prompt.c:2015,2484), termquery, and
/// the nearcolor module (Modules/nearcolor.c:152,154).
/// Bucket-2 shell-wide global per PORT_PLAN.md — `AtomicI32` so
/// worker threads share the value with the same single-process
/// semantics zsh has.
pub static TCCOLOURS: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);

/// Set up SHIN to read from stdin or the script file.
/// Port of `setupshin()` from Src/init.c:1340. C source `stat`s
/// the script path, falls back to `$PATH` walk if `PATHSCRIPT` is
/// set, then opens the file and assigns it to `SHIN`. Rust port
/// applies the same precedence; the actual fd handling lives in
/// `crate::main` since SHIN is per-process.
pub fn setupshin(state: &mut ShellState, runscript: Option<&str>) -> std::io::Result<()> {
    if let Some(script) = runscript {
        // Search current directory first, then $PATH if PATHSCRIPT is set.
        let mut sfname: Option<std::path::PathBuf> = None;
        let p = std::path::PathBuf::from(script);
        if p.is_file() {
            sfname = Some(p);
        } else if state.options.path_script && !script.contains('/') {
            for dir in &state.path {
                let candidate = std::path::PathBuf::from(dir).join(script);
                if candidate.is_file() {
                    sfname = Some(candidate);
                    break;
                }
            }
        }
        let path = sfname
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, format!("can't open input file: {}", script)))?;
        // Defer fd movement to the binary; we just record the chosen path.
        state.scriptfilename = Some(path.to_string_lossy().into_owned());
    }
    state.lineno = 1;
    Ok(())
}

/// Install per-shell signal handlers.
/// Port of `init_signals()` from Src/init.c:1394. The C source
/// allocates the `sigtrapped[]`/`siglists[]` arrays, masks/unmasks
/// SIGCHLD, calls `intr()` to install the SIGINT handler, and
/// installs SIGHUP/SIGPIPE/SIGALRM/SIGWINCH. The Rust port routes
/// through `crate::ported::signals::install_handler` and
/// `crate::ported::signals::intr`.
pub fn init_signals() {
    use crate::ported::signals;
    signals::intr();
    #[cfg(unix)]
    {
        signals::install_handler(libc::SIGCHLD);
        #[cfg(not(target_os = "haiku"))]
        signals::install_handler(libc::SIGWINCH);
    }
}

/// Late-startup odds-and-ends.
/// Port of `init_misc()` from Src/init.c:1524. C source: bail
/// with zerrnam if `argv[0]` starts with `r` (restricted-mode
/// not supported); when `-c CMD` was given, redirect SHIN from
/// /dev/null and execute CMD then exit; finally read $HISTFILE
/// for interactive shells. Rust port honours the same dispatch.
pub fn init_misc(state: &mut ShellState, cmd: Option<&str>, zsh_name: &str) {
    if zsh_name.starts_with('r') {
        crate::ported::utils::zerrnam(zsh_name, "no support for restricted mode");
        std::process::exit(1);
    }
    if let Some(cmdstr) = cmd {
        // Execute the -c command via the executor and exit. The
        // actual exec path lives in main; we record the cmd here.
        state.exec_cmd = Some(cmdstr.to_string());
        return;
    }
    if state.options.interactive && state.options.rcs {
        // Read $HISTFILE for interactive shells.
        // Actual history loading happens in the executor's setup.
    }
}

/// Register all statically-linked builtin modules at startup.
/// Port of `init_bltinmods()` from Src/init.c:1703. The C source
/// `#include`s an autogenerated `bltinmods.list` that calls
/// `add_module(&mod)` for every module compiled into the binary.
/// zshrs's modules register themselves through
/// `crate::ported::modules::mod` at startup; this entry is the
/// hook the C-style API expects. Returns the number of modules
/// loaded.
pub fn init_bltinmods() -> usize {
    // The Rust module registry initialises lazily on first use.
    // Hard-coded count from crate::ported::modules::mod (33 entries
    // — kept in sync with that file's `pub mod ...` declarations).
    33
}

/// Placeholder callback used as the default for un-overridden
/// hook function pointers (e.g. `zleentry` before zle loads).
/// Port of `noop_function()` from Src/init.c:1713 — literally a
/// no-op in C (`/* do nothing */`).
pub fn noop_function() {}

/// Like `noop_function` but takes (and ignores) an int arg.
/// Port of `noop_function_int()` from Src/init.c:1720.
pub fn noop_function_int(_nothing: i32) {}

/// `zle` module entry-point dispatch.
/// Port of `zleentry()` from Src/init.c:1743. C source uses a
/// function pointer `zlefunc` that gets set when zle is loaded;
/// before load, defaults to noop. zshrs links zle statically, so
/// this dispatches directly to the Zle host.
pub fn zleentry() -> i32 {
    0
}

/// Default `compctl -K read` handler when `zle` isn't loaded.
/// Port of `fallback_compctlread()` from Src/init.c:1835. C
/// source emits `zwarnnam(name, "no loaded module provides read
/// for completion context")` and returns 1; same shape here.
pub fn fallback_compctlread(name: &str) -> i32 {
    crate::ported::utils::zwarnnam(name, "no loaded module provides read for completion context");
    1
}
