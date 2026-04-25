//! zshrs - The most powerful shell ever created
//!
//! A drop-in zsh replacement that combines:
//! - Full bash/zsh script compatibility  
//! - Fish-quality completions with SQLite indexing
//! - Fish-style syntax highlighting and autosuggestions
//!
//! Copyright (C) 2026 MenkeTechnologies
//! License: GPL-2.0 (incorporates code from fish-shell)

use std::env;
use std::io::{self, Read};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use nu_ansi_term::{Color as AnsiColor, Style as AnsiStyle};
use reedline::Color;
use reedline::{
    default_emacs_keybindings, Completer, DefaultHinter, Editor, Emacs, FileBackedHistory,
    Highlighter, KeyCode, KeyModifiers, Menu as ReedlineMenuTrait, MenuBuilder, MenuEvent,
    MenuSettings, Painter, Prompt, PromptHistorySearch, PromptHistorySearchStatus, Reedline,
    ReedlineEvent, ReedlineMenu, Signal, Span, StyledText, Suggestion, ValidationResult, Validator,
    menu_functions,
};

use zsh::exec::ShellExecutor;
use zsh::history::HistoryEngine;
use zsh::zwc;

use compsys::{
    build_cache_from_fpath, cache::CompsysCache, compinit_lazy, do_completion, get_system_fpath,
    completion::CompletionGroup, menu::MenuState, Completion as CompsysCompletion,
    CompletionState,
};

use zsh::{
    highlight_shell, validate_command, with_abbrs_mut, AbbrPosition, Abbreviation, HighlightRole,
    ValidationStatus,
};

/// Print help message identical to zsh --help
fn print_help() {
    println!(
        r#"Usage: zsh [<options>] [<argument> ...]

Special options:
  --help       show this message, then exit
  --version    show zsh version number, then exit
  --doctor     full diagnostic report of shell health, caches, and performance
  --zsh-compat enable zsh compatibility mode (use .zcompdump, fpath scanning)
  --posix      POSIX strict mode (no SQLite, no worker pool, no zsh extensions)
  -b           end option processing, like --
  -c           take first argument as a command to execute
  -f           equivalent to --no-rcs (don't source startup files)
  -i           force interactive mode
  -l           force login shell mode
  -s           read commands from stdin
  -o OPTION    set an option by name (see below)
  -v           verbose (equivalent to --verbose)
  -x           xtrace (equivalent to --xtrace)

Normal options are named.  An option may be turned on by
`-o OPTION', `--OPTION', `+o no_OPTION' or `+-no-OPTION'.  An
option may be turned off by `-o no_OPTION', `--no-OPTION',
`+o OPTION' or `+-OPTION'.  Options are listed below only in
`--OPTION' or `--no-OPTION' form.

Named options:
  --aliases
  --aliasfuncdef
  --allexport
  --alwayslastprompt
  --alwaystoend
  --appendcreate
  --appendhistory
  --autocd
  --autocontinue
  --autolist
  --automenu
  --autonamedirs
  --autoparamkeys
  --autoparamslash
  --autopushd
  --autoremoveslash
  --autoresume
  --badpattern
  --banghist
  --bareglobqual
  --bashautolist
  --bashrematch
  --beep
  --bgnice
  --braceccl
  --bsdecho
  --caseglob
  --casematch
  --casepaths
  --cbases
  --cdablevars
  --cdsilent
  --chasedots
  --chaselinks
  --checkjobs
  --checkrunningjobs
  --clobber
  --clobberempty
  --combiningchars
  --completealiases
  --completeinword
  --continueonerror
  --correct
  --correctall
  --cprecedences
  --cshjunkiehistory
  --cshjunkieloops
  --cshjunkiequotes
  --cshnullcmd
  --cshnullglob
  --debugbeforecmd
  --dvorak
  --emacs
  --equals
  --errexit
  --errreturn
  --evallineno
  --exec
  --extendedglob
  --extendedhistory
  --flowcontrol
  --forcefloat
  --functionargzero
  --glob
  --globalexport
  --globalrcs
  --globassign
  --globcomplete
  --globdots
  --globstarshort
  --globsubst
  --hashcmds
  --hashdirs
  --hashexecutablesonly
  --hashlistall
  --histallowclobber
  --histbeep
  --histexpiredupsfirst
  --histfcntllock
  --histfindnodups
  --histignorealldups
  --histignoredups
  --histignorespace
  --histlexwords
  --histnofunctions
  --histnostore
  --histreduceblanks
  --histsavebycopy
  --histsavenodups
  --histsubstpattern
  --histverify
  --hup
  --ignorebraces
  --ignoreclosebraces
  --ignoreeof
  --incappendhistory
  --incappendhistorytime
  --interactive
  --interactivecomments
  --ksharrays
  --kshautoload
  --kshglob
  --kshoptionprint
  --kshtypeset
  --kshzerosubscript
  --listambiguous
  --listbeep
  --listpacked
  --listrowsfirst
  --listtypes
  --localloops
  --localoptions
  --localpatterns
  --localtraps
  --login
  --longlistjobs
  --magicequalsubst
  --mailwarning
  --markdirs
  --menucomplete
  --monitor
  --multibyte
  --multifuncdef
  --multios
  --nomatch
  --notify
  --nullglob
  --numericglobsort
  --octalzeroes
  --overstrike
  --pathdirs
  --pathscript
  --pipefail
  --posixaliases
  --posixargzero
  --posixbuiltins
  --posixcd
  --posixidentifiers
  --posixjobs
  --posixstrings
  --posixtraps
  --printeightbit
  --printexitvalue
  --privileged
  --promptbang
  --promptcr
  --promptpercent
  --promptsp
  --promptsubst
  --pushdignoredups
  --pushdminus
  --pushdsilent
  --pushdtohome
  --rcexpandparam
  --rcquotes
  --rcs
  --recexact
  --rematchpcre
  --restricted
  --rmstarsilent
  --rmstarwait
  --sharehistory
  --shfileexpansion
  --shglob
  --shinstdin
  --shnullcmd
  --shoptionletters
  --shortloops
  --shortrepeat
  --shwordsplit
  --singlecommand
  --singlelinezle
  --sourcetrace
  --sunkeyboardhack
  --transientrprompt
  --trapsasync
  --typesetsilent
  --typesettounset
  --unset
  --verbose
  --vi
  --warncreateglobal
  --warnnestedvar
  --xtrace
  --zle

Option aliases:
  --braceexpand            equivalent to --no-ignorebraces
  --dotglob                equivalent to --globdots
  --hashall                equivalent to --hashcmds
  --histappend             equivalent to --appendcreate
  --histexpand             equivalent to --badpattern
  --log                    equivalent to --no-histnofunctions
  --mailwarn               equivalent to --mailwarning
  --onecmd                 equivalent to --singlecommand
  --physical               equivalent to --cdsilent
  --promptvars             equivalent to --promptsubst
  --stdin                  equivalent to --shinstdin
  --trackall               equivalent to --hashcmds

Option letters:
  -0    equivalent to --completeinword
  -1    equivalent to --printexitvalue
  -2    equivalent to --no-autoresume
  -3    equivalent to --no-nomatch
  -4    equivalent to --globdots
  -5    equivalent to --notify
  -6    equivalent to --beep
  -7    equivalent to --ignoreeof
  -8    equivalent to --markdirs
  -9    equivalent to --autocontinue
  -B    equivalent to --no-bashrematch
  -C    equivalent to --no-checkjobs
  -D    equivalent to --pushdtohome
  -E    equivalent to --pushdsilent
  -F    equivalent to --no-glob
  -G    equivalent to --nullglob
  -H    equivalent to --rmstarsilent
  -I    equivalent to --ignorebraces
  -J    equivalent to --appendhistory
  -K    equivalent to --no-badpattern
  -L    equivalent to --sunkeyboardhack
  -M    equivalent to --singlelinezle
  -N    equivalent to --autoparamslash
  -O    equivalent to --continueonerror
  -P    equivalent to --rcexpandparam
  -Q    equivalent to --pathdirs
  -R    equivalent to --longlistjobs
  -S    equivalent to --recexact
  -T    equivalent to --cbases
  -U    equivalent to --mailwarning
  -V    equivalent to --no-promptcr
  -W    equivalent to --autoremoveslash
  -X    equivalent to --listtypes
  -Y    equivalent to --menucomplete
  -Z    equivalent to --zle
  -a    equivalent to --allexport
  -d    equivalent to --no-globalrcs
  -e    equivalent to --errexit
  -f    equivalent to --no-rcs
  -g    equivalent to --histignorespace
  -h    equivalent to --histignoredups
  -i    equivalent to --interactive
  -k    equivalent to --interactivecomments
  -l    equivalent to --login
  -m    equivalent to --monitor
  -n    equivalent to --no-exec
  -p    equivalent to --privileged
  -r    equivalent to --restricted
  -s    equivalent to --shinstdin
  -t    equivalent to --singlecommand
  -u    equivalent to --no-unset
  -v    equivalent to --verbose
  -w    equivalent to --cdsilent
  -x    equivalent to --xtrace
  -y    equivalent to --shwordsplit
"#
    );
}

/// Initialize default fish-style abbreviations
fn init_default_abbreviations() {
    with_abbrs_mut(|set| {
        // Git abbreviations (command position only)
        set.add(Abbreviation::new("g", "g", "git", AbbrPosition::Command));
        set.add(Abbreviation::new(
            "ga",
            "ga",
            "git add",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "gaa",
            "gaa",
            "git add --all",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "gc",
            "gc",
            "git commit",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "gcm",
            "gcm",
            "git commit -m",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "gco",
            "gco",
            "git checkout",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "gd",
            "gd",
            "git diff",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "gds",
            "gds",
            "git diff --staged",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "gl",
            "gl",
            "git log --oneline",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "gp",
            "gp",
            "git push",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "gpl",
            "gpl",
            "git pull",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "gs",
            "gs",
            "git status",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "gsw",
            "gsw",
            "git switch",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "gb",
            "gb",
            "git branch",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "gst",
            "gst",
            "git stash",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "grb",
            "grb",
            "git rebase",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "gm",
            "gm",
            "git merge",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "gf",
            "gf",
            "git fetch",
            AbbrPosition::Command,
        ));

        // Directory navigation
        set.add(Abbreviation::new(
            "...",
            "...",
            "../..",
            AbbrPosition::Anywhere,
        ));
        set.add(Abbreviation::new(
            "....",
            "....",
            "../../..",
            AbbrPosition::Anywhere,
        ));
        set.add(Abbreviation::new(
            ".....",
            ".....",
            "../../../..",
            AbbrPosition::Anywhere,
        ));

        // Common commands
        set.add(Abbreviation::new("l", "l", "ls -la", AbbrPosition::Command));
        set.add(Abbreviation::new(
            "ll",
            "ll",
            "ls -l",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "la",
            "la",
            "ls -la",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "md",
            "md",
            "mkdir -p",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "rd",
            "rd",
            "rmdir",
            AbbrPosition::Command,
        ));

        // Cargo/Rust
        set.add(Abbreviation::new(
            "cb",
            "cb",
            "cargo build",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "cr",
            "cr",
            "cargo run",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "ct",
            "ct",
            "cargo test",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "cc",
            "cc",
            "cargo check",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "cf",
            "cf",
            "cargo fmt",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "ccl",
            "ccl",
            "cargo clippy",
            AbbrPosition::Command,
        ));

        // Docker
        set.add(Abbreviation::new(
            "dc",
            "dc",
            "docker compose",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "dcu",
            "dcu",
            "docker compose up",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "dcd",
            "dcd",
            "docker compose down",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "dps",
            "dps",
            "docker ps",
            AbbrPosition::Command,
        ));

        // Kubernetes
        set.add(Abbreviation::new(
            "k",
            "k",
            "kubectl",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "kgp",
            "kgp",
            "kubectl get pods",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "kgs",
            "kgs",
            "kubectl get services",
            AbbrPosition::Command,
        ));
        set.add(Abbreviation::new(
            "kgd",
            "kgd",
            "kubectl get deployments",
            AbbrPosition::Command,
        ));
    });
}

/// Shell mode: zshrs (default), --zsh (zsh drop-in), --posix (POSIX sh strict)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellMode {
    /// Full zshrs — all features, --doctor, plugin cache UI, exclusive builtins
    Zshrs,
    /// zsh drop-in — same external interface as zsh, no zshrs-exclusive features visible,
    /// but full zshrs engine underneath (SQLite, worker pool, parallel everything)
    Zsh,
    /// POSIX sh strict — only POSIX builtins, no zsh extensions, no arrays, no [[,
    /// no extended globbing, no SQLite caches, no worker pool. Dinosaur mode.
    Posix,
}

static mut SHELL_MODE: ShellMode = ShellMode::Zshrs;

/// Global log file path for zshrs background operations (compinit, etc.)
pub fn zshrs_log_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".cache/zshrs/zshrs.log")
}

pub fn shell_mode() -> ShellMode {
    unsafe { SHELL_MODE }
}

pub fn is_zsh_mode() -> bool {
    matches!(shell_mode(), ShellMode::Zsh)
}

pub fn is_posix_mode() -> bool {
    matches!(shell_mode(), ShellMode::Posix)
}

pub fn is_zshrs_mode() -> bool {
    matches!(shell_mode(), ShellMode::Zshrs)
}

/// Legacy compat shim — maps to --zsh mode
pub fn is_zsh_compat() -> bool {
    is_zsh_mode()
}

fn main() {
    zshrs_main();
}

/// Main entry point — extracted so the fat binary can call it after
/// registering the stryke handler.
pub fn zshrs_main() {
    // Initialize logging first — everything after this can use tracing macros.
    let startup_t0 = Instant::now();

    // Default level: info. Override with ZSHRS_LOG=debug or ZSHRS_LOG=trace.
    zsh::log::init();

    let pid = std::process::id();
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "?".to_string());
    let path_count = env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .count();
    let fpath_count = env::var("FPATH")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .count();
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);

    tracing::info!(
        pid,
        cwd = %cwd,
        path_dirs = path_count,
        fpath_dirs = fpath_count,
        cpus,
        "zshrs starting"
    );

    let args: Vec<String> = env::args().collect();

    // Handle shell mode flags (must be checked early)
    if args.iter().any(|a| a == "--posix") {
        unsafe { SHELL_MODE = ShellMode::Posix; }
    } else if args.iter().any(|a| a == "--zsh" || a == "--zsh-compat") {
        unsafe { SHELL_MODE = ShellMode::Zsh; }
    }
    tracing::info!(mode = ?shell_mode(), "shell mode selected");

    // Handle --help (must be identical to zsh --help)
    if args.iter().any(|a| a == "--help") {
        print_help();
        return;
    }

    // Handle --version
    if args.iter().any(|a| a == "--version") {
        println!("zshrs {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    // Handle --doctor (zshrs-exclusive, not available in --zsh or --posix)
    if args.iter().any(|a| a == "--doctor") {
        if is_zshrs_mode() {
            run_doctor();
        } else {
            eprintln!("zshrs: --doctor is only available in zshrs mode (not --zsh or --posix)");
            std::process::exit(1);
        }
        return;
    }

    // Handle --dump-zwc for debugging .zwc files
    if args.len() >= 3 && args[1] == "--dump-zwc" {
        if args.len() >= 4 {
            // Dump specific function
            if let Err(e) = zwc::dump_zwc_function(&args[2], &args[3]) {
                eprintln!("zshrs: {}: {}", args[2], e);
                std::process::exit(1);
            }
        } else {
            // List all functions
            if let Err(e) = zwc::dump_zwc_info(&args[2]) {
                eprintln!("zshrs: {}: {}", args[2], e);
                std::process::exit(1);
            }
        }
        return;
    }

    // Extract flags before filtering: -x (xtrace), -f (no rcs), -v (verbose)
    let enable_xtrace = args.iter().any(|a| a == "-x");
    let enable_verbose = args.iter().any(|a| a == "-v");

    // Filter out flags that don't affect -c / script dispatch
    let args: Vec<String> = args
        .into_iter()
        .filter(|a| a != "--zsh-compat" && a != "--zsh" && a != "--posix" && a != "-f" && a != "--no-rcs" && a != "-x" && a != "-v")
        .collect();

    /// Apply CLI flags and shell mode to executor
    fn apply_cli_flags(executor: &mut ShellExecutor, xtrace: bool, verbose: bool) {
        // Apply shell mode
        executor.zsh_compat = is_zsh_mode();
        if is_posix_mode() {
            executor.enter_posix_mode();
        }
        if xtrace {
            executor.options.insert("xtrace".to_string(), true);
        }
        if verbose {
            executor.options.insert("verbose".to_string(), true);
        }
    }

    // Handle -c 'command' syntax
    if args.len() >= 3 && args[1] == "-c" {
        let code = &args[2];

        let mut executor = ShellExecutor::new();
        apply_cli_flags(&mut executor, enable_xtrace, enable_verbose);
        let start = Instant::now();
        let result = executor.execute_script(code);
        let duration = start.elapsed().as_millis() as i64;

        // Track in history
        if let Some(ref engine) = executor.history {
            let cwd = std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().to_string());
            if let Ok(id) = engine.add(code, cwd.as_deref()) {
                let _ = engine.update_last(id, duration, executor.last_status);
            }
        }

        if let Err(e) = result {
            eprintln!("zshrs: {}", e);
            std::process::exit(1);
        }
        std::process::exit(executor.last_status);
        #[allow(unreachable_code)]
        return;
    }

    // Handle script file argument
    if args.len() >= 2 && !args[1].starts_with('-') {
        let mut executor = ShellExecutor::new();
        apply_cli_flags(&mut executor, enable_xtrace, enable_verbose);
        if let Err(e) = executor.execute_script_file(&args[1]) {
            eprintln!("zshrs: {}: {}", args[1], e);
            std::process::exit(1);
        }
        return;
    }

    tracing::info!(
        startup_ms = startup_t0.elapsed().as_millis() as u64,
        "startup complete, entering main loop"
    );

    // Check if stdin is a TTY
    if atty::is(atty::Stream::Stdin) {
        run_interactive();
    } else {
        run_non_interactive();
    }
}

/// zshrs --doctor: full diagnostic report of shell health, caches, and performance.
fn run_doctor() {
    use std::os::unix::fs::MetadataExt;

    let green = |s: &str| format!("\x1b[32m{}\x1b[0m", s);
    let red = |s: &str| format!("\x1b[31m{}\x1b[0m", s);
    let yellow = |s: &str| format!("\x1b[33m{}\x1b[0m", s);
    let bold = |s: &str| format!("\x1b[1m{}\x1b[0m", s);
    let dim = |s: &str| format!("\x1b[2m{}\x1b[0m", s);

    println!("{}", bold("zshrs doctor"));
    println!("{}", dim(&"=".repeat(60)));
    println!();

    // --- Version & Environment ---
    println!("{}", bold("Environment"));
    println!("  version:    zshrs {}", env!("CARGO_PKG_VERSION"));
    println!("  pid:        {}", std::process::id());
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "?".to_string());
    println!("  cwd:        {}", cwd);
    println!("  shell:      {}", std::env::var("SHELL").unwrap_or_else(|_| "?".to_string()));
    let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    println!("  cpus:       {}", cpus);
    let pool_size = cpus.clamp(2, 18);
    println!("  pool size:  {}", pool_size);
    println!();

    // --- PATH ---
    println!("{}", bold("PATH"));
    let path_var = std::env::var("PATH").unwrap_or_default();
    let path_dirs: Vec<&str> = path_var.split(':').filter(|s| !s.is_empty()).collect();
    let mut path_ok = 0usize;
    let mut path_missing = 0usize;
    let mut path_cmds = 0usize;
    for dir in &path_dirs {
        if std::path::Path::new(dir).is_dir() {
            path_ok += 1;
            if let Ok(entries) = std::fs::read_dir(dir) {
                path_cmds += entries.count();
            }
        } else {
            path_missing += 1;
        }
    }
    println!("  directories: {} total, {} {}, {} {}",
        path_dirs.len(),
        path_ok, green("valid"),
        path_missing, if path_missing > 0 { red("missing") } else { green("missing") },
    );
    println!("  commands:    ~{}", path_cmds);
    if path_missing > 0 {
        for dir in &path_dirs {
            if !std::path::Path::new(dir).is_dir() {
                println!("  {} PATH entry does not exist: {}", red("!"), dir);
            }
        }
    }
    println!();

    // --- FPATH ---
    println!("{}", bold("FPATH"));
    let fpath_var = std::env::var("FPATH").unwrap_or_default();
    let fpath_dirs: Vec<&str> = fpath_var.split(':').filter(|s| !s.is_empty()).collect();
    let mut fpath_ok = 0usize;
    let mut fpath_missing = 0usize;
    let mut fpath_files = 0usize;
    for dir in &fpath_dirs {
        if std::path::Path::new(dir).is_dir() {
            fpath_ok += 1;
            if let Ok(entries) = std::fs::read_dir(dir) {
                fpath_files += entries.count();
            }
        } else {
            fpath_missing += 1;
        }
    }
    println!("  directories:   {} total, {} {}, {} {}",
        fpath_dirs.len(),
        fpath_ok, green("valid"),
        fpath_missing, if fpath_missing > 0 { red("missing") } else { green("missing") },
    );
    println!("  function files: {}", fpath_files);
    println!();

    // --- SQLite Databases ---
    println!("{}", bold("SQLite Caches"));

    // History
    let hist_path = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("zshrs/history.db");
    if hist_path.exists() {
        let size = std::fs::metadata(&hist_path).map(|m| m.len()).unwrap_or(0);
        let count = zsh::history::HistoryEngine::new()
            .ok()
            .and_then(|e| e.count().ok())
            .unwrap_or(0);
        println!("  history.db:  {} entries, {} bytes  {}",
            count,
            format_bytes(size),
            green("OK"),
        );
    } else {
        println!("  history.db:  {}", yellow("not found"));
    }

    // Compsys cache
    let compsys_path = compsys::cache::default_cache_path();
    if compsys_path.exists() {
        let size = std::fs::metadata(&compsys_path).map(|m| m.len()).unwrap_or(0);
        let count = CompsysCache::open(&compsys_path)
            .ok()
            .map(|c| compsys::cache_entry_count(&c))
            .unwrap_or(0);
        println!("  compsys.db:  {} completions, {}  {}",
            count,
            format_bytes(size),
            green("OK"),
        );
    } else {
        println!("  compsys.db:  {}", yellow("not found — run compinit to create"));
    }

    // Plugin cache
    let plugin_path = zsh::plugin_cache::default_cache_path();
    if plugin_path.exists() {
        let size = std::fs::metadata(&plugin_path).map(|m| m.len()).unwrap_or(0);
        let (plugins, functions) = zsh::plugin_cache::PluginCache::open(&plugin_path)
            .map(|c| c.stats())
            .unwrap_or((0, 0));
        println!("  plugins.db:  {} plugins, {} cached functions, {}  {}",
            plugins,
            functions,
            format_bytes(size),
            green("OK"),
        );

        // Check for stale entries
        if let Ok(cache) = zsh::plugin_cache::PluginCache::open(&plugin_path) {
            let stale = count_stale_plugins(&cache);
            if stale > 0 {
                println!("               {} {} plugin(s) have stale cache (file changed since cached)",
                    yellow("!"), stale);
            }
        }
    } else {
        println!("  plugins.db:  {}", yellow("not found — source a file to create"));
    }
    println!();

    // --- Log file ---
    println!("{}", bold("Log"));
    let log_path = zsh::log::log_path();
    if log_path.exists() {
        let size = std::fs::metadata(&log_path).map(|m| m.len()).unwrap_or(0);
        let lines = std::fs::read_to_string(&log_path)
            .map(|s| s.lines().count())
            .unwrap_or(0);
        println!("  {}  {} lines, {}",
            log_path.display(), lines, format_bytes(size));
    } else {
        println!("  {}", dim("no log file yet"));
    }
    println!();

    // --- Startup files ---
    println!("{}", bold("Startup Files"));
    let zdotdir = std::env::var("ZDOTDIR")
        .unwrap_or_else(|_| std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string()));
    let startup_files = [
        ("/etc/zshenv", true),
        (&format!("{}/.zshenv", zdotdir), false),
        ("/etc/zprofile", false),
        (&format!("{}/.zprofile", zdotdir), false),
        ("/etc/zshrc", false),
        (&format!("{}/.zshrc", zdotdir), false),
        ("/etc/zlogin", false),
        (&format!("{}/.zlogin", zdotdir), false),
    ];
    for (path, always) in &startup_files {
        let p = std::path::Path::new(path);
        if p.exists() {
            let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            let cached = is_script_cached(&plugin_path, path);
            let cache_status = if cached { green("cached") } else { yellow("uncached") };
            println!("  {} {} {}  [{}]",
                green("*"),
                path,
                dim(&format!("({})", format_bytes(size))),
                cache_status,
            );
        } else if *always {
            println!("  {} {}", dim("-"), dim(path));
        } else {
            println!("  {} {}", dim("-"), dim(path));
        }
    }
    println!();

    // --- Profiling ---
    println!("{}", bold("Profiling Features"));
    println!("  chrome tracing: {}", if zsh::log::profiling_enabled() { green("enabled") } else { dim("disabled (build with --features profiling)") });
    println!("  flamegraph:     {}", if zsh::log::flamegraph_enabled() { green("enabled") } else { dim("disabled (build with --features flamegraph)") });
    println!("  prometheus:     {}", if zsh::log::prometheus_enabled() { green("enabled") } else { dim("disabled (build with --features prometheus)") });
    println!("  ZSHRS_LOG:      {}", std::env::var("ZSHRS_LOG").unwrap_or_else(|_| "info (default)".to_string()));
    println!();

    // --- Startup benchmark ---
    println!("{}", bold("Startup Benchmark"));
    let t0 = Instant::now();
    let mut executor = ShellExecutor::new();
    let init_ms = t0.elapsed().as_millis();
    println!("  executor init:  {}ms", init_ms);

    let t1 = Instant::now();
    executor.drain_compinit_bg();
    let drain_ms = t1.elapsed().as_millis();
    println!("  compinit drain: {}ms", drain_ms);

    let total = init_ms + drain_ms;
    let status = if total < 30 {
        green(&format!("{}ms — excellent", total))
    } else if total < 100 {
        yellow(&format!("{}ms — good", total))
    } else {
        red(&format!("{}ms — slow", total))
    };
    println!("  total:          {}", status);
    println!();

    // --- Summary ---
    println!("{}", bold("Summary"));
    let mut issues = 0;
    if path_missing > 0 {
        println!("  {} {} PATH entries missing", red("!"), path_missing);
        issues += 1;
    }
    if fpath_missing > 0 {
        println!("  {} {} FPATH entries missing", red("!"), fpath_missing);
        issues += 1;
    }
    if !hist_path.exists() {
        println!("  {} no history database", yellow("!"));
        issues += 1;
    }
    if !compsys_path.exists() {
        println!("  {} no completion cache", yellow("!"));
        issues += 1;
    }
    if total > 100 {
        println!("  {} startup > 100ms", red("!"));
        issues += 1;
    }
    if issues == 0 {
        println!("  {} all checks passed", green("*"));
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{}B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn count_stale_plugins(cache: &zsh::plugin_cache::PluginCache) -> usize {
    cache.count_stale()
}

fn is_script_cached(plugin_db_path: &std::path::Path, script_path: &str) -> bool {
    if !plugin_db_path.exists() {
        return false;
    }
    let cache = match zsh::plugin_cache::PluginCache::open(plugin_db_path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    if let Some((mt_s, mt_ns)) = zsh::plugin_cache::file_mtime(std::path::Path::new(script_path)) {
        cache.check(script_path, mt_s, mt_ns).is_some()
    } else {
        false
    }
}

fn run_non_interactive() {
    let mut executor = ShellExecutor::new();
    executor.zsh_compat = is_zsh_mode();
    if is_posix_mode() { executor.enter_posix_mode(); }
    // Read all of stdin at once so multi-line constructs (heredocs, functions,
    // loops, etc.) are parsed correctly — line-by-line breaks them.
    let mut script = String::new();
    io::stdin().lock().read_to_string(&mut script).unwrap_or(0);
    if !script.is_empty() {
        if let Err(e) = executor.execute_script(&script) {
            eprintln!("zshrs: {}", e);
            std::process::exit(1);
        }
        std::process::exit(executor.last_status);
    }
}

fn get_zdotdir() -> PathBuf {
    std::env::var("ZDOTDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")))
}

/// Source zsh startup files in correct order per zshall(1) STARTUP/SHUTDOWN FILES
///
/// Behavior is controlled by RCS and GLOBAL_RCS options:
/// - RCS (default: on) - if unset, no startup files are read
/// - GLOBAL_RCS (default: on) - if unset, /etc/* files are skipped
///
/// Order for login shell:
///   1. /etc/zshenv (always, cannot be overridden - even with -f)
///   2. $ZDOTDIR/.zshenv
///   3. /etc/zprofile (login only)
///   4. $ZDOTDIR/.zprofile (login only)
///   5. /etc/zshrc (interactive only)
///   6. $ZDOTDIR/.zshrc (interactive only)
///   7. /etc/zlogin (login only)
///   8. $ZDOTDIR/.zlogin (login only)
///
/// If file.zwc exists and is newer than file, the compiled version is used.
///
/// Optimization: all startup file contents are read into memory in parallel
/// (overlapping disk I/O), then executed sequentially in the correct order.
fn source_startup_files(
    executor: &mut ShellExecutor,
    is_login: bool,
    is_interactive: bool,
    no_rcs: bool,
) {
    let zdotdir = get_zdotdir();

    // Build the ordered list of candidate startup files.
    // We read ALL of them in parallel to overlap disk latency, but execute
    // sequentially and honor RCS/GLOBAL_RCS checks between phases.
    let mut candidates: Vec<PathBuf> = Vec::with_capacity(8);

    // Phase 0: /etc/zshenv — always read
    candidates.push(PathBuf::from("/etc/zshenv"));

    if !no_rcs {
        // Phase 1: user .zshenv
        candidates.push(zdotdir.join(".zshenv"));

        // Phase 2: login profile files
        if is_login {
            candidates.push(PathBuf::from("/etc/zprofile"));
            candidates.push(zdotdir.join(".zprofile"));
        }

        // Phase 3: interactive rc files
        if is_interactive {
            candidates.push(PathBuf::from("/etc/zshrc"));
            candidates.push(zdotdir.join(".zshrc"));
        }

        // Phase 4: login files (after zshrc)
        if is_login {
            candidates.push(PathBuf::from("/etc/zlogin"));
            candidates.push(zdotdir.join(".zlogin"));
        }
    }

    // --- Parallel read phase: read all files at once on background threads ---
    let read_start = std::time::Instant::now();
    let file_count = candidates.len();

    let handles: Vec<std::thread::JoinHandle<(PathBuf, Option<String>)>> = candidates
        .into_iter()
        .map(|path| {
            std::thread::spawn(move || {
                let contents = if path.exists() {
                    std::fs::read_to_string(&path).ok()
                } else {
                    None
                };
                (path, contents)
            })
        })
        .collect();

    // Collect results in order (handles are in insertion order)
    let preloaded: Vec<(PathBuf, Option<String>)> = handles
        .into_iter()
        .map(|h| h.join().unwrap_or_else(|_| (PathBuf::new(), None)))
        .collect();

    tracing::debug!(
        files = file_count,
        read_ms = read_start.elapsed().as_millis() as u64,
        "startup files parallel read complete"
    );

    // --- Sequential execution phase: execute in correct order with RCS checks ---

    // Phase 0: /etc/zshenv — always
    if let Some((path, contents)) = preloaded.first() {
        if let Some(ref text) = contents {
            source_from_memory(executor, path, text);
        }
    }

    if no_rcs {
        return;
    }

    // Check RCS after /etc/zshenv
    if !executor.options.get("rcs").copied().unwrap_or(true) {
        return;
    }

    // Phase 1: $ZDOTDIR/.zshenv
    let mut idx = 1;
    if idx < preloaded.len() {
        if let Some(ref text) = preloaded[idx].1 {
            source_from_memory(executor, &preloaded[idx].0, text);
        }
        idx += 1;
    }

    // Re-check RCS after .zshenv
    if !executor.options.get("rcs").copied().unwrap_or(true) {
        return;
    }

    // Phase 2: login profile files
    if is_login {
        // /etc/zprofile
        if idx < preloaded.len() {
            if executor.options.get("globalrcs").copied().unwrap_or(true) {
                if let Some(ref text) = preloaded[idx].1 {
                    source_from_memory(executor, &preloaded[idx].0, text);
                }
            }
            idx += 1;
        }
        // $ZDOTDIR/.zprofile
        if idx < preloaded.len() {
            if executor.options.get("rcs").copied().unwrap_or(true) {
                if let Some(ref text) = preloaded[idx].1 {
                    source_from_memory(executor, &preloaded[idx].0, text);
                }
            }
            idx += 1;
        }
    }

    // Re-check RCS
    if !executor.options.get("rcs").copied().unwrap_or(true) {
        return;
    }

    // Phase 3: interactive rc files
    if is_interactive {
        // /etc/zshrc
        if idx < preloaded.len() {
            if executor.options.get("globalrcs").copied().unwrap_or(true) {
                if let Some(ref text) = preloaded[idx].1 {
                    source_from_memory(executor, &preloaded[idx].0, text);
                }
            }
            idx += 1;
        }
        // $ZDOTDIR/.zshrc
        if idx < preloaded.len() {
            if executor.options.get("rcs").copied().unwrap_or(true) {
                if let Some(ref text) = preloaded[idx].1 {
                    source_from_memory(executor, &preloaded[idx].0, text);
                }
            }
            idx += 1;
        }
    }

    // Re-check RCS
    if !executor.options.get("rcs").copied().unwrap_or(true) {
        return;
    }

    // Phase 4: login files (after zshrc)
    if is_login {
        // /etc/zlogin
        if idx < preloaded.len() {
            if executor.options.get("globalrcs").copied().unwrap_or(true) {
                if let Some(ref text) = preloaded[idx].1 {
                    source_from_memory(executor, &preloaded[idx].0, text);
                }
            }
            idx += 1;
        }
        // $ZDOTDIR/.zlogin
        if idx < preloaded.len() {
            if executor.options.get("rcs").copied().unwrap_or(true) {
                if let Some(ref text) = preloaded[idx].1 {
                    source_from_memory(executor, &preloaded[idx].0, text);
                }
            }
            // suppress unused assignment warning
            let _ = idx;
        }
    }
}

/// Execute a startup file from pre-read memory contents.
/// Mirrors source_file() logic but skips the fs::read_to_string.
fn source_from_memory(executor: &mut ShellExecutor, path: &PathBuf, contents: &str) {
    tracing::trace!(path = %path.display(), "sourcing startup file from memory");
    let mut buffer = String::new();
    let mut in_multiline = false;

    for line in contents.lines() {
        let trimmed = line.trim();

        // Skip empty lines and comments (unless in multiline)
        if !in_multiline && (trimmed.is_empty() || trimmed.starts_with('#')) {
            continue;
        }

        // Check for line continuation
        if line.ends_with('\\') {
            buffer.push_str(&line[..line.len() - 1]);
            buffer.push(' ');
            in_multiline = true;
            continue;
        }

        // Check for unclosed constructs (heredoc, quotes, braces)
        if in_multiline {
            buffer.push_str(line);
            let open_braces = buffer.matches('{').count();
            let close_braces = buffer.matches('}').count();
            let open_parens = buffer.matches('(').count();
            let close_parens = buffer.matches(')').count();

            if open_braces == close_braces && open_parens == close_parens {
                process_line(&buffer, executor);
                buffer.clear();
                in_multiline = false;
            } else {
                buffer.push('\n');
            }
        } else {
            process_line(line, executor);
        }
    }

    // Process any remaining buffered content
    if !buffer.is_empty() {
        process_line(&buffer, executor);
    }
}

/// Source a file
fn source_file_with_zwc(executor: &mut ShellExecutor, path: &PathBuf) {
    source_file(executor, path);
}

/// Source a single file, handling multi-line constructs
fn source_file(executor: &mut ShellExecutor, path: &PathBuf) {
    if !path.exists() {
        return;
    }

    if let Ok(contents) = std::fs::read_to_string(path) {
        let mut buffer = String::new();
        let mut in_multiline = false;

        for line in contents.lines() {
            let trimmed = line.trim();

            // Skip empty lines and comments (unless in multiline)
            if !in_multiline && (trimmed.is_empty() || trimmed.starts_with('#')) {
                continue;
            }

            // Check for line continuation
            if line.ends_with('\\') {
                buffer.push_str(&line[..line.len() - 1]);
                buffer.push(' ');
                in_multiline = true;
                continue;
            }

            // Check for unclosed constructs (heredoc, quotes, braces)
            if in_multiline {
                buffer.push_str(line);
                // Simple heuristic: if we have balanced braces/quotes, execute
                let open_braces = buffer.matches('{').count();
                let close_braces = buffer.matches('}').count();
                let open_parens = buffer.matches('(').count();
                let close_parens = buffer.matches(')').count();

                if open_braces == close_braces && open_parens == close_parens {
                    process_line(&buffer, executor);
                    buffer.clear();
                    in_multiline = false;
                } else {
                    buffer.push('\n');
                }
            } else {
                process_line(line, executor);
            }
        }

        // Process any remaining buffered content
        if !buffer.is_empty() {
            process_line(&buffer, executor);
        }
    }
}

/// Source logout files when shell exits (per zshall(1))
/// Only for login shells, respects RCS and GLOBAL_RCS options
#[allow(dead_code)]
fn source_logout_files(executor: &mut ShellExecutor, is_login: bool) {
    if !is_login {
        return;
    }

    // Check RCS option
    if !executor.options.get("rcs").copied().unwrap_or(true) {
        return;
    }

    let zdotdir = get_zdotdir();

    // $ZDOTDIR/.zlogout first
    source_file_with_zwc(executor, &zdotdir.join(".zlogout"));

    // /etc/zlogout (only if GLOBAL_RCS is set)
    if executor.options.get("globalrcs").copied().unwrap_or(true) {
        source_file_with_zwc(executor, &PathBuf::from("/etc/zlogout"));
    }
}

fn run_interactive() {
    tracing::info!("interactive mode starting");
    // Set up signal handling
    let interrupted = Arc::new(AtomicBool::new(false));
    let i = interrupted.clone();
    ctrlc::set_handler(move || {
        i.store(true, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    // Initialize fish-style abbreviations
    init_default_abbreviations();

    // Initialize compsys cache (single SQLite db for all completions)
    let cache_path = dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("zshrs/compsys.db");
    if let Some(parent) = cache_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let compsys_cache = match CompsysCache::open(&cache_path) {
        Ok(mut cache) => {
            // Index PATH executables on first run
            if !cache.has_executables().unwrap_or(false) {
                eprint!("Indexing completions... ");
                let path_var = std::env::var("PATH").unwrap_or_default();
                let mut executables = Vec::new();
                for dir in path_var.split(':') {
                    if let Ok(entries) = std::fs::read_dir(dir) {
                        for entry in entries.flatten() {
                            if let Ok(ft) = entry.file_type() {
                                if ft.is_file() || ft.is_symlink() {
                                    if let Some(name) = entry.file_name().to_str() {
                                        executables.push((name.to_string(), dir.to_string()));
                                    }
                                }
                            }
                        }
                    }
                }
                let cmd_count = executables.len();
                let _ = cache.set_executables_bulk(&executables);
                eprintln!("{} commands indexed", cmd_count);
            }
            Some(cache)
        }
        Err(e) => {
            eprintln!("Warning: compsys cache failed to initialize: {e}");
            None
        }
    };

    // Initialize SQLite history engine for frequency tracking
    let history_engine: Option<std::sync::Arc<std::sync::Mutex<HistoryEngine>>> =
        match HistoryEngine::new() {
            Ok(engine) => {
                let count = engine.count().unwrap_or(0);
                if count > 0 {
                    tracing::info!(entries = count, "history loaded");
                }
                Some(std::sync::Arc::new(std::sync::Mutex::new(engine)))
            }
            Err(e) => {
                eprintln!("Warning: history engine failed to initialize: {e}");
                None
            }
        };

    let line_editor = setup_editor(compsys_cache.map(|c| (c, cache_path)));
    if line_editor.is_none() {
        eprintln!("Failed to initialize line editor");
        return;
    }
    let mut line_editor = line_editor.unwrap();

    let mut executor = ShellExecutor::new();
    executor.zsh_compat = is_zsh_mode();
    if is_posix_mode() { executor.enter_posix_mode(); }

    // Determine shell type from invocation per zshall(1)
    let args: Vec<String> = std::env::args().collect();

    // -f: don't source startup files (except /etc/zshenv which is ALWAYS read)
    let no_rcs = args.iter().any(|a| a == "-f" || a == "--no-rcs");

    // Login shell detection:
    // - explicit -l or --login flag
    // - invoked as -zshrs (name starts with -)
    // - $SHELL ends with zshrs (login shell)
    let is_login = args.iter().any(|a| a == "-l" || a == "--login")
        || args.first().map(|a| a.starts_with('-')).unwrap_or(false)
        || std::env::var("SHELL")
            .map(|s| s.ends_with("zshrs"))
            .unwrap_or(false);

    let is_interactive = true; // We're in run_interactive()

    // Set default options (RCS and GLOBAL_RCS are on by default)
    executor.options.insert("rcs".to_string(), true);
    executor.options.insert("globalrcs".to_string(), true);

    // Source startup files in correct zsh order per zshall(1)
    source_startup_files(&mut executor, is_login, is_interactive, no_rcs);

    println!("zshrs {}", env!("CARGO_PKG_VERSION"));
    println!("Type exit to quit\n");

    loop {
        // Non-blocking: merge background compinit results if ready
        executor.drain_compinit_bg();

        let prompt = ZshrsPrompt::new(&executor);
        match line_editor.read_line(&prompt) {
            Ok(Signal::Success(line)) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                if line == "exit" || line == "logout" {
                    break;
                }

                let start = Instant::now();
                process_line(line, &mut executor);
                let duration = start.elapsed().as_millis() as i64;

                // Ship history write to worker pool — prompt returns instantly,
                // SQLite write happens in background.
                if let Some(ref engine) = history_engine {
                    let engine = std::sync::Arc::clone(engine);
                    let line = line.to_string();
                    let cwd = std::env::current_dir()
                        .ok()
                        .map(|p| p.to_string_lossy().to_string());
                    let status = executor.last_status;
                    executor.worker_pool.submit(move || {
                        if let Ok(eng) = engine.lock() {
                            if let Ok(id) = eng.add(&line, cwd.as_deref()) {
                                let _ = eng.update_last(id, duration, status);
                            }
                        }
                    });
                }
            }
            Ok(Signal::CtrlD) => {
                // EOF - exit shell
                executor.run_trap("EXIT");
                println!();
                break;
            }
            Ok(Signal::CtrlC) => {
                // Interrupt - run INT trap if set, otherwise just print newline
                interrupted.store(false, Ordering::SeqCst);
                executor.run_trap("INT");
                println!();
                continue;
            }
            Ok(_) => {
                // Handle any other signals
                continue;
            }
            Err(err) => {
                eprintln!("Error: {err}");
                break;
            }
        }
    }
}

fn process_line(line: &str, executor: &mut ShellExecutor) {
    // @ prefix: dispatch to stryke if fat binary registered a handler
    if line.starts_with('@') {
        let code = line.trim_start_matches('@').trim();
        if !code.is_empty() {
            if let Some(status) = zsh::try_stryke_dispatch(code) {
                executor.last_status = status;
                return;
            }
            // No handler registered (thin binary) — treat @ as normal shell input
        }
    }

    if let Err(e) = executor.execute_script(line) {
        eprintln!("zshrs: {}", e);
    }
}

fn setup_editor(compsys_cache: Option<(CompsysCache, PathBuf)>) -> Option<Reedline> {
    let history_path = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".zshrs_history");

    let history = Box::new(FileBackedHistory::with_file(10000, history_path).ok()?);

    let mut keybindings = default_emacs_keybindings();

    // Add Tab keybinding to trigger completion menu
    keybindings.add_binding(
        KeyModifiers::NONE,
        KeyCode::Tab,
        ReedlineEvent::UntilFound(vec![
            ReedlineEvent::Menu("completion_menu".to_string()),
            ReedlineEvent::MenuNext,
        ]),
    );

    let edit_mode = Box::new(Emacs::new(keybindings));

    let mut editor = Reedline::create()
        .with_history(history)
        .with_edit_mode(edit_mode)
        .with_hinter(Box::new(
            DefaultHinter::default().with_style(AnsiStyle::new().fg(AnsiColor::DarkGray)),
        ))
        .with_highlighter(Box::new(ZshrsHighlighter))
        .with_validator(Box::new(ZshrsValidator));

    if let Some((cache, cache_path)) = compsys_cache {
        let completer = Box::new(ZshrsCompleter::new(cache, cache_path));

        // Zsh-style menuselect — port of Src/Zle/complist.c domenuselect()
        // Uses compsys MenuState for rendering (group headers, zstyle colors,
        // grid navigation with column memory, viewport scrolling).
        let completion_menu = Box::new(ZshMenuSelect::new());

        editor = editor
            .with_completer(completer)
            .with_menu(ReedlineMenu::EngineCompleter(completion_menu));
    }

    Some(editor)
}

// ============================================================================
// SYNTAX HIGHLIGHTER - Fish-style real-time highlighting
// ============================================================================

struct ZshrsHighlighter;

impl Highlighter for ZshrsHighlighter {
    fn highlight(&self, line: &str, _cursor: usize) -> StyledText {
        let specs = highlight_shell(line);
        let mut styled = StyledText::new();

        if line.is_empty() {
            return styled;
        }

        let mut current_style = AnsiStyle::new();
        let mut current_text = String::new();
        let mut last_role = HighlightRole::Normal;

        for (i, c) in line.chars().enumerate() {
            let byte_pos = line.char_indices().nth(i).map(|(p, _)| p).unwrap_or(i);
            let role = specs
                .get(byte_pos)
                .map(|s| s.foreground)
                .unwrap_or(HighlightRole::Normal);

            if role != last_role && !current_text.is_empty() {
                styled.push((current_style, current_text.clone()));
                current_text.clear();
            }

            if role != last_role {
                current_style = role_to_style(role);
                last_role = role;
            }

            current_text.push(c);
        }

        if !current_text.is_empty() {
            styled.push((current_style, current_text));
        }

        styled
    }
}

fn role_to_style(role: HighlightRole) -> AnsiStyle {
    match role {
        HighlightRole::Normal => AnsiStyle::new(),
        HighlightRole::Command => AnsiStyle::new().fg(AnsiColor::Green).bold(),
        HighlightRole::Keyword => AnsiStyle::new().fg(AnsiColor::Blue).bold(),
        HighlightRole::Statement => AnsiStyle::new().fg(AnsiColor::Magenta).bold(),
        HighlightRole::Param => AnsiStyle::new(),
        HighlightRole::Option => AnsiStyle::new().fg(AnsiColor::Cyan),
        HighlightRole::Comment => AnsiStyle::new().fg(AnsiColor::DarkGray),
        HighlightRole::Error => AnsiStyle::new().fg(AnsiColor::Red).bold(),
        HighlightRole::String => AnsiStyle::new().fg(AnsiColor::Yellow),
        HighlightRole::Escape => AnsiStyle::new().fg(AnsiColor::Yellow).bold(),
        HighlightRole::Operator => AnsiStyle::new().fg(AnsiColor::White).bold(),
        HighlightRole::Redirection => AnsiStyle::new().fg(AnsiColor::Magenta),
        HighlightRole::Path => AnsiStyle::new().underline(),
        HighlightRole::PathValid => AnsiStyle::new().fg(AnsiColor::Green).underline(),
        HighlightRole::Autosuggestion => AnsiStyle::new().fg(AnsiColor::DarkGray),
        HighlightRole::Selection => AnsiStyle::new().reverse(),
        HighlightRole::Search => AnsiStyle::new().fg(AnsiColor::Black).on(AnsiColor::Yellow),
        HighlightRole::Variable => AnsiStyle::new().fg(AnsiColor::Cyan).bold(),
        HighlightRole::Quote => AnsiStyle::new().fg(AnsiColor::Yellow),
    }
}

// ============================================================================
// VALIDATOR - Multi-line support for incomplete commands
// ============================================================================

struct ZshrsValidator;

impl Validator for ZshrsValidator {
    fn validate(&self, line: &str) -> ValidationResult {
        match validate_command(line) {
            ValidationStatus::Valid => ValidationResult::Complete,
            ValidationStatus::Incomplete => ValidationResult::Incomplete,
            ValidationStatus::Invalid(_) => ValidationResult::Complete, // Let execution show the error
        }
    }
}

// ============================================================================
// ZSH MENUSELECT — Port of Src/Zle/complist.c domenuselect()
// ============================================================================

/// Reedline Menu backed by compsys MenuState.
///
/// Bridges reedline's MenuEvent dispatch to our full zsh-style menuselect
/// with group headers, zstyle colors, grid navigation with column memory,
/// and proper viewport scrolling.
struct ZshMenuSelect {
    settings: MenuSettings,
    active: bool,
    /// compsys menu state — the real engine
    state: MenuState,
    /// Cached reedline suggestions (kept for get_values/replace_in_buffer)
    values: Vec<Suggestion>,
    event: Option<MenuEvent>,
    /// Whether values have been loaded into MenuState
    loaded: bool,
}

impl ZshMenuSelect {
    fn new() -> Self {
        Self {
            settings: MenuSettings::default().with_name("completion_menu"),
            active: false,
            state: MenuState::new(),
            values: Vec::new(),
            event: None,
            loaded: false,
        }
    }

    /// Convert reedline Suggestions to compsys CompletionGroup and load into MenuState
    fn load_suggestions(&mut self, terminal_width: u16) {
        if self.values.is_empty() || self.loaded {
            return;
        }

        // Group suggestions by their extra[0] tag (e.g. "command", "file", "option")
        let mut groups: std::collections::HashMap<String, Vec<compsys::Completion>> =
            std::collections::HashMap::new();

        for sugg in &self.values {
            let group_name = sugg
                .extra
                .as_ref()
                .and_then(|e| e.first())
                .cloned()
                .unwrap_or_else(|| "completions".to_string());

            let mut comp = compsys::Completion::new(&sugg.value);
            if let Some(ref desc) = sugg.description {
                comp.desc = Some(desc.clone());
            }
            groups.entry(group_name).or_default().push(comp);
        }

        let mut comp_groups = Vec::new();
        for (name, matches) in groups {
            let mut g = CompletionGroup::new(&name);
            g.matches = matches;
            comp_groups.push(g);
        }

        self.state.set_term_size(terminal_width as usize, 24);
        self.state.set_completions(&comp_groups);
        self.state.start();
        self.loaded = true;
    }

    fn index(&self) -> usize {
        self.state.selected_index().unwrap_or(0)
    }
}

impl MenuBuilder for ZshMenuSelect {
    fn settings_mut(&mut self) -> &mut MenuSettings {
        &mut self.settings
    }
}

impl ReedlineMenuTrait for ZshMenuSelect {
    fn settings(&self) -> &MenuSettings {
        &self.settings
    }

    fn is_active(&self) -> bool {
        self.active
    }

    fn can_quick_complete(&self) -> bool {
        true
    }

    fn can_partially_complete(
        &mut self,
        values_updated: bool,
        editor: &mut Editor,
        completer: &mut dyn Completer,
    ) -> bool {
        if !values_updated {
            self.update_values(editor, completer);
        }
        menu_functions::can_partially_complete(self.get_values(), editor)
    }

    fn menu_event(&mut self, event: MenuEvent) {
        match &event {
            MenuEvent::Activate(_) => {
                self.active = true;
                self.loaded = false;
            }
            MenuEvent::Deactivate => {
                self.active = false;
                self.loaded = false;
                self.state.stop();
            }
            _ => {}
        }
        self.event = Some(event);
    }

    fn update_values(
        &mut self,
        editor: &mut Editor,
        completer: &mut dyn Completer,
    ) {
        let (input, pos) = menu_functions::completer_input(
            editor.get_buffer(),
            editor.line_buffer().insertion_point(),
            None,
            false,
        );
        let (values, _) = completer.complete_with_base_ranges(&input, pos);
        self.values = values;
        self.loaded = false;
    }

    fn update_working_details(
        &mut self,
        editor: &mut Editor,
        completer: &mut dyn Completer,
        painter: &Painter,
    ) {
        if let Some(event) = self.event.take() {
            match event {
                MenuEvent::Activate(updated) => {
                    if !updated {
                        self.update_values(editor, completer);
                    }
                    self.load_suggestions(painter.screen_width());
                }
                MenuEvent::Deactivate => {}
                MenuEvent::Edit(updated) => {
                    if !updated {
                        self.update_values(editor, completer);
                    }
                    self.loaded = false;
                    self.load_suggestions(painter.screen_width());
                }
                MenuEvent::NextElement => {
                    self.load_suggestions(painter.screen_width());
                    let _ = self
                        .state
                        .process_action(compsys::MenuAction::Next);
                }
                MenuEvent::PreviousElement => {
                    self.load_suggestions(painter.screen_width());
                    let _ = self
                        .state
                        .process_action(compsys::MenuAction::Prev);
                }
                MenuEvent::MoveUp => {
                    self.load_suggestions(painter.screen_width());
                    let _ = self.state.process_action(compsys::MenuAction::Up);
                }
                MenuEvent::MoveDown => {
                    self.load_suggestions(painter.screen_width());
                    let _ = self
                        .state
                        .process_action(compsys::MenuAction::Down);
                }
                MenuEvent::MoveLeft => {
                    self.load_suggestions(painter.screen_width());
                    let _ = self
                        .state
                        .process_action(compsys::MenuAction::Left);
                }
                MenuEvent::MoveRight => {
                    self.load_suggestions(painter.screen_width());
                    let _ = self
                        .state
                        .process_action(compsys::MenuAction::Right);
                }
                MenuEvent::NextPage => {
                    self.load_suggestions(painter.screen_width());
                    let _ = self
                        .state
                        .process_action(compsys::MenuAction::PageDown);
                }
                MenuEvent::PreviousPage => {
                    self.load_suggestions(painter.screen_width());
                    let _ = self
                        .state
                        .process_action(compsys::MenuAction::PageUp);
                }
            }
        }
    }

    fn replace_in_buffer(&self, editor: &mut Editor) {
        let value = self.get_values().get(self.index()).cloned();
        menu_functions::replace_in_buffer(value, editor);
    }

    fn min_rows(&self) -> u16 {
        3
    }

    fn get_values(&self) -> &[Suggestion] {
        &self.values
    }

    fn menu_required_lines(&self, _terminal_columns: u16) -> u16 {
        // Estimate from item count and columns
        let cols = self.state.cols().max(1);
        let rows = (self.values.len() + cols - 1) / cols;
        (rows as u16).max(3)
    }

    fn menu_string(&self, available_lines: u16, _use_ansi_coloring: bool) -> String {
        // Use a mutable clone for rendering (Menu trait takes &self)
        let mut state = self.state.clone();
        state.set_available_rows(available_lines as usize);
        let rendering = state.render();

        let mut output = String::new();
        for (i, line) in rendering.lines.iter().enumerate() {
            if i > 0 {
                output.push_str("\r\n");
            }
            output.push_str(&line.content);
        }
        if rendering.lines.is_empty() {
            output.push_str("NO RECORDS FOUND");
        }
        output
    }
}

// ============================================================================
// COMPLETER
// ============================================================================

struct ZshrsCompleter {
    cache: CompsysCache,
    /// Path to the SQLite cache file — needed by completion threads that open
    /// their own read-only connections to avoid Send issues with rusqlite.
    cache_path: PathBuf,
    #[allow(dead_code)]
    comp_state: CompletionState,
}

impl ZshrsCompleter {
    fn new(mut cache: CompsysCache, cache_path: PathBuf) -> Self {
        // Check if completion mappings need to be built
        let (valid, count) = compinit_lazy(&cache);
        if !valid || count == 0 {
            // Build cache from fpath
            let fpath = get_system_fpath();
            let _ = build_cache_from_fpath(&fpath, &mut cache);
        }

        Self {
            cache,
            cache_path,
            comp_state: CompletionState::new(),
        }
    }

    /// Get completions for command options using compsys
    #[allow(dead_code)]
    fn complete_options(&mut self, cmd: &str, prefix: &str) -> Vec<CompsysCompletion> {
        // Use do_completion with a closure that adds known options
        let line = format!("{} {}", cmd, prefix);
        let cursor = line.len();

        self.comp_state = CompletionState::from_line(&line, cursor);

        let _nmatches = do_completion(&line, cursor, &mut self.comp_state, |state| {
            state.begin_group("options", true);

            match cmd {
                "ls" => {
                    for (opt, desc) in &[
                        ("-l", "long listing format"),
                        ("-a", "show hidden files"),
                        ("-h", "human readable sizes"),
                        ("-R", "recursive"),
                        ("-t", "sort by time"),
                        ("-S", "sort by size"),
                        ("-r", "reverse order"),
                        ("-1", "one entry per line"),
                        ("-d", "list directories themselves"),
                        ("-F", "append indicator"),
                        ("--color", "colorize output"),
                        ("--help", "display help"),
                    ] {
                        if opt.starts_with(prefix) || prefix.is_empty() {
                            let mut comp = CompsysCompletion::new(*opt);
                            comp.desc = Some(desc.to_string());
                            state.add_match(comp, Some("options"));
                        }
                    }
                }
                "git" => {
                    for (opt, desc) in &[
                        ("--version", "show version"),
                        ("--help", "show help"),
                        ("-C", "run as if started in path"),
                        ("-c", "pass configuration parameter"),
                        ("--exec-path", "path to git executables"),
                        ("--work-tree", "set working tree"),
                        ("--git-dir", "set git directory"),
                    ] {
                        if opt.starts_with(prefix) || prefix.is_empty() {
                            let mut comp = CompsysCompletion::new(*opt);
                            comp.desc = Some(desc.to_string());
                            state.add_match(comp, Some("options"));
                        }
                    }
                }
                "grep" | "rg" => {
                    for (opt, desc) in &[
                        ("-i", "ignore case"),
                        ("-v", "invert match"),
                        ("-r", "recursive"),
                        ("-n", "line numbers"),
                        ("-l", "files with matches"),
                        ("-c", "count matches"),
                        ("-E", "extended regex"),
                        ("-w", "whole words"),
                        ("-A", "lines after"),
                        ("-B", "lines before"),
                        ("-C", "lines context"),
                        ("--color", "colorize output"),
                    ] {
                        if opt.starts_with(prefix) || prefix.is_empty() {
                            let mut comp = CompsysCompletion::new(*opt);
                            comp.desc = Some(desc.to_string());
                            state.add_match(comp, Some("options"));
                        }
                    }
                }
                "cargo" => {
                    for (opt, desc) in &[
                        ("--help", "show help"),
                        ("--version", "show version"),
                        ("-V", "show version"),
                        ("-v", "verbose"),
                        ("-q", "quiet"),
                        ("--color", "colorize output"),
                        ("--frozen", "require lockfile up to date"),
                        ("--locked", "require lockfile matches"),
                        ("--offline", "run without network"),
                    ] {
                        if opt.starts_with(prefix) || prefix.is_empty() {
                            let mut comp = CompsysCompletion::new(*opt);
                            comp.desc = Some(desc.to_string());
                            state.add_match(comp, Some("options"));
                        }
                    }
                }
                _ => {
                    // Generic options for unknown commands
                    for opt in &["--help", "--version", "-h", "-v"] {
                        if opt.starts_with(prefix) || prefix.is_empty() {
                            state.add_match(CompsysCompletion::new(*opt), Some("options"));
                        }
                    }
                }
            }

            state.end_group();
        });

        // Collect all completions from groups
        let mut result = Vec::new();
        for group in &self.comp_state.groups {
            for comp in &group.matches {
                result.push(comp.clone());
            }
        }
        result
    }
}

impl Completer for ZshrsCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> Vec<Suggestion> {
        let line_to_pos = &line[..pos];
        let word_start = line_to_pos
            .rfind(|c: char| c.is_whitespace())
            .map(|i| i + 1)
            .unwrap_or(0);
        let current_word = &line_to_pos[word_start..];
        let current_word = current_word.trim_start_matches('@');

        let words: Vec<&str> = line_to_pos.split_whitespace().collect();
        let is_first_word = words.len() <= 1 && !line_to_pos.ends_with(' ');

        if is_first_word {
            // Command position — launch executable, builtin, and function lookups
            // on separate threads to overlap SQLite I/O and string matching.
            if current_word.is_empty() {
                return Vec::new();
            }

            let prefix = current_word.to_string();
            let prefix_lower = current_word.to_lowercase();
            let ws = word_start;
            let p = pos;

            // --- Thread 1: executables from SQLite FTS cache ---
            let cache_path = self.cache_path.clone();
            let prefix_exec = prefix.clone();
            let exec_handle = std::thread::spawn(move || -> Vec<Suggestion> {
                let mut results = Vec::new();
                if let Ok(cache) = compsys::cache::CompsysCache::open(&cache_path) {
                    if let Ok(executables) = cache.get_executables_prefix_fts(&prefix_exec) {
                        for (name, path) in executables.into_iter().take(100) {
                            results.push(Suggestion {
                                value: name,
                                description: Some(path),
                                style: None,
                                extra: Some(vec!["command".to_string()]),
                                span: Span::new(ws, p),
                                append_whitespace: true,
                                display_override: None,
                                match_indices: None,
                            });
                        }
                    }
                }
                results
            });

            // --- Thread 2: builtin matching (pure CPU, fast) ---
            let prefix_builtin = prefix_lower.clone();
            let builtin_handle = std::thread::spawn(move || -> Vec<Suggestion> {
                let builtins = [
                    "alias", "autoload", "bg", "bindkey", "break", "builtin", "cd",
                    "command", "compctl", "continue", "declare", "dirs", "disown",
                    "echo", "emulate", "enable", "eval", "exec", "exit", "export",
                    "false", "fc", "fg", "float", "functions", "getopts", "hash",
                    "history", "integer", "jobs", "kill", "let", "limit", "local",
                    "log", "logout", "noglob", "popd", "print", "printf", "pushd",
                    "pwd", "read", "readonly", "rehash", "return", "set", "setopt",
                    "shift", "source", "suspend", "test", "times", "trap", "true",
                    "type", "typeset", "ulimit", "umask", "unalias", "unfunction",
                    "unhash", "unlimit", "unset", "unsetopt", "wait", "whence",
                    "where", "which", "zcompile", "zformat", "zle", "zmodload",
                    "zparseopts", "zprof", "zpty", "zregexparse", "zsocket", "zstat",
                    "zstyle",
                ];
                let mut results = Vec::new();
                for builtin in builtins {
                    if builtin.starts_with(&prefix_builtin) {
                        results.push(Suggestion {
                            value: builtin.to_string(),
                            description: Some("builtin".to_string()),
                            style: None,
                            extra: Some(vec!["builtin".to_string()]),
                            span: Span::new(ws, p),
                            append_whitespace: true,
                            display_override: None,
                            match_indices: None,
                        });
                    }
                }
                results
            });

            // --- Thread 3: shell functions from SQLite cache ---
            let cache_path2 = self.cache_path.clone();
            let prefix_func = prefix.clone();
            let func_handle = std::thread::spawn(move || -> Vec<Suggestion> {
                let mut results = Vec::new();
                if let Ok(cache) = compsys::cache::CompsysCache::open(&cache_path2) {
                    if let Ok(funcs) = cache.get_shell_functions_prefix(&prefix_func) {
                        for (name, source) in funcs.into_iter().take(50) {
                            results.push(Suggestion {
                                value: name,
                                description: Some(source),
                                style: None,
                                extra: Some(vec!["function".to_string()]),
                                span: Span::new(ws, p),
                                append_whitespace: true,
                                display_override: None,
                                match_indices: None,
                            });
                        }
                    }
                }
                results
            });

            // Collect results from all threads
            let mut suggestions = Vec::new();
            if let Ok(mut exec_results) = exec_handle.join() {
                suggestions.append(&mut exec_results);
            }
            if let Ok(mut builtin_results) = builtin_handle.join() {
                suggestions.append(&mut builtin_results);
            }
            if let Ok(mut func_results) = func_handle.join() {
                suggestions.append(&mut func_results);
            }

            tracing::trace!(
                count = suggestions.len(),
                prefix = %prefix,
                "parallel command completion complete"
            );

            suggestions.sort_by(|a, b| a.value.cmp(&b.value));
            suggestions.dedup_by(|a, b| a.value == b.value);
            return suggestions;
        }

        let mut suggestions = Vec::new();

        if current_word.starts_with('-') {
            // Option completion - use compsys cache to find options
            if let Some(cmd) = words.first() {
                // Try to get options from completion function in cache
                if let Ok(Some(func)) = self.cache.get_comp(*cmd) {
                    if let Ok(Some(stub)) = self.cache.get_autoload(&func) {
                        if let Ok(content) = std::fs::read_to_string(&stub.source) {
                            let prefix_lower = current_word.to_lowercase();
                            for line in content.lines() {
                                let line = line.trim();
                                if !line.contains('[') || line.starts_with('#') {
                                    continue;
                                }
                                for segment in line.split('\'') {
                                    if let Some((opt, desc)) = parse_option_spec(segment) {
                                        if opt.to_lowercase().starts_with(&prefix_lower) {
                                            suggestions.push(Suggestion {
                                                value: opt,
                                                description: if desc.is_empty() {
                                                    None
                                                } else {
                                                    Some(desc)
                                                },
                                                style: None,
                                                extra: Some(vec!["option".to_string()]),
                                                span: Span::new(word_start, pos),
                                                append_whitespace: true,
                                                display_override: None,
                                                match_indices: None,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Fallback: hardcoded options for common commands
                if suggestions.is_empty() {
                    let options: Vec<(&str, &str)> = match *cmd {
                        "ls" => vec![
                            ("-l", "long listing"),
                            ("-a", "show hidden"),
                            ("-h", "human sizes"),
                            ("-R", "recursive"),
                            ("-t", "sort by time"),
                            ("-S", "sort by size"),
                            ("-r", "reverse"),
                            ("-1", "one per line"),
                            ("-d", "directories"),
                            ("-F", "indicators"),
                            ("--color", "colorize"),
                            ("--help", "help"),
                        ],
                        "git" => vec![
                            ("--version", "version"),
                            ("--help", "help"),
                            ("-C", "path"),
                            ("-c", "config"),
                        ],
                        "grep" | "rg" => vec![
                            ("-i", "ignore case"),
                            ("-v", "invert"),
                            ("-r", "recursive"),
                            ("-n", "line numbers"),
                            ("-l", "files only"),
                            ("-c", "count"),
                        ],
                        "cargo" => vec![
                            ("--help", "help"),
                            ("--version", "version"),
                            ("-v", "verbose"),
                            ("-q", "quiet"),
                        ],
                        "cd" => vec![
                            ("-", "previous"),
                            ("-L", "follow symlinks"),
                            ("-P", "physical"),
                        ],
                        _ => vec![
                            ("--help", "help"),
                            ("--version", "version"),
                            ("-h", "help"),
                            ("-v", "verbose"),
                        ],
                    };
                    for (opt, desc) in options {
                        if opt.starts_with(current_word) {
                            suggestions.push(Suggestion {
                                value: opt.to_string(),
                                description: Some(desc.to_string()),
                                style: None,
                                extra: Some(vec!["option".to_string()]),
                                span: Span::new(word_start, pos),
                                append_whitespace: true,
                                display_override: None,
                                match_indices: None,
                            });
                        }
                    }
                }
            }
        } else {
            // Argument position - complete files
            let (dir, file_prefix) = if current_word.contains('/') {
                let idx = current_word.rfind('/').unwrap();
                let dir = if idx == 0 { "/" } else { &current_word[..idx] };
                (dir.to_string(), &current_word[idx + 1..])
            } else {
                (".".to_string(), current_word)
            };

            let dir_path = if dir.starts_with('~') {
                dirs::home_dir()
                    .map(|h| dir.replacen('~', &h.to_string_lossy(), 1))
                    .unwrap_or(dir.clone())
            } else {
                dir.clone()
            };

            if let Ok(entries) = std::fs::read_dir(&dir_path) {
                let prefix_lower = file_prefix.to_lowercase();
                for entry in entries.take(100).flatten() {
                    if let Some(name) = entry.file_name().to_str() {
                        if name.to_lowercase().starts_with(&prefix_lower) || file_prefix.is_empty()
                        {
                            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                            let display = if dir == "." {
                                name.to_string()
                            } else if dir.ends_with('/') {
                                format!("{}{}", dir, name)
                            } else {
                                format!("{}/{}", dir, name)
                            };
                            let value = if is_dir {
                                format!("{}/", display)
                            } else {
                                display
                            };
                            suggestions.push(Suggestion {
                                value,
                                description: if is_dir {
                                    Some("directory".to_string())
                                } else {
                                    None
                                },
                                style: None,
                                extra: Some(vec!["file".to_string()]),
                                span: Span::new(word_start, pos),
                                append_whitespace: !is_dir,
                                display_override: None,
                                match_indices: None,
                            });
                        }
                    }
                }
            }
        }

        // Deduplicate by value
        suggestions.sort_by(|a, b| a.value.cmp(&b.value));
        suggestions.dedup_by(|a, b| a.value == b.value);
        suggestions
    }
}

fn parse_option_spec(spec: &str) -> Option<(String, String)> {
    let spec = spec.trim();
    if !spec.contains('-') {
        return None;
    }
    let opt_start = if spec.starts_with('(') {
        spec.find(')')?.checked_add(1)?
    } else {
        0
    };
    let rest = &spec[opt_start..];
    if !rest.starts_with('-') {
        return None;
    }
    let opt_end = rest
        .find(|c| c == '[' || c == '=' || c == ':' || c == ' ')
        .unwrap_or(rest.len());
    let opt_name = rest[..opt_end].trim_end_matches(|c| c == '+' || c == '=');
    if opt_name.is_empty() || opt_name == "-" || opt_name == "--" {
        return None;
    }
    let desc = if let Some(bracket_start) = rest.find('[') {
        if let Some(bracket_end) = rest[bracket_start..].find(']') {
            rest[bracket_start + 1..bracket_start + bracket_end].to_string()
        } else {
            String::new()
        }
    } else {
        String::new()
    };
    Some((opt_name.to_string(), desc))
}

/// Custom prompt that supports PS1/PROMPT with zsh escape sequences
struct ZshrsPrompt {
    left_prompt: String,
    right_prompt: String,
}

impl ZshrsPrompt {
    fn new(executor: &ShellExecutor) -> Self {
        // Check for PS1 or PROMPT (zsh uses PROMPT, bash uses PS1)
        let prompt_str = executor
            .variables
            .get("PROMPT")
            .or_else(|| executor.variables.get("PS1"))
            .cloned()
            .or_else(|| env::var("PROMPT").ok())
            .or_else(|| env::var("PS1").ok())
            .unwrap_or_else(|| "%n@%m %1~ %# ".to_string());

        // Check for RPROMPT (right prompt, zsh feature)
        let rprompt_str = executor
            .variables
            .get("RPROMPT")
            .cloned()
            .or_else(|| env::var("RPROMPT").ok())
            .unwrap_or_default();

        let left_prompt = expand_prompt_escapes(&prompt_str, executor);
        let right_prompt = expand_prompt_escapes(&rprompt_str, executor);

        Self {
            left_prompt,
            right_prompt,
        }
    }
}

impl Prompt for ZshrsPrompt {
    fn render_prompt_left(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed(&self.left_prompt)
    }

    fn render_prompt_right(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed(&self.right_prompt)
    }

    fn render_prompt_indicator(
        &self,
        _edit_mode: reedline::PromptEditMode,
    ) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("")
    }

    fn render_prompt_multiline_indicator(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("> ")
    }

    fn render_prompt_history_search_indicator(
        &self,
        history_search: PromptHistorySearch,
    ) -> std::borrow::Cow<'_, str> {
        let prefix = match history_search.status {
            PromptHistorySearchStatus::Passing => "",
            PromptHistorySearchStatus::Failing => "failing ",
        };
        std::borrow::Cow::Owned(format!(
            "({}reverse-i-search)`{}': ",
            prefix, history_search.term
        ))
    }

    fn get_prompt_color(&self) -> Color {
        Color::Green
    }

    fn get_indicator_color(&self) -> Color {
        Color::Cyan
    }

    fn get_prompt_right_color(&self) -> Color {
        Color::AnsiValue(5)
    }

    fn right_prompt_on_last_line(&self) -> bool {
        true
    }
}

fn expand_prompt_escapes(prompt: &str, executor: &ShellExecutor) -> String {
    let mut result = String::new();
    let mut chars = prompt.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.next() {
                Some('n') => {
                    // Username
                    result.push_str(&env::var("USER").unwrap_or_else(|_| "user".to_string()));
                }
                Some('m') => {
                    // Hostname (short)
                    result.push_str(
                        &hostname::get()
                            .map(|h| {
                                h.to_string_lossy()
                                    .split('.')
                                    .next()
                                    .unwrap_or("localhost")
                                    .to_string()
                            })
                            .unwrap_or_else(|_| "localhost".to_string()),
                    );
                }
                Some('M') => {
                    // Hostname (full)
                    result.push_str(
                        &hostname::get()
                            .map(|h| h.to_string_lossy().to_string())
                            .unwrap_or_else(|_| "localhost".to_string()),
                    );
                }
                Some('~') | Some('d') => {
                    // Current directory (~ for home)
                    let cwd = env::current_dir()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|_| "?".to_string());
                    if let Some(home) = dirs::home_dir() {
                        let home_str = home.to_string_lossy();
                        if cwd.starts_with(home_str.as_ref()) {
                            result.push('~');
                            result.push_str(&cwd[home_str.len()..]);
                        } else {
                            result.push_str(&cwd);
                        }
                    } else {
                        result.push_str(&cwd);
                    }
                }
                Some('/') => {
                    // Current directory (full path)
                    result.push_str(
                        &env::current_dir()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_else(|_| "?".to_string()),
                    );
                }
                Some('1') | Some('c') | Some('C') => {
                    // Trailing component of current directory
                    let cwd = env::current_dir()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|_| "?".to_string());
                    if let Some(name) = PathBuf::from(&cwd).file_name() {
                        result.push_str(&name.to_string_lossy());
                    } else {
                        result.push('/');
                    }
                }
                Some('#') | Some('%') => {
                    // # if root, % otherwise
                    let is_root = env::var("EUID")
                        .or_else(|_| env::var("UID"))
                        .map(|uid| uid == "0")
                        .unwrap_or(false);
                    if is_root {
                        result.push('#');
                    } else {
                        result.push('%');
                    }
                }
                Some('?') => {
                    // Exit status of last command
                    result.push_str(&executor.last_status.to_string());
                }
                Some('j') => {
                    // Number of jobs
                    result.push_str(&executor.jobs.count().to_string());
                }
                Some('T') => {
                    // Current time in 12-hour format
                    let now = chrono::Local::now();
                    result.push_str(&now.format("%I:%M").to_string());
                }
                Some('t') | Some('@') => {
                    // Current time in 12-hour format with am/pm
                    let now = chrono::Local::now();
                    result.push_str(&now.format("%I:%M %p").to_string());
                }
                Some('*') => {
                    // Current time in 24-hour format
                    let now = chrono::Local::now();
                    result.push_str(&now.format("%H:%M").to_string());
                }
                Some('D') => {
                    // Date
                    let now = chrono::Local::now();
                    result.push_str(&now.format("%Y-%m-%d").to_string());
                }
                Some('F') => {
                    // Bold (start)
                    result.push_str("\x1b[1m");
                }
                Some('f') => {
                    // Bold (end) / reset
                    result.push_str("\x1b[0m");
                }
                Some('B') => {
                    // Bold (start, alternative)
                    result.push_str("\x1b[1m");
                }
                Some('b') => {
                    // Bold (end, alternative)
                    result.push_str("\x1b[22m");
                }
                Some('{') => {
                    // Start of literal escape sequence (ignored)
                }
                Some('}') => {
                    // End of literal escape sequence (ignored)
                }
                Some(other) => {
                    result.push('%');
                    result.push(other);
                }
                None => {
                    result.push('%');
                }
            }
        } else if c == '\\' {
            // Bash-style escapes
            match chars.next() {
                Some('u') => {
                    result.push_str(&env::var("USER").unwrap_or_else(|_| "user".to_string()));
                }
                Some('h') => {
                    result.push_str(
                        &hostname::get()
                            .map(|h| {
                                h.to_string_lossy()
                                    .split('.')
                                    .next()
                                    .unwrap_or("localhost")
                                    .to_string()
                            })
                            .unwrap_or_else(|_| "localhost".to_string()),
                    );
                }
                Some('H') => {
                    result.push_str(
                        &hostname::get()
                            .map(|h| h.to_string_lossy().to_string())
                            .unwrap_or_else(|_| "localhost".to_string()),
                    );
                }
                Some('w') => {
                    let cwd = env::current_dir()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|_| "?".to_string());
                    if let Some(home) = dirs::home_dir() {
                        let home_str = home.to_string_lossy();
                        if cwd.starts_with(home_str.as_ref()) {
                            result.push('~');
                            result.push_str(&cwd[home_str.len()..]);
                        } else {
                            result.push_str(&cwd);
                        }
                    } else {
                        result.push_str(&cwd);
                    }
                }
                Some('W') => {
                    let cwd = env::current_dir()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|_| "?".to_string());
                    if let Some(name) = PathBuf::from(&cwd).file_name() {
                        result.push_str(&name.to_string_lossy());
                    } else {
                        result.push('/');
                    }
                }
                Some('$') => {
                    let is_root = env::var("EUID")
                        .or_else(|_| env::var("UID"))
                        .map(|uid| uid == "0")
                        .unwrap_or(false);
                    if is_root {
                        result.push('#');
                    } else {
                        result.push('$');
                    }
                }
                Some('n') => {
                    result.push('\n');
                }
                Some('r') => {
                    result.push('\r');
                }
                Some('\\') => {
                    result.push('\\');
                }
                Some('[') | Some(']') => {
                    // Non-printing character markers (ignored in output)
                }
                Some(other) => {
                    result.push('\\');
                    result.push(other);
                }
                None => {
                    result.push('\\');
                }
            }
        } else {
            result.push(c);
        }
    }

    result
}
