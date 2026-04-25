//! Shell initialization for zshrs
//!
//! Port from zsh/Src/init.c
//!
//! Provides shell initialization, startup script sourcing, and main loop.

use std::env;
use std::path::{Path, PathBuf};

/// Shell initialization options
#[derive(Clone, Debug, Default)]
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
pub enum LoopReturn {
    Ok,
    Empty,
    Error,
}

/// Source result
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceReturn {
    Ok,
    NotFound,
    Error,
}

/// Parse command line arguments
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
        if !cmd.is_some() {
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
pub fn sourcehome(state: &mut ShellState, filename: &str) -> SourceReturn {
    let zdotdir = env::var("ZDOTDIR").unwrap_or_else(|_| state.home.clone());
    let path = format!("{}/{}", zdotdir, filename);
    source(state, &path)
}

/// Run initialization scripts
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

// ---------------------------------------------------------------------------
// Missing functions from init.c
// ---------------------------------------------------------------------------

/// Initialize terminal settings (from init.c init_term)
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
pub fn set_pwd_env(state: &mut ShellState) {
    if let Ok(cwd) = env::current_dir() {
        state.pwd = cwd.to_string_lossy().to_string();
    }
    env::set_var("PWD", &state.pwd);
    env::set_var("OLDPWD", &state.oldpwd);
}

/// Run logout scripts (from init.c run_exit_scripts counterpart)
pub fn run_exit_scripts(state: &mut ShellState) {
    if state.options.login {
        if state.options.rcs && state.options.global_rcs {
            source(state, "/etc/zlogout");
        }
        if state.options.rcs && !state.options.privileged {
            sourcehome(state, ".zlogout");
        }
    }
}

/// Close the shell (from init.c zexit)
pub fn zexit(val: i32, from_where: i32) -> ! {
    // from_where: 0=normal, 1=signal, 2=exec
    std::process::exit(val)
}

/// Set up the tty (from init.c init_tty)
pub fn init_tty(state: &mut ShellState) {
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

/// Set up the hash tables (from init.c init_hashtable equivalent)
pub fn init_hashtable() {
    // In Rust, hash tables are managed by the exec module
    // This is a placeholder for compatibility
}

/// Set up options from emulation mode (from init.c setupvals emulation portion)
pub fn setup_emulation_opts(state: &mut ShellState) {
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

/// Determine if shell is a login shell from argv[0]
pub fn is_login_shell(argv0: &str) -> bool {
    argv0.starts_with('-')
}

/// Get the ZDOTDIR
pub fn get_zdotdir() -> String {
    env::var("ZDOTDIR").unwrap_or_else(|_| env::var("HOME").unwrap_or_else(|_| ".".to_string()))
}

/// Full initialization sequence (from init.c init_main)
pub fn init_main(args: &[String]) -> ShellState {
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
    init_tty(&mut state);

    // Set up values
    setupvals(&mut state);

    // Set up emulation-specific options
    setup_emulation_opts(&mut state);

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
