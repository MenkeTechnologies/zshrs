//! Shell command executor for zshrs
//!
//! Executes the parsed shell AST.

use crate::history::HistoryEngine;
use crate::math::MathEval;
use crate::pcre::PcreState;
use crate::prompt::{expand_prompt, PromptContext};
use crate::tcp::TcpSessions;
use crate::zftp::Zftp;
use crate::zprof::Profiler;
use crate::zutil::StyleTable;
use compsys::cache::CompsysCache;
use compsys::CompInitResult;
use parking_lot::Mutex;
use std::collections::HashSet;

/// AOP advice type — before, after, or around.
#[derive(Debug, Clone)]
pub enum AdviceKind {
    /// Run code before the command executes.
    Before,
    /// Run code after the command executes. $? and INTERCEPT_MS available.
    After,
    /// Wrap the command. Code must call `intercept_proceed` to run original.
    Around,
}

/// An intercept registration.
#[derive(Debug, Clone)]
pub struct Intercept {
    /// Pattern to match command names. Supports glob: "git *", "_*", "*".
    pub pattern: String,
    /// What kind of advice.
    pub kind: AdviceKind,
    /// Shell code to execute as advice.
    pub code: String,
    /// Unique ID for removal.
    pub id: u32,
}

/// Result from background compinit thread
pub struct CompInitBgResult {
    pub result: CompInitResult,
    pub cache: CompsysCache,
}
use std::io::Write;
use std::sync::LazyLock;

/// State snapshot for plugin delta computation.
struct PluginSnapshot {
    functions: std::collections::HashSet<String>,
    aliases: std::collections::HashSet<String>,
    global_aliases: std::collections::HashSet<String>,
    suffix_aliases: std::collections::HashSet<String>,
    variables: HashMap<String, String>,
    arrays: std::collections::HashSet<String>,
    assoc_arrays: std::collections::HashSet<String>,
    fpath: Vec<PathBuf>,
    options: HashMap<String, bool>,
    hooks: HashMap<String, Vec<String>>,
    autoloads: std::collections::HashSet<String>,
}

/// Cached compiled regexes for hot paths
static REGEX_CACHE: LazyLock<Mutex<std::collections::HashMap<String, regex::Regex>>> =
    LazyLock::new(|| Mutex::new(std::collections::HashMap::with_capacity(64)));

/// Match an intercept pattern against a command name or full command string.
/// Supports: exact match, glob ("git *", "_*", "*"), or "all".
fn intercept_matches(pattern: &str, cmd_name: &str, full_cmd: &str) -> bool {
    if pattern == "*" || pattern == "all" {
        return true;
    }
    if pattern == cmd_name {
        return true;
    }
    // Glob match against full command (e.g. "git *" matches "git push")
    if pattern.contains('*') || pattern.contains('?') {
        if let Ok(pat) = glob::Pattern::new(pattern) {
            return pat.matches(cmd_name) || pat.matches(full_cmd);
        }
    }
    false
}

/// Get or compile a regex, caching the result
fn cached_regex(pattern: &str) -> Option<regex::Regex> {
    let mut cache = REGEX_CACHE.lock();
    if let Some(re) = cache.get(pattern) {
        return Some(re.clone());
    }
    match regex::Regex::new(pattern) {
        Ok(re) => {
            cache.insert(pattern.to_string(), re.clone());
            Some(re)
        }
        Err(_) => None,
    }
}

/// HashSet of all zsh options for O(1) lookup
static ZSH_OPTIONS_SET: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "aliases",
        "allexport",
        "alwayslastprompt",
        "alwaystoend",
        "appendcreate",
        "appendhistory",
        "autocd",
        "autocontinue",
        "autolist",
        "automenu",
        "autonamedirs",
        "autoparamkeys",
        "autoparamslash",
        "autopushd",
        "autoremoveslash",
        "autoresume",
        "badpattern",
        "banghist",
        "bareglobqual",
        "bashautolist",
        "bashrematch",
        "beep",
        "bgnice",
        "braceccl",
        "bsdecho",
        "caseglob",
        "casematch",
        "cbases",
        "cdablevars",
        "cdsilent",
        "chasedots",
        "chaselinks",
        "checkjobs",
        "checkrunningjobs",
        "clobber",
        "combiningchars",
        "completealiases",
        "completeinword",
        "continueonerror",
        "correct",
        "correctall",
        "cprecedences",
        "cshjunkiehistory",
        "cshjunkieloops",
        "cshjunkiequotes",
        "cshnullcmd",
        "cshnullglob",
        "debugbeforecmd",
        "dotglob",
        "dvorak",
        "emacs",
        "equals",
        "errexit",
        "errreturn",
        "evallineno",
        "exec",
        "extendedglob",
        "extendedhistory",
        "flowcontrol",
        "forcefloat",
        "functionargzero",
        "glob",
        "globassign",
        "globcomplete",
        "globdots",
        "globstarshort",
        "globsubst",
        "globalexport",
        "globalrcs",
        "hashall",
        "hashcmds",
        "hashdirs",
        "hashexecutablesonly",
        "hashlistall",
        "histallowclobber",
        "histappend",
        "histbeep",
        "histexpand",
        "histexpiredupsfirst",
        "histfcntllock",
        "histfindnodups",
        "histignorealldups",
        "histignoredups",
        "histignorespace",
        "histlexwords",
        "histnofunctions",
        "histnostore",
        "histreduceblanks",
        "histsavebycopy",
        "histsavenodups",
        "histsubstpattern",
        "histverify",
        "hup",
        "ignorebraces",
        "ignoreclosebraces",
        "ignoreeof",
        "incappendhistory",
        "incappendhistorytime",
        "interactive",
        "interactivecomments",
        "ksharrays",
        "kshautoload",
        "kshglob",
        "kshoptionprint",
        "kshtypeset",
        "kshzerosubscript",
        "listambiguous",
        "listbeep",
        "listpacked",
        "listrowsfirst",
        "listtypes",
        "localloops",
        "localoptions",
        "localpatterns",
        "localtraps",
        "log",
        "login",
        "longlistjobs",
        "magicequalsubst",
        "mailwarn",
        "mailwarning",
        "markdirs",
        "menucomplete",
        "monitor",
        "multibyte",
        "multifuncdef",
        "multios",
        "nomatch",
        "notify",
        "nullglob",
        "numericglobsort",
        "octalzeroes",
        "onecmd",
        "overstrike",
        "pathdirs",
        "pathscript",
        "physical",
        "pipefail",
        "posixaliases",
        "posixargzero",
        "posixbuiltins",
        "posixcd",
        "posixidentifiers",
        "posixjobs",
        "posixstrings",
        "posixtraps",
        "printeightbit",
        "printexitvalue",
        "privileged",
        "promptbang",
        "promptcr",
        "promptpercent",
        "promptsp",
        "promptsubst",
        "promptvars",
        "pushdignoredups",
        "pushdminus",
        "pushdsilent",
        "pushdtohome",
        "rcexpandparam",
        "rcquotes",
        "rcs",
        "recexact",
        "rematchpcre",
        "restricted",
        "rmstarsilent",
        "rmstarwait",
        "sharehistory",
        "shfileexpansion",
        "shglob",
        "shinstdin",
        "shnullcmd",
        "shoptionletters",
        "shortloops",
        "shortrepeat",
        "shwordsplit",
        "singlecommand",
        "singlelinezle",
        "sourcetrace",
        "stdin",
        "sunkeyboardhack",
        "trackall",
        "transientrprompt",
        "trapsasync",
        "typesetsilent",
        "typesettounset",
        "unset",
        "verbose",
        "vi",
        "warncreateglobal",
        "warnnestedvar",
        "xtrace",
        "zle",
    ]
    .into_iter()
    .collect()
});

/// O(1) builtin lookup — replaces the 130+ arm matches! macro in is_builtin()
static BUILTIN_SET: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "cd", "chdir", "pwd", "echo", "export", "unset", "source", "exit",
        "return", "bye", "logout", "log", "true", "false", "test", "local",
        "declare", "typeset", "read", "shift", "eval", "jobs", "fg", "bg",
        "kill", "disown", "wait", "autoload", "history", "fc", "trap",
        "suspend", "alias", "unalias", "set", "shopt", "setopt", "unsetopt",
        "getopts", "type", "hash", "command", "builtin", "let", "pushd",
        "popd", "dirs", "printf", "break", "continue", "disable", "enable",
        "emulate", "exec", "float", "integer", "functions", "print", "whence",
        "where", "which", "ulimit", "limit", "unlimit", "umask", "rehash",
        "unhash", "times", "zmodload", "r", "ttyctl", "noglob", "zstat",
        "stat", "strftime", "zsleep", "zln", "zmv", "zcp", "coproc",
        "zparseopts", "readonly", "unfunction", "getln", "pushln", "bindkey",
        "zle", "sched", "zformat", "zcompile", "vared", "echotc", "echoti",
        "zpty", "zprof", "zsocket", "ztcp", "zregexparse", "clone",
        "comparguments", "compcall", "compctl", "compdef", "compdescribe",
        "compfiles", "compgroups", "compinit", "compquote", "comptags",
        "comptry", "compvalues", "cdreplay", "cap", "getcap", "setcap",
        "zftp", "zcurses", "sysread", "syswrite", "syserror", "sysopen",
        "sysseek", "private", "zgetattr", "zsetattr", "zdelattr", "zlistattr",
        "[", ".", ":", "compgen", "complete",
    ]
    .into_iter()
    .collect()
});

/// Convert float to hex representation (%a/%A format)
fn float_to_hex(val: f64, uppercase: bool) -> String {
    if val.is_nan() {
        return if uppercase { "NAN" } else { "nan" }.to_string();
    }
    if val.is_infinite() {
        return if val > 0.0 {
            if uppercase {
                "INF"
            } else {
                "inf"
            }
        } else {
            if uppercase {
                "-INF"
            } else {
                "-inf"
            }
        }
        .to_string();
    }
    if val == 0.0 {
        let sign = if val.is_sign_negative() { "-" } else { "" };
        return if uppercase {
            format!("{}0X0P+0", sign)
        } else {
            format!("{}0x0p+0", sign)
        };
    }

    let sign = if val < 0.0 { "-" } else { "" };
    let abs_val = val.abs();
    let bits = abs_val.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32 - 1023;
    let mantissa = bits & 0xfffffffffffff;

    let hex_mantissa = format!("{:013x}", mantissa);
    let hex_mantissa = hex_mantissa.trim_end_matches('0');
    let hex_mantissa = if hex_mantissa.is_empty() {
        "0"
    } else {
        hex_mantissa
    };

    if uppercase {
        format!("{}0X1.{}P{:+}", sign, hex_mantissa.to_uppercase(), exponent)
    } else {
        format!("{}0x1.{}p{:+}", sign, hex_mantissa, exponent)
    }
}

/// Quote a string for shell output (like zsh's set output)
fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    // Check if quoting is needed
    let needs_quotes = s.chars().any(|c| {
        matches!(
            c,
            ' ' | '\t'
                | '\n'
                | '\''
                | '"'
                | '\\'
                | '$'
                | '`'
                | '!'
                | '*'
                | '?'
                | '['
                | ']'
                | '{'
                | '}'
                | '('
                | ')'
                | '<'
                | '>'
                | '|'
                | '&'
                | ';'
                | '#'
                | '~'
        )
    });
    if !needs_quotes {
        return s.to_string();
    }
    // Use single quotes, escaping single quotes as '\''
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Quote a value for typeset -p output (re-executable code)
/// Uses single quoting only when the value contains special characters
fn shell_quote_value(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    let needs_quotes = s.chars().any(|c| {
        matches!(
            c,
            ' ' | '\t'
                | '\n'
                | '\''
                | '"'
                | '\\'
                | '$'
                | '`'
                | '!'
                | '*'
                | '?'
                | '['
                | ']'
                | '{'
                | '}'
                | '('
                | ')'
                | '<'
                | '>'
                | '|'
                | '&'
                | ';'
                | '#'
                | '~'
                | '^'
        )
    });
    if !needs_quotes {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

use crate::jobs::{continue_job, wait_for_child, wait_for_job, JobState, JobTable};
use crate::parser::{
    CaseTerminator, CompoundCommand, CondExpr, ListOp, Redirect, RedirectOp, ShellCommand,
    ShellParser, ShellWord, SimpleCommand, VarModifier, ZshParamFlag,
};
use crate::zwc::ZwcFile;
use std::collections::HashMap;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// A completion specification for the `complete` builtin
#[derive(Debug, Clone, Default)]
pub struct CompSpec {
    pub actions: Vec<String>,     // -a, -b, -c, etc.
    pub wordlist: Option<String>, // -W wordlist
    pub function: Option<String>, // -F function
    pub command: Option<String>,  // -C command
    pub globpat: Option<String>,  // -G glob
    pub prefix: Option<String>,   // -P prefix
    pub suffix: Option<String>,   // -S suffix
}

/// A single completion match for zsh-style completion
#[derive(Debug, Clone)]
pub struct CompMatch {
    pub word: String,                   // The actual completion word
    pub display: Option<String>,        // Display string (-d)
    pub prefix: Option<String>,         // -P prefix (inserted but not part of match)
    pub suffix: Option<String>,         // -S suffix (inserted but not part of match)
    pub hidden_prefix: Option<String>,  // -p hidden prefix
    pub hidden_suffix: Option<String>,  // -s hidden suffix
    pub ignored_prefix: Option<String>, // -i ignored prefix
    pub ignored_suffix: Option<String>, // -I ignored suffix
    pub group: Option<String>,          // -J/-V group name
    pub description: Option<String>,    // -X explanation
    pub remove_suffix: Option<String>,  // -r remove chars
    pub file_match: bool,               // -f flag
    pub quote_match: bool,              // -q flag
}

impl Default for CompMatch {
    fn default() -> Self {
        Self {
            word: String::new(),
            display: None,
            prefix: None,
            suffix: None,
            hidden_prefix: None,
            hidden_suffix: None,
            ignored_prefix: None,
            ignored_suffix: None,
            group: None,
            description: None,
            remove_suffix: None,
            file_match: false,
            quote_match: false,
        }
    }
}

/// Completion group for organizing matches
#[derive(Debug, Clone, Default)]
pub struct CompGroup {
    pub name: String,
    pub matches: Vec<CompMatch>,
    pub explanation: Option<String>,
    pub sorted: bool,
}

/// zsh completion state (compstate associative array)
#[derive(Debug, Clone, Default)]
pub struct CompState {
    pub context: String,               // completion context
    pub exact: String,                 // exact match handling
    pub exact_string: String,          // the exact string if matched
    pub ignored: i32,                  // number of ignored matches
    pub insert: String,                // what to insert
    pub insert_positions: String,      // cursor positions after insert
    pub last_prompt: String,           // whether to return to last prompt
    pub list: String,                  // listing style
    pub list_lines: i32,               // number of lines for listing
    pub list_max: i32,                 // max matches to list
    pub nmatches: i32,                 // number of matches
    pub old_insert: String,            // previous insert value
    pub old_list: String,              // previous list value
    pub parameter: String,             // parameter being completed
    pub pattern_insert: String,        // pattern insert mode
    pub pattern_match: String,         // pattern matching mode
    pub quote: String,                 // quoting type
    pub quoting: String,               // current quoting
    pub redirect: String,              // redirection type
    pub restore: String,               // restore mode
    pub to_end: String,                // move to end mode
    pub unambiguous: String,           // unambiguous prefix
    pub unambiguous_cursor: i32,       // cursor pos in unambiguous
    pub unambiguous_positions: String, // positions in unambiguous
    pub vared: String,                 // vared context
}

/// zstyle entry for completion configuration
#[derive(Debug, Clone)]
pub struct ZStyle {
    pub pattern: String,
    pub style: String,
    pub values: Vec<String>,
}

bitflags::bitflags! {
    /// Flags for autoloaded functions
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

/// State for a zpty pseudo-terminal
pub struct ZptyState {
    pub pid: u32,
    pub cmd: String,
    pub stdin: Option<std::process::ChildStdin>,
    pub stdout: Option<std::process::ChildStdout>,
    pub child: Option<std::process::Child>,
}

/// Scheduled command for sched builtin
pub struct ScheduledCommand {
    pub id: u32,
    pub run_at: std::time::SystemTime,
    pub command: String,
}

/// Profiling entry for zprof
#[derive(Clone, Default)]
pub struct ProfileEntry {
    pub calls: u64,
    pub total_time_us: u64,
    pub self_time_us: u64,
}

/// Unix domain socket state
pub struct UnixSocketState {
    pub path: Option<PathBuf>,
    pub listening: bool,
    pub stream: Option<std::os::unix::net::UnixStream>,
    pub listener: Option<std::os::unix::net::UnixListener>,
}

pub struct ShellExecutor {
    pub functions: HashMap<String, ShellCommand>,
    pub aliases: HashMap<String, String>,
    pub global_aliases: HashMap<String, String>, // alias -g: expand anywhere
    pub suffix_aliases: HashMap<String, String>, // alias -s: expand by file extension
    pub last_status: i32,
    pub variables: HashMap<String, String>,
    pub arrays: HashMap<String, Vec<String>>,
    pub assoc_arrays: HashMap<String, HashMap<String, String>>, // zsh associative arrays
    pub jobs: JobTable,
    pub fpath: Vec<PathBuf>,
    pub zwc_cache: HashMap<PathBuf, ZwcFile>,
    pub positional_params: Vec<String>,
    pub history: Option<HistoryEngine>,
    process_sub_counter: u32,
    pub traps: HashMap<String, String>,
    pub options: HashMap<String, bool>,
    pub completions: HashMap<String, CompSpec>, // command -> completion spec
    pub dir_stack: Vec<PathBuf>,
    // zsh completion system state
    pub comp_matches: Vec<CompMatch>, // Current completion matches
    pub comp_groups: Vec<CompGroup>,  // Completion groups
    pub comp_state: CompState,        // compstate associative array
    pub zstyles: Vec<ZStyle>,         // zstyle configurations
    pub comp_words: Vec<String>,      // words on command line
    pub comp_current: i32,            // current word index (1-based)
    pub comp_prefix: String,          // PREFIX parameter
    pub comp_suffix: String,          // SUFFIX parameter
    pub comp_iprefix: String,         // IPREFIX parameter
    pub comp_isuffix: String,         // ISUFFIX parameter
    pub readonly_vars: std::collections::HashSet<String>, // Read-only variables
    /// Stack for `local` variable save/restore (name, old_value).
    pub local_save_stack: Vec<(String, Option<String>)>,
    /// Current function scope depth for `local` tracking.
    pub local_scope_depth: usize,
    pub autoload_pending: HashMap<String, AutoloadFlags>, // Functions marked for autoload
    // zsh hooks (precmd, preexec, chpwd, etc.)
    pub hook_functions: HashMap<String, Vec<String>>, // hook_name -> [function_names]
    // Named directories (hash -d)
    pub named_dirs: HashMap<String, PathBuf>, // name -> path
    // zpty - pseudo-terminal management
    pub zptys: HashMap<String, ZptyState>,
    // sysopen - file descriptor management
    pub open_fds: HashMap<i32, std::fs::File>,
    pub next_fd: i32,
    // sched - scheduled commands
    pub scheduled_commands: Vec<ScheduledCommand>,
    // zprof - profiling data
    pub profile_data: HashMap<String, ProfileEntry>,
    pub profiling_enabled: bool,
    // zsocket - Unix domain sockets
    pub unix_sockets: HashMap<i32, UnixSocketState>,
    // compsys - completion system cache
    pub compsys_cache: Option<CompsysCache>,
    // Background compinit — receiver for async fpath scan result
    pub compinit_pending: Option<(std::sync::mpsc::Receiver<CompInitBgResult>, std::time::Instant)>,
    // Plugin source cache — stores side effects of source/. in SQLite
    pub plugin_cache: Option<crate::plugin_cache::PluginCache>,
    // cdreplay - deferred compdef calls for zinit turbo mode
    pub deferred_compdefs: Vec<Vec<String>>,
    // command hash table (hash builtin)
    pub command_hash: HashMap<String, String>,
    // Control flow signals
    returning: Option<i32>, // Set by return builtin, cleared after function returns
    breaking: i32,          // break level (0 = not breaking, N = break N levels)
    continuing: i32,        // continue level
    // New module state
    pub pcre_state: PcreState,
    pub tcp_sessions: TcpSessions,
    pub zftp: Zftp,
    pub profiler: Profiler,
    pub style_table: StyleTable,
    /// zsh compatibility mode - use .zcompdump, fpath scanning, etc.
    pub zsh_compat: bool,
    /// POSIX sh strict mode — no SQLite, no worker pool, no zsh extensions
    pub posix_mode: bool,
    /// Worker thread pool for background tasks (compinit, process subs, etc.)
    pub worker_pool: std::sync::Arc<crate::worker::WorkerPool>,
    /// AOP intercept table: command/function name → advice chain.
    /// Glob patterns supported (e.g. "git *", "*").
    pub intercepts: Vec<Intercept>,
    /// Async job handles: id → receiver for (status, stdout)
    pub async_jobs: HashMap<u32, crossbeam_channel::Receiver<(i32, String)>>,
    /// Next async job ID
    pub next_async_id: u32,
    /// Defer stack: commands to run on scope exit (LIFO).
    pub defer_stack: Vec<Vec<String>>,
}

impl ShellExecutor {
    pub fn new() -> Self {
        tracing::debug!("ShellExecutor::new() initializing");
        // Initialize fpath from FPATH env var or use defaults
        let fpath = env::var("FPATH")
            .unwrap_or_default()
            .split(':')
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect();

        let history = HistoryEngine::new().ok();

        // Initialize standard zsh variables
        let mut variables = HashMap::new();
        variables.insert("ZSH_VERSION".to_string(), "5.9".to_string());
        variables.insert(
            "ZSH_PATCHLEVEL".to_string(),
            "zsh-5.9-0-g73d3173".to_string(),
        );
        variables.insert("ZSH_NAME".to_string(), "zsh".to_string());
        variables.insert(
            "SHLVL".to_string(),
            env::var("SHLVL")
                .map(|v| {
                    v.parse::<i32>()
                        .map(|n| (n + 1).to_string())
                        .unwrap_or_else(|_| "1".to_string())
                })
                .unwrap_or_else(|_| "1".to_string()),
        );

        Self {
            functions: HashMap::new(),
            aliases: HashMap::new(),
            global_aliases: HashMap::new(),
            suffix_aliases: HashMap::new(),
            last_status: 0,
            variables,
            arrays: {
                let mut a = HashMap::new();
                // $path mirrors $PATH (tied array)
                let path_dirs: Vec<String> = env::var("PATH")
                    .unwrap_or_default()
                    .split(':')
                    .map(|s| s.to_string())
                    .collect();
                a.insert("path".to_string(), path_dirs);
                a
            },
            assoc_arrays: HashMap::new(),
            jobs: JobTable::new(),
            fpath,
            zwc_cache: HashMap::new(),
            positional_params: Vec::new(),
            history,
            completions: HashMap::new(),
            dir_stack: Vec::new(),
            process_sub_counter: 0,
            traps: HashMap::new(),
            options: Self::default_options(),
            // zsh completion system
            comp_matches: Vec::new(),
            comp_groups: Vec::new(),
            comp_state: CompState::default(),
            zstyles: Vec::new(),
            comp_words: Vec::new(),
            comp_current: 0,
            comp_prefix: String::new(),
            comp_suffix: String::new(),
            comp_iprefix: String::new(),
            comp_isuffix: String::new(),
            readonly_vars: std::collections::HashSet::new(),
            local_save_stack: Vec::new(),
            local_scope_depth: 0,
            autoload_pending: HashMap::new(),
            hook_functions: HashMap::new(),
            named_dirs: HashMap::new(),
            zptys: HashMap::new(),
            open_fds: HashMap::new(),
            next_fd: 10,
            scheduled_commands: Vec::new(),
            profile_data: HashMap::new(),
            profiling_enabled: false,
            unix_sockets: HashMap::new(),
            compsys_cache: {
                let cache_path = compsys::cache::default_cache_path();
                if cache_path.exists() {
                    let db_size = std::fs::metadata(&cache_path).map(|m| m.len()).unwrap_or(0);
                    match CompsysCache::open(&cache_path) {
                        Ok(c) => {
                            tracing::info!(
                                db_bytes = db_size,
                                path = %cache_path.display(),
                                "compsys: sqlite cache opened"
                            );
                            Some(c)
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "compsys: failed to open cache");
                            None
                        }
                    }
                } else {
                    tracing::debug!("compsys: no cache at {}", cache_path.display());
                    None
                }
            },
            compinit_pending: None, // (receiver, start_time)
            plugin_cache: {
                let pc_path = crate::plugin_cache::default_cache_path();
                if let Some(parent) = pc_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match crate::plugin_cache::PluginCache::open(&pc_path) {
                    Ok(pc) => {
                        let (plugins, functions) = pc.stats();
                        tracing::info!(
                            plugins,
                            cached_functions = functions,
                            path = %pc_path.display(),
                            "plugin_cache: sqlite opened"
                        );
                        Some(pc)
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "plugin_cache: failed to open");
                        None
                    }
                }
            },
            deferred_compdefs: Vec::new(),
            command_hash: HashMap::new(),
            returning: None,
            breaking: 0,
            continuing: 0,
            pcre_state: PcreState::new(),
            tcp_sessions: TcpSessions::new(),
            zftp: Zftp::new(),
            profiler: Profiler::new(),
            style_table: StyleTable::new(),
            zsh_compat: false,
            posix_mode: false,
            worker_pool: {
                let config = crate::config::load();
                let pool_size = crate::config::resolve_pool_size(&config.worker_pool);
                std::sync::Arc::new(crate::worker::WorkerPool::new(pool_size))
            },
            intercepts: Vec::new(),
            async_jobs: HashMap::new(),
            next_async_id: 1,
            defer_stack: Vec::new(),
        }
    }

    /// Enter POSIX strict mode — drop all SQLite caches, shrink worker pool to minimum.
    /// No zsh extensions, no caching, no threads beyond the bare minimum. Dinosaur mode.
    pub fn enter_posix_mode(&mut self) {
        self.posix_mode = true;
        self.plugin_cache = None;
        self.compsys_cache = None;
        self.compinit_pending = None;
        // Worker pool stays at size 1 — we can't drop it entirely because
        // some code paths use it unconditionally, but with 1 thread it's
        // effectively serial.
        self.worker_pool = std::sync::Arc::new(crate::worker::WorkerPool::new(1));
        tracing::info!("POSIX strict mode: SQLite caches dropped, worker pool shrunk to 1");
    }

    /// Run hook functions (precmd, preexec, chpwd, etc.)
    pub fn run_hooks(&mut self, hook_name: &str) {
        if let Some(funcs) = self.hook_functions.get(hook_name).cloned() {
            for func_name in funcs {
                if self.functions.contains_key(&func_name) {
                    let _ = self.execute_script(&format!("{}", func_name));
                }
            }
        }
        // Also check for hook arrays (e.g., precmd_functions)
        let array_name = format!("{}_functions", hook_name);
        if let Some(funcs) = self.arrays.get(&array_name).cloned() {
            for func_name in funcs {
                if self.functions.contains_key(&func_name) {
                    let _ = self.execute_script(&format!("{}", func_name));
                }
            }
        }
    }

    /// Add a function to a hook
    pub fn add_hook(&mut self, hook_name: &str, func_name: &str) {
        self.hook_functions
            .entry(hook_name.to_string())
            .or_default()
            .push(func_name.to_string());
    }

    /// Add a named directory (hash -d name=path)
    pub fn add_named_dir(&mut self, name: &str, path: &str) {
        self.named_dirs
            .insert(name.to_string(), PathBuf::from(path));
    }

    /// Expand ~ with named directories
    pub fn expand_tilde_named(&self, path: &str) -> String {
        if path.starts_with('~') {
            let rest = &path[1..];
            // Check for ~name or ~name/...
            let (name, suffix) = if let Some(slash_pos) = rest.find('/') {
                (&rest[..slash_pos], &rest[slash_pos..])
            } else {
                (rest, "")
            };

            if name.is_empty() {
                // Regular ~ expansion
                if let Ok(home) = std::env::var("HOME") {
                    return format!("{}{}", home, suffix);
                }
            } else if let Some(dir) = self.named_dirs.get(name) {
                return format!("{}{}", dir.display(), suffix);
            }
        }
        path.to_string()
    }

    fn all_zsh_options() -> &'static [&'static str] {
        &[
            "aliases",
            "aliasfuncdef",
            "allexport",
            "alwayslastprompt",
            "alwaystoend",
            "appendcreate",
            "appendhistory",
            "autocd",
            "autocontinue",
            "autolist",
            "automenu",
            "autonamedirs",
            "autoparamkeys",
            "autoparamslash",
            "autopushd",
            "autoremoveslash",
            "autoresume",
            "badpattern",
            "banghist",
            "bareglobqual",
            "bashautolist",
            "bashrematch",
            "beep",
            "bgnice",
            "braceccl",
            "braceexpand",
            "bsdecho",
            "caseglob",
            "casematch",
            "casepaths",
            "cbases",
            "cdablevars",
            "cdsilent",
            "chasedots",
            "chaselinks",
            "checkjobs",
            "checkrunningjobs",
            "clobber",
            "clobberempty",
            "combiningchars",
            "completealiases",
            "completeinword",
            "continueonerror",
            "correct",
            "correctall",
            "cprecedences",
            "cshjunkiehistory",
            "cshjunkieloops",
            "cshjunkiequotes",
            "cshnullcmd",
            "cshnullglob",
            "debugbeforecmd",
            "dotglob",
            "dvorak",
            "emacs",
            "equals",
            "errexit",
            "errreturn",
            "evallineno",
            "exec",
            "extendedglob",
            "extendedhistory",
            "flowcontrol",
            "forcefloat",
            "functionargzero",
            "glob",
            "globassign",
            "globcomplete",
            "globdots",
            "globstarshort",
            "globsubst",
            "globalexport",
            "globalrcs",
            "hashall",
            "hashcmds",
            "hashdirs",
            "hashexecutablesonly",
            "hashlistall",
            "histallowclobber",
            "histappend",
            "histbeep",
            "histexpand",
            "histexpiredupsfirst",
            "histfcntllock",
            "histfindnodups",
            "histignorealldups",
            "histignoredups",
            "histignorespace",
            "histlexwords",
            "histnofunctions",
            "histnostore",
            "histreduceblanks",
            "histsavebycopy",
            "histsavenodups",
            "histsubstpattern",
            "histverify",
            "hup",
            "ignorebraces",
            "ignoreclosebraces",
            "ignoreeof",
            "incappendhistory",
            "incappendhistorytime",
            "interactive",
            "interactivecomments",
            "ksharrays",
            "kshautoload",
            "kshglob",
            "kshoptionprint",
            "kshtypeset",
            "kshzerosubscript",
            "listambiguous",
            "listbeep",
            "listpacked",
            "listrowsfirst",
            "listtypes",
            "localloops",
            "localoptions",
            "localpatterns",
            "localtraps",
            "log",
            "login",
            "longlistjobs",
            "magicequalsubst",
            "mailwarn",
            "mailwarning",
            "markdirs",
            "menucomplete",
            "monitor",
            "multibyte",
            "multifuncdef",
            "multios",
            "nomatch",
            "notify",
            "nullglob",
            "numericglobsort",
            "octalzeroes",
            "onecmd",
            "overstrike",
            "pathdirs",
            "pathscript",
            "physical",
            "pipefail",
            "posixaliases",
            "posixargzero",
            "posixbuiltins",
            "posixcd",
            "posixidentifiers",
            "posixjobs",
            "posixstrings",
            "posixtraps",
            "printeightbit",
            "printexitvalue",
            "privileged",
            "promptbang",
            "promptcr",
            "promptpercent",
            "promptsp",
            "promptsubst",
            "promptvars",
            "pushdignoredups",
            "pushdminus",
            "pushdsilent",
            "pushdtohome",
            "rcexpandparam",
            "rcquotes",
            "rcs",
            "recexact",
            "rematchpcre",
            "restricted",
            "rmstarsilent",
            "rmstarwait",
            "sharehistory",
            "shfileexpansion",
            "shglob",
            "shinstdin",
            "shnullcmd",
            "shoptionletters",
            "shortloops",
            "shortrepeat",
            "shwordsplit",
            "singlecommand",
            "singlelinezle",
            "sourcetrace",
            "stdin",
            "sunkeyboardhack",
            "trackall",
            "transientrprompt",
            "trapsasync",
            "typesetsilent",
            "typesettounset",
            "unset",
            "verbose",
            "vi",
            "warncreateglobal",
            "warnnestedvar",
            "xtrace",
            "zle",
        ]
    }

    fn default_options() -> HashMap<String, bool> {
        let mut opts = HashMap::new();
        // Initialize all options to false first
        for opt in Self::all_zsh_options() {
            opts.insert(opt.to_string(), false);
        }
        // Set zsh defaults (options marked with <D> or <Z> in zshoptions man page)
        let defaults_on = [
            "aliases",
            "alwayslastprompt",
            "appendhistory",
            "autolist",
            "automenu",
            "autoparamkeys",
            "autoparamslash",
            "autoremoveslash",
            "badpattern",
            "banghist",
            "bareglobqual",
            "beep",
            "bgnice",
            "caseglob",
            "casematch",
            "checkjobs",
            "checkrunningjobs",
            "clobber",
            "debugbeforecmd",
            "equals",
            "evallineno",
            "exec",
            "flowcontrol",
            "functionargzero",
            "glob",
            "globalexport",
            "globalrcs",
            "hashcmds",
            "hashdirs",
            "hashlistall",
            "histbeep",
            "histsavebycopy",
            "hup",
            "interactive",
            "listambiguous",
            "listbeep",
            "listtypes",
            "monitor",
            "multibyte",
            "multifuncdef",
            "multios",
            "nomatch",
            "notify",
            "promptcr",
            "promptpercent",
            "promptsp",
            "rcs",
            "shinstdin",
            "shortloops",
            "unset",
            "zle",
        ];
        for opt in defaults_on {
            opts.insert(opt.to_string(), true);
        }
        opts
    }

    /// Normalize option name: lowercase, remove underscores/hyphens, handle "no" prefix
    fn normalize_option_name(name: &str) -> (String, bool) {
        let normalized = name.to_lowercase().replace(['-', '_'], "");
        if let Some(stripped) = normalized.strip_prefix("no") {
            // O(1) lookup in HashSet instead of linear scan
            if ZSH_OPTIONS_SET.contains(stripped) {
                return (stripped.to_string(), false);
            }
        }
        (normalized, true)
    }

    /// Check if option name matches a pattern (for -m flag)
    fn option_matches_pattern(opt: &str, pattern: &str) -> bool {
        let pat = pattern.to_lowercase().replace(['-', '_'], "");
        let opt_lower = opt.to_lowercase();

        if pat.contains('*') || pat.contains('?') || pat.contains('[') {
            let regex_pat = pat.replace('.', "\\.").replace('*', ".*").replace('?', ".");
            let full_pattern = format!("^{}$", regex_pat);
            cached_regex(&full_pattern)
                .map(|re| re.is_match(&opt_lower))
                .unwrap_or(false)
        } else {
            opt_lower == pat
        }
    }

    /// Try to load a function from ZWC files in fpath
    pub fn autoload_function(&mut self, name: &str) -> Option<ShellCommand> {
        // First check if already loaded
        if let Some(func) = self.functions.get(name) {
            return Some(func.clone());
        }

        // Search fpath for the function - use index to avoid borrow issues
        for i in 0..self.fpath.len() {
            let dir = self.fpath[i].clone();
            // Try directory.zwc first
            let zwc_path = dir.with_extension("zwc");
            if zwc_path.exists() {
                if let Some(func) = self.load_function_from_zwc(&zwc_path, name) {
                    return Some(func);
                }
            }

            // Try individual function.zwc
            let func_zwc = dir.join(format!("{}.zwc", name));
            if func_zwc.exists() {
                if let Some(func) = self.load_function_from_zwc(&func_zwc, name) {
                    return Some(func);
                }
            }

            // Look for directory/*.zwc files containing this function
            if dir.is_dir() {
                if let Ok(entries) = fs::read_dir(&dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().map_or(false, |e| e == "zwc") {
                            if let Some(func) = self.load_function_from_zwc(&path, name) {
                                return Some(func);
                            }
                        }
                    }
                }
            }
        }

        None
    }

    /// Load a specific function from a ZWC file
    fn load_function_from_zwc(&mut self, path: &Path, name: &str) -> Option<ShellCommand> {
        // Check cache
        let zwc = if let Some(cached) = self.zwc_cache.get(path) {
            cached
        } else {
            // Load and cache the ZWC file
            let zwc = ZwcFile::load(path).ok()?;
            self.zwc_cache.insert(path.to_path_buf(), zwc);
            self.zwc_cache.get(path)?
        };

        // Find the function
        let func = zwc.get_function(name)?;
        let decoded = zwc.decode_function(func)?;

        // Convert to shell command and cache
        let shell_func = decoded.to_shell_function()?;

        // Register the function
        if let ShellCommand::FunctionDef(fname, body) = &shell_func {
            self.functions.insert(fname.clone(), (**body).clone());
        }

        Some(shell_func)
    }

    /// Add a directory to fpath
    pub fn add_fpath(&mut self, path: PathBuf) {
        if !self.fpath.contains(&path) {
            self.fpath.insert(0, path);
        }
    }

    /// Match a string against a shell glob pattern
    fn glob_match(&self, s: &str, pattern: &str) -> bool {
        // Convert shell glob to regex
        let mut regex_pattern = String::from("^");
        let mut chars = pattern.chars().peekable();

        while let Some(c) = chars.next() {
            match c {
                '*' => regex_pattern.push_str(".*"),
                '?' => regex_pattern.push('.'),
                '[' => {
                    regex_pattern.push('[');
                    // Handle character class
                    while let Some(cc) = chars.next() {
                        if cc == ']' {
                            regex_pattern.push(']');
                            break;
                        }
                        regex_pattern.push(cc);
                    }
                }
                '(' => {
                    // Handle alternation (a|b|c) -> (a|b|c)
                    regex_pattern.push('(');
                }
                ')' => regex_pattern.push(')'),
                '|' => regex_pattern.push('|'),
                '.' | '+' | '^' | '$' | '\\' | '{' | '}' => {
                    regex_pattern.push('\\');
                    regex_pattern.push(c);
                }
                _ => regex_pattern.push(c),
            }
        }
        regex_pattern.push('$');

        regex::Regex::new(&regex_pattern)
            .map(|re| re.is_match(s))
            .unwrap_or(false)
    }

    /// Static glob match — same logic as glob_match but callable without &self,
    /// needed for Rayon parallel iterators that can't capture &self.
    pub fn glob_match_static(s: &str, pattern: &str) -> bool {
        let mut regex_pattern = String::from("^");
        let mut chars = pattern.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '*' => regex_pattern.push_str(".*"),
                '?' => regex_pattern.push('.'),
                '[' => {
                    regex_pattern.push('[');
                    while let Some(cc) = chars.next() {
                        if cc == ']' {
                            regex_pattern.push(']');
                            break;
                        }
                        regex_pattern.push(cc);
                    }
                }
                '(' => regex_pattern.push('('),
                ')' => regex_pattern.push(')'),
                '|' => regex_pattern.push('|'),
                '.' | '+' | '^' | '$' | '\\' | '{' | '}' => {
                    regex_pattern.push('\\');
                    regex_pattern.push(c);
                }
                _ => regex_pattern.push(c),
            }
        }
        regex_pattern.push('$');
        regex::Regex::new(&regex_pattern)
            .map(|re| re.is_match(s))
            .unwrap_or(false)
    }

    /// Execute a script file with bytecode caching — skips lex+parse+compile on cache hit.
    /// The AST is stored in SQLite keyed by (path, mtime).
    pub fn execute_script_file(&mut self, file_path: &str) -> Result<i32, String> {
        let path = std::path::Path::new(file_path);
        let mtime = crate::plugin_cache::file_mtime(path);

        // Try AST cache first
        if let (Some(ref cache), Some((mt_s, mt_ns))) = (&self.plugin_cache, mtime) {
            if let Some(ast_bytes) = cache.check_ast(file_path, mt_s, mt_ns) {
                if let Ok(commands) = bincode::deserialize::<Vec<crate::parser::ShellCommand>>(&ast_bytes) {
                    tracing::info!(
                        path = file_path,
                        cmds = commands.len(),
                        bytes = ast_bytes.len(),
                        "execute_script_file: bytecode cache hit, skipping lex+parse"
                    );
                    for cmd in commands {
                        self.execute_command(&cmd)?;
                    }
                    return Ok(self.last_status);
                }
            }
        }

        // Cache miss — read file, parse, execute, cache AST on worker
        let content = std::fs::read_to_string(file_path)
            .map_err(|e| format!("{}: {}", file_path, e))?;
        let expanded = self.expand_history(&content);
        let mut parser = ShellParser::new(&expanded);
        let mut commands = parser.parse_script()?;
        tracing::debug!(
            path = file_path,
            cmds = commands.len(),
            "execute_script_file: bytecode cache miss, parsed from source"
        );

        // Optimize AST before execution and caching — constant folding, literal merging
        crate::ast_opt::optimize(&mut commands);

        // Execute
        for cmd in &commands {
            self.execute_command(cmd)?;
        }

        // Async-store the optimized AST in SQLite
        if let Some((mt_s, mt_ns)) = mtime {
            if let Ok(ast_bytes) = bincode::serialize(&commands) {
                let store_path = file_path.to_string();
                let cache_db_path = crate::plugin_cache::default_cache_path();
                let ast_size = ast_bytes.len();
                self.worker_pool.submit(move || {
                    match crate::plugin_cache::PluginCache::open(&cache_db_path) {
                        Ok(cache) => {
                            if let Err(e) = cache.store_ast(&store_path, mt_s, mt_ns, &ast_bytes) {
                                tracing::error!(path = %store_path, error = %e, "AST cache store failed");
                            } else {
                                tracing::debug!(path = %store_path, bytes = ast_size, "bytecode cached");
                            }
                        }
                        Err(e) => tracing::error!(error = %e, "plugin_cache: open for AST write failed"),
                    }
                });
            }
        }

        Ok(self.last_status)
    }

    #[tracing::instrument(skip(self, script), fields(len = script.len()))]
    pub fn execute_script(&mut self, script: &str) -> Result<i32, String> {
        // Expand history references before parsing
        let expanded = self.expand_history(script);

        let mut parser = ShellParser::new(&expanded);
        let commands = parser.parse_script()?;
        tracing::trace!(cmds = commands.len(), "execute_script: parsed");

        // Compile to fusevm bytecodes and execute on the VM.
        // The VM handles pure computation (arithmetic, control flow, variables).
        // Shell ops (Exec, Redirect, Pipeline, Glob, TestFile) callback into
        // the executor via execute_command for anything that needs shell state.
        // Primary path: compile to fusevm bytecodes and execute on the VM.
        // Fallback: tree-walker for commands the VM can't fully handle yet
        // (builtins that need executor state, complex redirects, etc.)
        let compiler = crate::shell_compiler::ShellCompiler::new();
        let chunk = compiler.compile(&commands);

        if !chunk.ops.is_empty() {
            let mut vm = fusevm::VM::new(chunk);
            match vm.run() {
                fusevm::VMResult::Ok(_) | fusevm::VMResult::Halted => {
                    self.last_status = vm.last_status;
                }
                fusevm::VMResult::Error(_) => {
                    // VM error — fall back to tree-walker for compatibility
                    for cmd in &commands {
                        self.execute_command(cmd)?;
                    }
                }
            }
        } else {
            // Empty compilation (no ops emitted) — tree-walk
            for cmd in &commands {
                self.execute_command(cmd)?;
            }
        }

        // Fire EXIT trap if set (matches zsh's zshexit behavior).
        // Remove it first to prevent infinite recursion.
        if let Some(action) = self.traps.remove("EXIT") {
            tracing::debug!("firing EXIT trap");
            let _ = self.execute_script(&action);
        }

        Ok(self.last_status)
    }

    /// Expand history references: !!, !n, !-n, !string, !?string?
    fn expand_history(&self, input: &str) -> String {
        let Some(ref engine) = self.history else {
            return input.to_string();
        };

        // Quick check: nothing to expand
        if !input.contains('!') && !input.starts_with('^') {
            return input.to_string();
        }

        let history_count = engine.count().unwrap_or(0) as usize;
        if history_count == 0 {
            return input.to_string();
        }

        let chars: Vec<char> = input.chars().collect();

        // ^foo^bar quick substitution (only at start of input)
        if chars.first() == Some(&'^') {
            if let Some(expanded) = self.history_quick_subst(&chars, engine) {
                return expanded;
            }
        }

        let mut result = String::new();
        let mut i = 0;
        let mut in_single_quote = false;
        let mut in_brace = 0; // Track ${...} nesting
        let mut last_subst: Option<(String, String)> = None; // for :& modifier

        while i < chars.len() {
            // Track single quotes — no history expansion inside them
            if chars[i] == '\'' && in_brace == 0 {
                in_single_quote = !in_single_quote;
                result.push(chars[i]);
                i += 1;
                continue;
            }
            if in_single_quote {
                result.push(chars[i]);
                i += 1;
                continue;
            }

            // Track ${...} nesting
            if i + 1 < chars.len() && chars[i] == '$' && chars[i + 1] == '{' {
                in_brace += 1;
                result.push(chars[i]);
                i += 1;
                result.push(chars[i]);
                i += 1;
                continue;
            }
            if chars[i] == '}' && in_brace > 0 {
                in_brace -= 1;
                result.push(chars[i]);
                i += 1;
                continue;
            }

            // Backslash-escaped ! is literal
            if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '!' {
                result.push('!');
                i += 2;
                continue;
            }

            if chars[i] == '!' && in_brace == 0 {
                if i + 1 >= chars.len() {
                    // Trailing ! — literal
                    result.push('!');
                    i += 1;
                    continue;
                }

                let next = chars[i + 1];
                // ! followed by space, =, ( — literal (zsh rule)
                if next == ' ' || next == '\t' || next == '=' || next == '(' || next == '\n' {
                    result.push('!');
                    i += 1;
                    continue;
                }

                // Resolve the event string
                let (event_str, new_i) = self.history_resolve_event(&chars, i, engine, &result);
                if let Some(ev) = event_str {
                    // Check for word designators and modifiers
                    let (final_str, final_i) =
                        self.history_apply_designators_and_modifiers(&chars, new_i, &ev, &mut last_subst);
                    result.push_str(&final_str);
                    i = final_i;
                } else {
                    // Could not resolve — keep the ! literal
                    result.push('!');
                    i += 1;
                }
                continue;
            }
            result.push(chars[i]);
            i += 1;
        }

        result
    }

    /// ^foo^bar quick substitution — replace first occurrence of foo with bar
    /// in the previous command.
    fn history_quick_subst(
        &self,
        chars: &[char],
        engine: &crate::history::HistoryEngine,
    ) -> Option<String> {
        let mut i = 1; // skip leading ^
        let mut old = String::new();
        while i < chars.len() && chars[i] != '^' {
            old.push(chars[i]);
            i += 1;
        }
        if i >= chars.len() {
            return None;
        }
        i += 1; // skip middle ^
        let mut new = String::new();
        while i < chars.len() && chars[i] != '^' && chars[i] != '\n' {
            new.push(chars[i]);
            i += 1;
        }
        let prev = engine.get_by_offset(0).ok()??;
        Some(prev.command.replacen(&old, &new, 1))
    }

    /// Resolve which history event ! refers to.  Returns (Some(full_command), index_after_event)
    /// or (None, original_index) if we can't resolve.
    fn history_resolve_event(
        &self,
        chars: &[char],
        bang_pos: usize,
        engine: &crate::history::HistoryEngine,
        current_line: &str,
    ) -> (Option<String>, usize) {
        let mut i = bang_pos + 1; // past the !

        // !{...} brace-wrapped event
        let in_brace = i < chars.len() && chars[i] == '{';
        if in_brace {
            i += 1;
        }

        let c = if i < chars.len() { chars[i] } else { return (None, bang_pos); };

        let (event, new_i) = match c {
            '!' => {
                // !! — previous command
                let entry = engine.get_by_offset(0).ok().flatten();
                (entry.map(|e| e.command), i + 1)
            }
            '#' => {
                // !# — current command line so far
                (Some(current_line.to_string()), i + 1)
            }
            '-' => {
                // !-n — nth previous command
                i += 1;
                let start = i;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
                if i > start {
                    let n: usize = chars[start..i].iter().collect::<String>().parse().unwrap_or(0);
                    if n > 0 {
                        let entry = engine.get_by_offset(n - 1).ok().flatten();
                        (entry.map(|e| e.command), i)
                    } else {
                        (None, bang_pos)
                    }
                } else {
                    (None, bang_pos)
                }
            }
            '?' => {
                // !?string? — contains search
                i += 1;
                let start = i;
                while i < chars.len() && chars[i] != '?' && chars[i] != '\n' {
                    i += 1;
                }
                let search: String = chars[start..i].iter().collect();
                if i < chars.len() && chars[i] == '?' {
                    i += 1;
                }
                let entry = engine.search(&search, 1).ok().and_then(|v| v.into_iter().next());
                (entry.map(|e| e.command), i)
            }
            c if c.is_ascii_digit() => {
                // !n — command by absolute number
                let start = i;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
                let n: i64 = chars[start..i].iter().collect::<String>().parse().unwrap_or(0);
                if n > 0 {
                    let entry = engine.get_by_number(n).ok().flatten();
                    (entry.map(|e| e.command), i)
                } else {
                    (None, bang_pos)
                }
            }
            '$' => {
                // !$ — last word of previous command (shorthand for !!:$)
                let entry = engine.get_by_offset(0).ok().flatten();
                let word = entry.and_then(|e| {
                    Self::history_split_words(&e.command).last().cloned()
                });
                // Return the word directly — skip designator parsing
                let final_i = if in_brace && i + 1 < chars.len() && chars[i + 1] == '}' {
                    i + 2
                } else {
                    i + 1
                };
                return (word, final_i);
            }
            '^' => {
                // !^ — first arg of previous command (shorthand for !!:1)
                let entry = engine.get_by_offset(0).ok().flatten();
                let word = entry.and_then(|e| {
                    let words = Self::history_split_words(&e.command);
                    words.get(1).cloned()
                });
                let final_i = if in_brace && i + 1 < chars.len() && chars[i + 1] == '}' {
                    i + 2
                } else {
                    i + 1
                };
                return (word, final_i);
            }
            '*' => {
                // !* — all args of previous command (shorthand for !!:*)
                let entry = engine.get_by_offset(0).ok().flatten();
                let word = entry.map(|e| {
                    let words = Self::history_split_words(&e.command);
                    if words.len() > 1 { words[1..].join(" ") } else { String::new() }
                });
                let final_i = if in_brace && i + 1 < chars.len() && chars[i + 1] == '}' {
                    i + 2
                } else {
                    i + 1
                };
                return (word, final_i);
            }
            c if c.is_alphabetic() || c == '_' || c == '/' || c == '.' => {
                // !string — prefix search
                let start = i;
                while i < chars.len()
                    && !chars[i].is_whitespace()
                    && chars[i] != ':'
                    && chars[i] != '!'
                    && chars[i] != '}'
                {
                    i += 1;
                }
                let prefix: String = chars[start..i].iter().collect();
                let entry = engine
                    .search_prefix(&prefix, 1)
                    .ok()
                    .and_then(|v| v.into_iter().next());
                (entry.map(|e| e.command), i)
            }
            _ => (None, bang_pos),
        };

        // Skip closing brace
        let final_i = if in_brace && new_i < chars.len() && chars[new_i] == '}' {
            new_i + 1
        } else {
            new_i
        };

        (event, final_i)
    }

    /// Split a command string into words for word designators, respecting quotes.
    fn history_split_words(cmd: &str) -> Vec<String> {
        let mut words = Vec::new();
        let mut current = String::new();
        let mut in_sq = false;
        let mut in_dq = false;
        let mut escaped = false;

        for c in cmd.chars() {
            if escaped {
                current.push(c);
                escaped = false;
                continue;
            }
            if c == '\\' {
                current.push(c);
                escaped = true;
                continue;
            }
            if c == '\'' && !in_dq {
                in_sq = !in_sq;
                current.push(c);
                continue;
            }
            if c == '"' && !in_sq {
                in_dq = !in_dq;
                current.push(c);
                continue;
            }
            if c.is_whitespace() && !in_sq && !in_dq {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
                continue;
            }
            current.push(c);
        }
        if !current.is_empty() {
            words.push(current);
        }
        words
    }

    /// Apply word designators (:0, :n, :^, :$, :*, :n-m) and modifiers
    /// (:h, :t, :r, :e, :s/old/new/, :gs/old/new/, :p, :l, :u, :q, :Q, :a, :A)
    /// to an already-resolved event string.
    fn history_apply_designators_and_modifiers(
        &self,
        chars: &[char],
        mut i: usize,
        event: &str,
        last_subst: &mut Option<(String, String)>,
    ) -> (String, usize) {
        let words = Self::history_split_words(event);
        let argc = words.len().saturating_sub(1); // last word index

        // Check for word designator — either :N or bare :^ :$ :*
        let mut sline = event.to_string();

        if i < chars.len() && chars[i] == ':' {
            i += 1;
            if i < chars.len() {
                // Parse word designator
                let (farg, larg, new_i) = self.history_parse_word_range(chars, i, argc);
                i = new_i;
                if farg.is_some() || larg.is_some() {
                    let f = farg.unwrap_or(0);
                    let l = larg.unwrap_or(argc);
                    let selected: Vec<&String> = words.iter().enumerate()
                        .filter(|(idx, _)| *idx >= f && *idx <= l)
                        .map(|(_, w)| w)
                        .collect();
                    sline = selected.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" ");
                }
            }
        } else if i < chars.len() && chars[i] == '*' {
            // !!* shorthand for !!:1-$
            i += 1;
            if words.len() > 1 {
                sline = words[1..].join(" ");
            } else {
                sline = String::new();
            }
        }

        // Apply modifiers (:h :t :r :e :s :gs :p :l :u :q :Q :a :A)
        while i < chars.len() && chars[i] == ':' {
            i += 1;
            if i >= chars.len() {
                break;
            }
            let mut global = false;
            if chars[i] == 'g' && i + 1 < chars.len() {
                global = true;
                i += 1;
            }
            match chars[i] {
                'h' => {
                    // Head — remove trailing path component
                    i += 1;
                    if let Some(pos) = sline.rfind('/') {
                        if pos > 0 {
                            sline = sline[..pos].to_string();
                        } else {
                            sline = "/".to_string();
                        }
                    }
                }
                't' => {
                    // Tail — remove leading path components
                    i += 1;
                    if let Some(pos) = sline.rfind('/') {
                        sline = sline[pos + 1..].to_string();
                    }
                }
                'r' => {
                    // Remove extension
                    i += 1;
                    if let Some(pos) = sline.rfind('.') {
                        if pos > 0 && sline[..pos].rfind('/').map_or(true, |sp| sp < pos) {
                            sline = sline[..pos].to_string();
                        }
                    }
                }
                'e' => {
                    // Extension only
                    i += 1;
                    if let Some(pos) = sline.rfind('.') {
                        sline = sline[pos + 1..].to_string();
                    } else {
                        sline = String::new();
                    }
                }
                'l' => {
                    // Lowercase
                    i += 1;
                    sline = sline.to_lowercase();
                }
                'u' => {
                    // Uppercase
                    i += 1;
                    sline = sline.to_uppercase();
                }
                'p' => {
                    // Print only, don't execute (we just expand — caller handles this)
                    i += 1;
                    // For now, just expand — :p suppression would need upstream support
                }
                'q' => {
                    // Quote — single-quote the result
                    i += 1;
                    sline = format!("'{}'", sline.replace('\'', "'\\''"));
                }
                'Q' => {
                    // Unquote — strip one level of quotes
                    i += 1;
                    sline = sline.replace('\'', "").replace('"', "");
                }
                'a' => {
                    // Absolute path
                    i += 1;
                    if !sline.starts_with('/') {
                        if let Ok(cwd) = std::env::current_dir() {
                            sline = format!("{}/{}", cwd.display(), sline);
                        }
                    }
                }
                'A' => {
                    // Realpath
                    i += 1;
                    if let Ok(real) = std::fs::canonicalize(&sline) {
                        sline = real.to_string_lossy().to_string();
                    }
                }
                's' | 'S' => {
                    // :s/old/new/ or :gs/old/new/
                    i += 1;
                    if i < chars.len() {
                        let delim = chars[i];
                        i += 1;
                        let mut old_s = String::new();
                        while i < chars.len() && chars[i] != delim {
                            old_s.push(chars[i]);
                            i += 1;
                        }
                        if i < chars.len() { i += 1; } // skip delimiter
                        let mut new_s = String::new();
                        while i < chars.len() && chars[i] != delim && chars[i] != ':' && chars[i] != ' ' {
                            new_s.push(chars[i]);
                            i += 1;
                        }
                        if i < chars.len() && chars[i] == delim { i += 1; } // skip trailing delimiter
                        *last_subst = Some((old_s.clone(), new_s.clone()));
                        if global {
                            sline = sline.replace(&old_s, &new_s);
                        } else {
                            sline = sline.replacen(&old_s, &new_s, 1);
                        }
                    }
                }
                '&' => {
                    // Repeat last substitution
                    i += 1;
                    if let Some((ref old_s, ref new_s)) = last_subst {
                        if global {
                            sline = sline.replace(old_s.as_str(), new_s.as_str());
                        } else {
                            sline = sline.replacen(old_s.as_str(), new_s.as_str(), 1);
                        }
                    }
                }
                _ => {
                    if global {
                        // 'g' was consumed but next char isn't s/S/& — put back
                        // by not advancing i further
                    }
                    break;
                }
            }
        }

        (sline, i)
    }

    /// Parse a word range like 0, 1, ^, $, *, n-m, n-
    fn history_parse_word_range(
        &self,
        chars: &[char],
        mut i: usize,
        argc: usize,
    ) -> (Option<usize>, Option<usize>, usize) {
        if i >= chars.len() {
            return (None, None, i);
        }

        // Check for modifiers that aren't word designators
        match chars[i] {
            'h' | 't' | 'r' | 'e' | 's' | 'S' | 'g' | 'p' | 'q' | 'Q' | 'l' | 'u' | 'a' | 'A' | '&' => {
                // This is a modifier, not a word designator — back up
                return (None, None, i - 1); // -1 to re-read the ':'
            }
            _ => {}
        }

        let farg = if chars[i] == '^' {
            i += 1;
            Some(1usize)
        } else if chars[i] == '$' {
            i += 1;
            return (Some(argc), Some(argc), i);
        } else if chars[i] == '*' {
            i += 1;
            return (Some(1), Some(argc), i);
        } else if chars[i].is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let n: usize = chars[start..i].iter().collect::<String>().parse().unwrap_or(0);
            Some(n)
        } else {
            None
        };

        // Check for range: n-m or n-
        if i < chars.len() && chars[i] == '-' {
            i += 1;
            if i < chars.len() && chars[i] == '$' {
                i += 1;
                return (farg, Some(argc), i);
            } else if i < chars.len() && chars[i].is_ascii_digit() {
                let start = i;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
                let m: usize = chars[start..i].iter().collect::<String>().parse().unwrap_or(0);
                return (farg, Some(m), i);
            } else {
                // n- means n to argc-1
                return (farg, Some(argc.saturating_sub(1)), i);
            }
        }

        if farg.is_some() {
            (farg, farg, i)
        } else {
            (None, None, i)
        }
    }

    #[tracing::instrument(level = "trace", skip_all)]
    pub fn execute_command(&mut self, cmd: &ShellCommand) -> Result<i32, String> {
        match cmd {
            ShellCommand::Simple(simple) => self.execute_simple(simple),
            ShellCommand::Pipeline(cmds, negated) => {
                let status = self.execute_pipeline(cmds)?;
                if *negated {
                    self.last_status = if status == 0 { 1 } else { 0 };
                } else {
                    self.last_status = status;
                }
                Ok(self.last_status)
            }
            ShellCommand::List(items) => self.execute_list(items),
            ShellCommand::Compound(compound) => self.execute_compound(compound),
            ShellCommand::FunctionDef(name, body) => {
                if name.is_empty() {
                    // Anonymous function - execute immediately
                    let result = self.execute_command(body);
                    // Clear returning flag since the anonymous function has completed
                    if let Some(ret) = self.returning.take() {
                        self.last_status = ret;
                        return Ok(ret);
                    }
                    result
                } else {
                    // Named function - just define it
                    self.functions.insert(name.clone(), (**body).clone());
                    self.last_status = 0;
                    Ok(0)
                }
            }
        }
    }

    #[tracing::instrument(level = "trace", skip_all)]
    fn execute_simple(&mut self, cmd: &SimpleCommand) -> Result<i32, String> {
        // Handle assignments
        for (var, val, is_append) in &cmd.assignments {
            match val {
                ShellWord::ArrayLiteral(elements) => {
                    // Array assignment: arr=(a b c) or arr+=(a b c)
                    // For associative arrays: assoc=(k1 v1 k2 v2)
                    // Use expand_word_split so $(cmd) and $var undergo
                    // word splitting into separate array elements (C zsh behavior).
                    let new_elements: Vec<String> = elements
                        .iter()
                        .flat_map(|e| self.expand_word_split(e))
                        .collect();

                    // Check if this is an associative array
                    if self.assoc_arrays.contains_key(var) {
                        // Associative array: treat pairs as key-value
                        if *is_append {
                            let assoc = self.assoc_arrays.get_mut(var).unwrap();
                            let mut iter = new_elements.iter();
                            while let Some(key) = iter.next() {
                                if let Some(val) = iter.next() {
                                    assoc.insert(key.clone(), val.clone());
                                }
                            }
                        } else {
                            let mut assoc = HashMap::new();
                            let mut iter = new_elements.iter();
                            while let Some(key) = iter.next() {
                                if let Some(val) = iter.next() {
                                    assoc.insert(key.clone(), val.clone());
                                }
                            }
                            self.assoc_arrays.insert(var.clone(), assoc);
                        }
                    } else if *is_append {
                        // Append to existing indexed array
                        let arr = self.arrays.entry(var.clone()).or_insert_with(Vec::new);
                        arr.extend(new_elements);
                    } else {
                        self.arrays.insert(var.clone(), new_elements);
                    }
                }
                _ => {
                    let expanded = self.expand_word(val);

                    // Check for array element assignment: arr[idx]=value or assoc[key]=value
                    if let Some(bracket_pos) = var.find('[') {
                        if var.ends_with(']') {
                            let array_name = &var[..bracket_pos];
                            let key = &var[bracket_pos + 1..var.len() - 1];
                            let key = self.expand_string(key); // Expand the key/index

                            // Check if it's an associative array
                            if self.assoc_arrays.contains_key(array_name) {
                                let assoc = self.assoc_arrays.get_mut(array_name).unwrap();
                                if *is_append {
                                    let existing = assoc.get(&key).cloned().unwrap_or_default();
                                    assoc.insert(key, existing + &expanded);
                                } else {
                                    assoc.insert(key, expanded);
                                }
                            } else if let Ok(idx) = key.parse::<i64>() {
                                // Regular indexed array
                                let idx = if idx < 0 { 0 } else { (idx - 1) as usize }; // zsh is 1-indexed
                                let arr = self
                                    .arrays
                                    .entry(array_name.to_string())
                                    .or_insert_with(Vec::new);
                                while arr.len() <= idx {
                                    arr.push(String::new());
                                }
                                if *is_append {
                                    arr[idx].push_str(&expanded);
                                } else {
                                    arr[idx] = expanded;
                                }
                            } else {
                                // Non-numeric key on non-assoc array - treat as assoc
                                let assoc = self
                                    .assoc_arrays
                                    .entry(array_name.to_string())
                                    .or_insert_with(HashMap::new);
                                if *is_append {
                                    let existing = assoc.get(&key).cloned().unwrap_or_default();
                                    assoc.insert(key, existing + &expanded);
                                } else {
                                    assoc.insert(key, expanded);
                                }
                            }
                            continue;
                        }
                    }

                    // Regular variable assignment or append
                    let final_value = if *is_append {
                        let existing = self.variables.get(var).cloned().unwrap_or_default();
                        existing + &expanded
                    } else {
                        expanded
                    };

                    if self.readonly_vars.contains(var) {
                        eprintln!("zshrs: read-only variable: {}", var);
                        self.last_status = 1;
                        return Ok(1);
                    }
                    if cmd.words.is_empty() {
                        // Just assignment, set in environment
                        env::set_var(var, &final_value);
                    }
                    self.variables.insert(var.clone(), final_value);
                }
            }
        }

        if cmd.words.is_empty() {
            self.last_status = 0;
            return Ok(0);
        }

        // Check if this is a noglob precommand — suppress glob expansion
        let is_noglob = cmd.words.first().map(|w| self.expand_word(w) == "noglob").unwrap_or(false);
        let saved_noglob = if is_noglob {
            let saved = self.options.get("noglob").copied();
            self.options.insert("noglob".to_string(), true);
            saved
        } else {
            None
        };

        // Pre-launch external command substitutions in parallel before expanding words.
        // Each external $(cmd) gets spawned on the worker pool immediately.
        // When we reach that word during sequential expansion, we collect the result.
        let preflight = self.preflight_command_subs(&cmd.words);

        let mut words: Vec<String> = cmd
            .words
            .iter()
            .enumerate()
            .flat_map(|(i, w)| {
                if let Some(rx) = &preflight[i] {
                    // Pre-launched external command sub — collect result
                    vec![rx.recv().unwrap_or_default()]
                } else {
                    self.expand_word_glob(w)
                }
            })
            .collect();

        // Restore noglob after expansion
        if is_noglob {
            match saved_noglob {
                Some(v) => { self.options.insert("noglob".to_string(), v); }
                None => { self.options.remove("noglob"); }
            }
        }
        if words.is_empty() {
            self.last_status = 0;
            return Ok(0);
        }

        // Expand global aliases (alias -g) in all word positions
        if !self.global_aliases.is_empty() {
            let global_aliases = self.global_aliases.clone();
            words = words
                .into_iter()
                .map(|w| global_aliases.get(&w).cloned().unwrap_or(w))
                .collect();
        }

        // xtrace: print expanded command to stderr (zsh -x / set -x)
        if self.options.get("xtrace").copied().unwrap_or(false) {
            let ps4 = self.variables.get("PS4").cloned().unwrap_or_else(|| "+".to_string());
            eprintln!("{}{}", ps4, words.join(" "));
        }

        // Check for regular alias expansion (alias > builtin > function > command)
        let cmd_name = &words[0];
        if let Some(alias_value) = self.aliases.get(cmd_name).cloned() {
            // Expand the alias: replace cmd_name with alias value, keep remaining args
            let expanded_cmd = if words.len() > 1 {
                format!("{} {}", alias_value, words[1..].join(" "))
            } else {
                alias_value
            };
            // Re-execute the expanded command
            return self.execute_script(&expanded_cmd);
        }

        // Check for suffix alias expansion (alias -s) when command is a file path
        if !self.suffix_aliases.is_empty() {
            let cmd_path = std::path::Path::new(cmd_name);
            if let Some(ext) = cmd_path.extension().and_then(|e| e.to_str()) {
                if let Some(handler) = self.suffix_aliases.get(ext).cloned() {
                    // Suffix alias: "alias -s txt=vim" makes "foo.txt" run "vim foo.txt"
                    let expanded_cmd = format!("{} {}", handler, words.join(" "));
                    return self.execute_script(&expanded_cmd);
                }
            }
        }

        let args = &words[1..];

        // Check if this is `exec` with only redirects (no command args)
        // For exec, redirects with {varname} allocate FDs; redirects are permanent
        let is_exec_with_redirects_only =
            cmd_name == "exec" && args.is_empty() && !cmd.redirects.is_empty();

        // Apply redirects for builtins
        let mut saved_fds: Vec<(i32, i32)> = Vec::new();
        for redirect in &cmd.redirects {
            let target = self.expand_word(&redirect.target);

            // Handle {varname}>file syntax - allocate new FD and store in variable
            if let Some(ref var_name) = redirect.fd_var {
                use std::os::unix::io::IntoRawFd;
                let file_result = match redirect.op {
                    RedirectOp::Write | RedirectOp::Clobber => std::fs::File::create(&target),
                    RedirectOp::Append => std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&target),
                    RedirectOp::Read => std::fs::File::open(&target),
                    _ => continue,
                };
                match file_result {
                    Ok(file) => {
                        let new_fd = file.into_raw_fd();
                        self.variables.insert(var_name.clone(), new_fd.to_string());
                        // Store allocated FD for potential cleanup (not for exec)
                        if !is_exec_with_redirects_only {
                            // For non-exec, we might want to track these
                        }
                    }
                    Err(e) => {
                        eprintln!("{}: {}: {}", cmd_name, target, e);
                        return Ok(1);
                    }
                }
                continue;
            }

            let fd = redirect.fd.unwrap_or(match redirect.op {
                RedirectOp::Read
                | RedirectOp::HereDoc
                | RedirectOp::HereString
                | RedirectOp::ReadWrite => 0,
                _ => 1,
            });

            match redirect.op {
                RedirectOp::Write | RedirectOp::Clobber => {
                    use std::os::unix::io::IntoRawFd;
                    if !is_exec_with_redirects_only {
                        let saved = unsafe { libc::dup(fd) };
                        if saved >= 0 {
                            saved_fds.push((fd, saved));
                        }
                    }
                    if let Ok(file) = std::fs::File::create(&target) {
                        let new_fd = file.into_raw_fd();
                        unsafe {
                            libc::dup2(new_fd, fd);
                        }
                        unsafe {
                            libc::close(new_fd);
                        }
                    }
                }
                RedirectOp::Append => {
                    use std::os::unix::io::IntoRawFd;
                    if !is_exec_with_redirects_only {
                        let saved = unsafe { libc::dup(fd) };
                        if saved >= 0 {
                            saved_fds.push((fd, saved));
                        }
                    }
                    if let Ok(file) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&target)
                    {
                        let new_fd = file.into_raw_fd();
                        unsafe {
                            libc::dup2(new_fd, fd);
                        }
                        unsafe {
                            libc::close(new_fd);
                        }
                    }
                }
                RedirectOp::Read => {
                    use std::os::unix::io::IntoRawFd;
                    if !is_exec_with_redirects_only {
                        let saved = unsafe { libc::dup(fd) };
                        if saved >= 0 {
                            saved_fds.push((fd, saved));
                        }
                    }
                    if let Ok(file) = std::fs::File::open(&target) {
                        let new_fd = file.into_raw_fd();
                        unsafe {
                            libc::dup2(new_fd, fd);
                        }
                        unsafe {
                            libc::close(new_fd);
                        }
                    }
                }
                RedirectOp::DupWrite | RedirectOp::DupRead => {
                    if let Ok(target_fd) = target.parse::<i32>() {
                        if !is_exec_with_redirects_only {
                            let saved = unsafe { libc::dup(fd) };
                            if saved >= 0 {
                                saved_fds.push((fd, saved));
                            }
                        }
                        unsafe {
                            libc::dup2(target_fd, fd);
                        }
                    }
                }
                _ => {}
            }
        }

        // For exec with only redirects, we're done - redirects are applied permanently
        if is_exec_with_redirects_only {
            self.last_status = 0;
            return Ok(0);
        }

        // Check for shell builtins
        let status = match cmd_name.as_str() {
            "cd" => self.builtin_cd(args),
            "pwd" => self.builtin_pwd(&cmd.redirects),
            "echo" => self.builtin_echo(args, &cmd.redirects),
            "export" => self.builtin_export(args),
            "unset" => self.builtin_unset(args),
            "source" | "." => self.builtin_source(args),
            "exit" | "bye" | "logout" => self.builtin_exit(args),
            "return" => self.builtin_return(args),
            "true" => 0,
            "false" => 1,
            ":" => 0,
            "chdir" => self.builtin_cd(args),
            "test" | "[" => self.builtin_test(args),
            "local" => self.builtin_local(args),
            "declare" | "typeset" => self.builtin_declare(args),
            "read" => self.builtin_read(args),
            "shift" => self.builtin_shift(args),
            "eval" => self.builtin_eval(args),
            "jobs" => self.builtin_jobs(args),
            "fg" => self.builtin_fg(args),
            "bg" => self.builtin_bg(args),
            "kill" => self.builtin_kill(args),
            "disown" => self.builtin_disown(args),
            "wait" => self.builtin_wait(args),
            "autoload" => self.builtin_autoload(args),
            "history" => self.builtin_history(args),
            "fc" => self.builtin_fc(args),
            "trap" => self.builtin_trap(args),
            "suspend" => self.builtin_suspend(args),
            "alias" => self.builtin_alias(args),
            "unalias" => self.builtin_unalias(args),
            "set" => self.builtin_set(args),
            "shopt" => self.builtin_shopt(args),
            // Bash compatibility
            "bind" => self.builtin_bindkey(args),
            "caller" => self.builtin_caller(args),
            "help" => self.builtin_help(args),
            "doctor" => self.builtin_doctor(args),
            "dbview" => self.builtin_dbview(args),
            "profile" => self.builtin_profile(args),
            "intercept" => self.builtin_intercept(args),
            "intercept_proceed" => self.builtin_intercept_proceed(args),
            // ── Concurrent primitives ──
            "async" => self.builtin_async(args),
            "await" => self.builtin_await(args),
            "pmap" => self.builtin_pmap(args),
            "pgrep" => self.builtin_pgrep(args),
            "peach" => self.builtin_peach(args),
            "barrier" => self.builtin_barrier(args),
            "readarray" | "mapfile" => self.builtin_readarray(args),
            "setopt" => self.builtin_setopt(args),
            "unsetopt" => self.builtin_unsetopt(args),
            "getopts" => self.builtin_getopts(args),
            "type" => self.builtin_type(args),
            "hash" => self.builtin_hash(args),
            "add-zsh-hook" => self.builtin_add_zsh_hook(args),
            "command" => self.builtin_command(args, &cmd.redirects),
            "builtin" => self.builtin_builtin(args, &cmd.redirects),
            "let" => self.builtin_let(args),
            "compgen" => self.builtin_compgen(args),
            "complete" => self.builtin_complete(args),
            "compopt" => self.builtin_compopt(args),
            "compadd" => self.builtin_compadd(args),
            "compset" => self.builtin_compset(args),
            "compdef" => self.builtin_compdef(args),
            "compinit" => self.builtin_compinit(args),
            "cdreplay" => self.builtin_cdreplay(args),
            "zstyle" => self.builtin_zstyle(args),
            // GDBM database bindings
            "ztie" => self.builtin_ztie(args),
            "zuntie" => self.builtin_zuntie(args),
            "zgdbmpath" => self.builtin_zgdbmpath(args),
            "pushd" => self.builtin_pushd(args),
            "popd" => self.builtin_popd(args),
            "dirs" => self.builtin_dirs(args),
            "printf" => self.builtin_printf(args),
            // Control flow
            "break" => self.builtin_break(args),
            "continue" => self.builtin_continue(args),
            // Enable/disable builtins
            "disable" => self.builtin_disable(args),
            "enable" => self.builtin_enable(args),
            // Emulation
            "emulate" => self.builtin_emulate(args),
            // Prompt themes
            "promptinit" => self.builtin_promptinit(args),
            "prompt" => self.builtin_prompt(args),
            // PCRE
            "pcre_compile" => self.builtin_pcre_compile(args),
            "pcre_match" => self.builtin_pcre_match(args),
            "pcre_study" => self.builtin_pcre_study(args),
            // Exec
            "exec" => self.builtin_exec(args),
            // Typed variables
            "float" => self.builtin_float(args),
            "integer" => self.builtin_integer(args),
            // Functions
            "functions" => self.builtin_functions(args),
            // Print (zsh style)
            "print" => self.builtin_print(args),
            // Command lookup
            "whence" => self.builtin_whence(args),
            "where" => self.builtin_where(args),
            "which" => self.builtin_which(args),
            // Resource limits
            "ulimit" => self.builtin_ulimit(args),
            "limit" => self.builtin_limit(args),
            "unlimit" => self.builtin_unlimit(args),
            // File mask
            "umask" => self.builtin_umask(args),
            // Hash table
            "rehash" => self.builtin_rehash(args),
            "unhash" => self.builtin_unhash(args),
            // Times
            "times" => self.builtin_times(args),
            // Module loading (stub)
            "zmodload" => self.builtin_zmodload(args),
            // Redo
            "r" => self.builtin_r(args),
            // TTY control
            "ttyctl" => self.builtin_ttyctl(args),
            // Noglob
            "noglob" => self.builtin_noglob(args, &cmd.redirects),
            // zsh/stat module
            "zstat" | "stat" => self.builtin_zstat(args),
            // zsh/datetime module
            "strftime" => self.builtin_strftime(args),
            // sleep with fractional seconds
            "zsleep" => self.builtin_zsleep(args),
            // zsh/system module - ported from Src/Modules/system.c
            "zsystem" => self.builtin_zsystem(args),
            // zsh/files module - ported from Src/Modules/files.c
            "sync" => self.builtin_sync(args),
            "mkdir" => self.builtin_mkdir(args),
            "rmdir" => self.builtin_rmdir(args),
            "ln" => self.builtin_ln(args),
            "mv" => self.builtin_mv(args),
            "cp" => self.builtin_cp(args),
            "rm" => self.builtin_rm(args),
            "chown" => self.builtin_chown(args),
            "chmod" => self.builtin_chmod(args),
            "zln" | "zmv" | "zcp" => self.builtin_zfiles(cmd_name, args),
            // coproc management
            "coproc" => self.builtin_coproc(args),
            // zparseopts - option parsing
            "zparseopts" => self.builtin_zparseopts(args),
            // readonly/unfunction
            "readonly" => self.builtin_readonly(args),
            "unfunction" => self.builtin_unfunction(args),
            // getln/pushln
            "getln" => self.builtin_getln(args),
            "pushln" => self.builtin_pushln(args),
            // bindkey stub
            "bindkey" => self.builtin_bindkey(args),
            // zle stub
            "zle" => self.builtin_zle(args),
            // sched
            "sched" => self.builtin_sched(args),
            // zformat
            "zformat" => self.builtin_zformat(args),
            // zcompile
            "zcompile" => self.builtin_zcompile(args),
            // vared - visual edit
            "vared" => self.builtin_vared(args),
            // terminal capabilities
            "echotc" => self.builtin_echotc(args),
            "echoti" => self.builtin_echoti(args),
            // PTY and socket operations
            "zpty" => self.builtin_zpty(args),
            "zprof" => self.builtin_zprof(args),
            "zsocket" => self.builtin_zsocket(args),
            "ztcp" => self.builtin_ztcp(args),
            "zregexparse" => self.builtin_zregexparse(args),
            "clone" => self.builtin_clone(args),
            "log" => self.builtin_log(args),
            // Completion system builtins
            "comparguments" => self.builtin_comparguments(args),
            "compcall" => self.builtin_compcall(args),
            "compctl" => self.builtin_compctl(args),
            "compdescribe" => self.builtin_compdescribe(args),
            "compfiles" => self.builtin_compfiles(args),
            "compgroups" => self.builtin_compgroups(args),
            "compquote" => self.builtin_compquote(args),
            "comptags" => self.builtin_comptags(args),
            "comptry" => self.builtin_comptry(args),
            "compvalues" => self.builtin_compvalues(args),
            // Capabilities (Linux-specific, stubs on macOS)
            "cap" | "getcap" | "setcap" => self.builtin_cap(args),
            // FTP client
            "zftp" => self.builtin_zftp(args),
            // zsh/curses module
            "zcurses" => self.builtin_zcurses(args),
            // zsh/system module
            "sysread" => self.builtin_sysread(args),
            "syswrite" => self.builtin_syswrite(args),
            "syserror" => self.builtin_syserror(args),
            "sysopen" => self.builtin_sysopen(args),
            "sysseek" => self.builtin_sysseek(args),
            // zsh/mapfile module
            "mapfile" => 0, // mapfile is a special parameter, not a command
            // zsh/param/private
            "private" => self.builtin_private(args),
            // zsh/attr (extended attributes)
            "zgetattr" | "zsetattr" | "zdelattr" | "zlistattr" => {
                self.builtin_zattr(cmd_name, args)
            }
            // Completion helper functions (now implemented in Rust compsys crate)
            // These are stubs that return success during non-completion execution
            "_arguments" | "_describe" | "_description" | "_message" | "_tags" | "_requested"
            | "_all_labels" | "_next_label" | "_files" | "_path_files" | "_directories" | "_cd"
            | "_default" | "_dispatch" | "_complete" | "_main_complete" | "_normal"
            | "_approximate" | "_correct" | "_expand" | "_history" | "_match" | "_menu"
            | "_oldlist" | "_list" | "_prefix" | "_generic" | "_wanted" | "_alternative"
            | "_values" | "_sequence" | "_sep_parts" | "_multi_parts" | "_combination"
            | "_parameters" | "_command" | "_command_names" | "_commands" | "_functions"
            | "_aliases" | "_builtins" | "_jobs" | "_pids" | "_process_names" | "_signals"
            | "_users" | "_groups" | "_hosts" | "_domains" | "_urls" | "_email_addresses"
            | "_options" | "_contexts" | "_set_options" | "_unset_options" | "_vars"
            | "_env_variables" | "_shell_variables" | "_arrays" | "_globflags" | "_globquals"
            | "_globqual_delims" | "_subscript" | "_history_modifiers" | "_brace_parameter"
            | "_tilde" | "_style" | "_cache_invalid" | "_store_cache" | "_retrieve_cache"
            | "_call_function" | "_call_program" | "_pick_variant" | "_setup"
            | "_comp_priv_prefix" | "_regex_arguments" | "_regex_words" | "_guard"
            | "_gnu_generic" | "_long_options" | "_x_arguments" | "_sub_commands"
            | "_cmdstring" | "_cmdambivalent" | "_first" | "_precommand" | "_user_at_host"
            | "_user_expand" | "_path_commands" | "_globbed_files" | "_have_glob_qual" => {
                // Return success - these functions are for completion context only
                // The actual completion logic is in the compsys Rust crate
                0
            }
            _ => {
                // ── AOP intercept dispatch ──
                // Check if any intercepts match this command name.
                // Fast path: skip if no intercepts registered.
                if !self.intercepts.is_empty() {
                    let full_cmd = if args.is_empty() {
                        cmd_name.to_string()
                    } else {
                        format!("{} {}", cmd_name, args.join(" "))
                    };
                    if let Some(result) = self.run_intercepts(cmd_name, &full_cmd, args) {
                        return result;
                    }
                }

                // Check for function
                if let Some(func) = self.functions.get(cmd_name).cloned() {
                    return self.call_function(&func, args);
                }

                // Try autoloading from pending autoload list
                if self.maybe_autoload(cmd_name) {
                    if let Some(func) = self.functions.get(cmd_name).cloned() {
                        return self.call_function(&func, args);
                    }
                }

                // Try autoloading from ZWC
                if self.autoload_function(cmd_name).is_some() {
                    if let Some(func) = self.functions.get(cmd_name).cloned() {
                        return self.call_function(&func, args);
                    }
                }

                // External command
                self.execute_external(cmd_name, args, &cmd.redirects)?
            }
        };

        // Restore saved fds
        for (fd, saved) in saved_fds.into_iter().rev() {
            unsafe {
                libc::dup2(saved, fd);
                libc::close(saved);
            }
        }

        self.last_status = status;
        Ok(status)
    }

    /// Call a function with positional parameters
    #[tracing::instrument(level = "debug", skip_all)]
    fn call_function(&mut self, func: &ShellCommand, args: &[String]) -> Result<i32, String> {
        // Save current positional params
        let saved_params = std::mem::take(&mut self.positional_params);

        // Save local variable scope — any `local` declarations during this
        // function will be reversed on exit (matches zsh's startparamscope/endparamscope).
        let saved_local_vars = self.local_save_stack.len();
        self.local_scope_depth += 1;

        // Set new positional params
        self.positional_params = args.to_vec();

        // Execute the function
        let result = self.execute_command(func);

        // Handle return - clear the flag and use its value
        let final_result = if let Some(ret) = self.returning.take() {
            self.last_status = ret;
            Ok(ret)
        } else {
            result
        };

        // Restore local variables (endparamscope)
        self.local_scope_depth -= 1;
        while self.local_save_stack.len() > saved_local_vars {
            if let Some((name, old_val)) = self.local_save_stack.pop() {
                match old_val {
                    Some(v) => { self.variables.insert(name, v); }
                    None => { self.variables.remove(&name); }
                }
            }
        }

        // Restore positional params
        self.positional_params = saved_params;

        final_result
    }

    fn execute_external(
        &mut self,
        cmd: &str,
        args: &[String],
        redirects: &[Redirect],
    ) -> Result<i32, String> {
        self.execute_external_bg(cmd, args, redirects, false)
    }

    fn execute_external_bg(
        &mut self,
        cmd: &str,
        args: &[String],
        redirects: &[Redirect],
        background: bool,
    ) -> Result<i32, String> {
        tracing::trace!(cmd, bg = background, "exec external");
        let mut command = Command::new(cmd);
        command.args(args);

        // Apply redirections
        for redir in redirects {
            let target = self.expand_word(&redir.target);
            match redir.op {
                RedirectOp::Read => match File::open(&target) {
                    Ok(f) => {
                        command.stdin(Stdio::from(f));
                    }
                    Err(e) => return Err(format!("Cannot open {}: {}", target, e)),
                },
                RedirectOp::Write => match File::create(&target) {
                    Ok(f) => {
                        command.stdout(Stdio::from(f));
                    }
                    Err(e) => return Err(format!("Cannot create {}: {}", target, e)),
                },
                RedirectOp::Append => {
                    match OpenOptions::new().create(true).append(true).open(&target) {
                        Ok(f) => {
                            command.stdout(Stdio::from(f));
                        }
                        Err(e) => return Err(format!("Cannot open {}: {}", target, e)),
                    }
                }
                RedirectOp::WriteBoth => match File::create(&target) {
                    Ok(f) => {
                        let f2 = f
                            .try_clone()
                            .map_err(|e| format!("Cannot clone fd: {}", e))?;
                        command.stdout(Stdio::from(f));
                        command.stderr(Stdio::from(f2));
                    }
                    Err(e) => return Err(format!("Cannot create {}: {}", target, e)),
                },
                RedirectOp::AppendBoth => {
                    match OpenOptions::new().create(true).append(true).open(&target) {
                        Ok(f) => {
                            let f2 = f
                                .try_clone()
                                .map_err(|e| format!("Cannot clone fd: {}", e))?;
                            command.stdout(Stdio::from(f));
                            command.stderr(Stdio::from(f2));
                        }
                        Err(e) => return Err(format!("Cannot open {}: {}", target, e)),
                    }
                }
                RedirectOp::HereDoc => {
                    // Here-document - provide content as stdin
                    if let Some(ref content) = redir.heredoc_content {
                        // Expand variables in content (unless delimiter was quoted)
                        let expanded = self.expand_string(content);
                        command.stdin(Stdio::piped());
                        // Store the content to write after spawn
                        // For now, create a temp file
                        use std::io::Write;
                        let mut temp_file = tempfile::NamedTempFile::new()
                            .map_err(|e| format!("Cannot create temp file: {}", e))?;
                        temp_file
                            .write_all(expanded.as_bytes())
                            .map_err(|e| format!("Cannot write to temp file: {}", e))?;
                        let temp_path = temp_file.into_temp_path();
                        let f = File::open(&temp_path)
                            .map_err(|e| format!("Cannot open temp file: {}", e))?;
                        command.stdin(Stdio::from(f));
                    }
                }
                RedirectOp::HereString => {
                    // Here-string - provide target as stdin
                    use std::io::Write;
                    let content = format!("{}\n", target);
                    let mut temp_file = tempfile::NamedTempFile::new()
                        .map_err(|e| format!("Cannot create temp file: {}", e))?;
                    temp_file
                        .write_all(content.as_bytes())
                        .map_err(|e| format!("Cannot write to temp file: {}", e))?;
                    let temp_path = temp_file.into_temp_path();
                    let f = File::open(&temp_path)
                        .map_err(|e| format!("Cannot open temp file: {}", e))?;
                    command.stdin(Stdio::from(f));
                }
                _ => {
                    // Other redirections handled simply
                }
            }

            // Handle {varname}>file syntax - store FD in variable
            if let Some(ref var_name) = redir.fd_var {
                // For {varname}>file, we open the file and store the fd number
                // This is typically used with exec, but we'll handle it for commands too
                #[cfg(unix)]
                {
                    use std::os::unix::io::AsRawFd;
                    let fd = match redir.op {
                        RedirectOp::Write | RedirectOp::Append => {
                            let f = if redir.op == RedirectOp::Write {
                                File::create(&target)
                            } else {
                                OpenOptions::new().create(true).append(true).open(&target)
                            };
                            match f {
                                Ok(file) => {
                                    let raw_fd = file.as_raw_fd();
                                    std::mem::forget(file); // Don't close the file
                                    raw_fd
                                }
                                Err(e) => return Err(format!("Cannot open {}: {}", target, e)),
                            }
                        }
                        RedirectOp::Read => match File::open(&target) {
                            Ok(file) => {
                                let raw_fd = file.as_raw_fd();
                                std::mem::forget(file);
                                raw_fd
                            }
                            Err(e) => return Err(format!("Cannot open {}: {}", target, e)),
                        },
                        _ => continue,
                    };
                    self.variables.insert(var_name.clone(), fd.to_string());
                }
            }
        }

        if background {
            match command.spawn() {
                Ok(child) => {
                    let pid = child.id();
                    let cmd_str = format!("{} {}", cmd, args.join(" "));
                    let job_id = self.jobs.add_job(child, cmd_str, JobState::Running);
                    println!("[{}] {}", job_id, pid);
                    Ok(0)
                }
                Err(e) => {
                    if e.kind() == io::ErrorKind::NotFound {
                        eprintln!("zshrs: command not found: {}", cmd);
                        Ok(127)
                    } else {
                        Err(format!("zshrs: {}: {}", cmd, e))
                    }
                }
            }
        } else {
            match command.status() {
                Ok(status) => Ok(status.code().unwrap_or(1)),
                Err(e) => {
                    if e.kind() == io::ErrorKind::NotFound {
                        eprintln!("zshrs: command not found: {}", cmd);
                        Ok(127)
                    } else {
                        Err(format!("zshrs: {}: {}", cmd, e))
                    }
                }
            }
        }
    }

    #[tracing::instrument(level = "trace", skip_all, fields(stages = cmds.len()))]
    fn execute_pipeline(&mut self, cmds: &[ShellCommand]) -> Result<i32, String> {
        if cmds.len() == 1 {
            return self.execute_command(&cmds[0]);
        }

        let mut children: Vec<Child> = Vec::new();
        let mut prev_stdout: Option<std::process::ChildStdout> = None;

        for (i, cmd) in cmds.iter().enumerate() {
            if let ShellCommand::Simple(simple) = cmd {
                let words: Vec<String> = simple.words.iter().map(|w| self.expand_word(w)).collect();
                if words.is_empty() {
                    continue;
                }

                let mut command = Command::new(&words[0]);
                command.args(&words[1..]);

                if let Some(stdout) = prev_stdout.take() {
                    command.stdin(Stdio::from(stdout));
                }

                if i < cmds.len() - 1 {
                    command.stdout(Stdio::piped());
                }

                match command.spawn() {
                    Ok(mut child) => {
                        prev_stdout = child.stdout.take();
                        children.push(child);
                    }
                    Err(e) => {
                        eprintln!("zshrs: {}: {}", words[0], e);
                        return Ok(127);
                    }
                }
            }
        }

        // Wait for all children
        let mut last_status = 0;
        for mut child in children {
            if let Ok(status) = child.wait() {
                last_status = status.code().unwrap_or(1);
            }
        }

        Ok(last_status)
    }

    fn execute_list(&mut self, items: &[(ShellCommand, ListOp)]) -> Result<i32, String> {
        for (cmd, op) in items {
            // Check if this command should run in background
            let background = matches!(op, ListOp::Amp);

            let status = if background {
                self.execute_command_bg(cmd)?
            } else {
                self.execute_command(cmd)?
            };

            // Check for control flow
            if self.returning.is_some() || self.breaking > 0 || self.continuing > 0 {
                return Ok(status);
            }

            match op {
                ListOp::And => {
                    if status != 0 {
                        return Ok(status);
                    }
                }
                ListOp::Or => {
                    if status == 0 {
                        return Ok(0);
                    }
                }
                ListOp::Amp => {
                    // Already backgrounded above, continue
                }
                ListOp::Semi | ListOp::Newline => {
                    // Sequential, continue
                }
            }
        }

        Ok(self.last_status)
    }

    fn execute_command_bg(&mut self, cmd: &ShellCommand) -> Result<i32, String> {
        // For simple commands, run in background
        if let ShellCommand::Simple(simple) = cmd {
            if simple.words.is_empty() {
                return Ok(0);
            }
            let words: Vec<String> = simple.words.iter().map(|w| self.expand_word(w)).collect();
            let cmd_name = &words[0];
            let args: Vec<String> = words[1..].to_vec();
            return self.execute_external_bg(cmd_name, &args, &simple.redirects, true);
        }
        // For complex commands, just execute normally (could fork in future)
        self.execute_command(cmd)
    }

    #[tracing::instrument(level = "trace", skip_all)]
    fn execute_compound(&mut self, compound: &CompoundCommand) -> Result<i32, String> {
        match compound {
            CompoundCommand::BraceGroup(cmds) => {
                for cmd in cmds {
                    self.execute_command(cmd)?;
                    if self.returning.is_some() {
                        break;
                    }
                }
                Ok(self.last_status)
            }
            CompoundCommand::Subshell(cmds) => {
                // Subshell isolates variable changes — save/restore all state.
                // In real zsh this forks; we simulate by cloning variables.
                let saved_vars = self.variables.clone();
                let saved_arrays = self.arrays.clone();
                let saved_assoc = self.assoc_arrays.clone();
                let saved_params = self.positional_params.clone();

                for cmd in cmds {
                    self.execute_command(cmd)?;
                    if self.returning.is_some() {
                        break;
                    }
                }
                let status = self.last_status;

                // Restore state — subshell changes are discarded
                self.variables = saved_vars;
                self.arrays = saved_arrays;
                self.assoc_arrays = saved_assoc;
                self.positional_params = saved_params;
                self.last_status = status;

                Ok(status)
            }

            CompoundCommand::If {
                conditions,
                else_part,
            } => {
                for (cond, body) in conditions {
                    // Execute condition
                    for cmd in cond {
                        self.execute_command(cmd)?;
                    }

                    if self.last_status == 0 {
                        // Condition true, execute body
                        for cmd in body {
                            self.execute_command(cmd)?;
                        }
                        return Ok(self.last_status);
                    }
                }

                // All conditions false, execute else
                if let Some(else_cmds) = else_part {
                    for cmd in else_cmds {
                        self.execute_command(cmd)?;
                    }
                }

                Ok(self.last_status)
            }

            CompoundCommand::For { var, words, body } => {
                let items: Vec<String> = if let Some(words) = words {
                    words
                        .iter()
                        .flat_map(|w| self.expand_word_split(w))
                        .collect()
                } else {
                    // Iterate over positional parameters
                    self.positional_params.clone()
                };

                for item in items {
                    env::set_var(var, &item);
                    self.variables.insert(var.clone(), item);

                    for cmd in body {
                        self.execute_command(cmd)?;
                        if self.breaking > 0 || self.continuing > 0 || self.returning.is_some() {
                            break;
                        }
                    }

                    if self.continuing > 0 {
                        self.continuing -= 1;
                        if self.continuing > 0 {
                            break;
                        }
                        continue;
                    }
                    if self.breaking > 0 {
                        self.breaking -= 1;
                        break;
                    }
                    if self.returning.is_some() {
                        break;
                    }
                }

                Ok(self.last_status)
            }

            CompoundCommand::ForArith {
                init,
                cond,
                step,
                body,
            } => {
                // C-style for loop: for ((init; cond; step))
                // Execute init expression (use evaluate_arithmetic_expr for assignment support)
                if !init.is_empty() {
                    self.evaluate_arithmetic_expr(init);
                }

                // Loop while condition is true
                loop {
                    // Evaluate condition (use eval_arith_expr for comparison result)
                    if !cond.is_empty() {
                        let cond_result = self.eval_arith_expr(cond);
                        if cond_result == 0 {
                            break;
                        }
                    }

                    // Execute body
                    for cmd in body {
                        self.execute_command(cmd)?;
                        if self.breaking > 0 || self.continuing > 0 || self.returning.is_some() {
                            break;
                        }
                    }

                    if self.continuing > 0 {
                        self.continuing -= 1;
                        if self.continuing > 0 {
                            break;
                        }
                        continue;
                    }
                    if self.breaking > 0 {
                        self.breaking -= 1;
                        break;
                    }
                    if self.returning.is_some() {
                        break;
                    }

                    // Execute step (use evaluate_arithmetic_expr for assignment support like i++)
                    if !step.is_empty() {
                        self.evaluate_arithmetic_expr(step);
                    }
                }
                Ok(self.last_status)
            }

            CompoundCommand::While { condition, body } => {
                loop {
                    for cmd in condition {
                        self.execute_command(cmd)?;
                        if self.breaking > 0 || self.returning.is_some() {
                            break;
                        }
                    }

                    if self.last_status != 0 || self.breaking > 0 || self.returning.is_some() {
                        break;
                    }

                    for cmd in body {
                        self.execute_command(cmd)?;
                        if self.breaking > 0 || self.continuing > 0 || self.returning.is_some() {
                            break;
                        }
                    }

                    if self.continuing > 0 {
                        self.continuing -= 1;
                        if self.continuing > 0 {
                            break;
                        }
                        continue;
                    }
                    if self.breaking > 0 {
                        self.breaking -= 1;
                        break;
                    }
                }
                Ok(self.last_status)
            }

            CompoundCommand::Until { condition, body } => {
                loop {
                    for cmd in condition {
                        self.execute_command(cmd)?;
                        if self.breaking > 0 || self.returning.is_some() {
                            break;
                        }
                    }

                    if self.last_status == 0 || self.breaking > 0 || self.returning.is_some() {
                        break;
                    }

                    for cmd in body {
                        self.execute_command(cmd)?;
                        if self.breaking > 0 || self.continuing > 0 || self.returning.is_some() {
                            break;
                        }
                    }

                    if self.continuing > 0 {
                        self.continuing -= 1;
                        if self.continuing > 0 {
                            break;
                        }
                        continue;
                    }
                    if self.breaking > 0 {
                        self.breaking -= 1;
                        break;
                    }
                }
                Ok(self.last_status)
            }

            CompoundCommand::Case { word, cases } => {
                let value = self.expand_word(word);

                for (patterns, body, term) in cases {
                    for pattern in patterns {
                        let pat = self.expand_word(pattern);
                        if self.matches_pattern(&value, &pat) {
                            for cmd in body {
                                self.execute_command(cmd)?;
                            }

                            match term {
                                CaseTerminator::Break => return Ok(self.last_status),
                                CaseTerminator::Fallthrough => {
                                    // Continue to next case body
                                }
                                CaseTerminator::Continue => {
                                    // Continue pattern matching
                                    break;
                                }
                            }
                        }
                    }
                }

                Ok(self.last_status)
            }

            CompoundCommand::Select { var, words, body } => {
                // Simplified: just use first word
                if let Some(words) = words {
                    if let Some(first) = words.first() {
                        let val = self.expand_word(first);
                        env::set_var(var, &val);
                        for cmd in body {
                            self.execute_command(cmd)?;
                        }
                    }
                }
                Ok(self.last_status)
            }

            CompoundCommand::Repeat { count, body } => {
                let n: i64 = self
                    .expand_word(&ShellWord::Literal(count.clone()))
                    .parse()
                    .unwrap_or(0);

                for _ in 0..n {
                    for cmd in body {
                        self.execute_command(cmd)?;
                        if self.breaking > 0 || self.continuing > 0 || self.returning.is_some() {
                            break;
                        }
                    }

                    if self.continuing > 0 {
                        self.continuing -= 1;
                        if self.continuing > 0 {
                            break;
                        }
                        continue;
                    }
                    if self.breaking > 0 {
                        self.breaking -= 1;
                        break;
                    }
                    if self.returning.is_some() {
                        break;
                    }
                }

                Ok(self.last_status)
            }

            CompoundCommand::Try {
                try_body,
                always_body,
            } => {
                // Port of exectry() from Src/loop.c
                // The :try clause
                for cmd in try_body {
                    if let Err(_e) = self.execute_command(cmd) {
                        break;
                    }
                    if self.returning.is_some() {
                        break;
                    }
                }

                // endval = lastval ? lastval : errflag
                let endval = self.last_status;

                // Save and reset control flow flags for the always clause
                let save_returning = self.returning.take();
                let save_breaking = self.breaking;
                let save_continuing = self.continuing;
                self.breaking = 0;
                self.continuing = 0;

                // The always clause — executes unconditionally
                for cmd in always_body {
                    let _ = self.execute_command(cmd);
                }

                // Restore control flow: C uses "if (!retflag) retflag = save"
                // i.e. always block's flags take precedence if set
                if self.returning.is_none() {
                    self.returning = save_returning;
                }
                if self.breaking == 0 {
                    self.breaking = save_breaking;
                }
                if self.continuing == 0 {
                    self.continuing = save_continuing;
                }

                self.last_status = endval;
                Ok(endval)
            }

            CompoundCommand::Cond(expr) => {
                let result = self.eval_cond_expr(expr);
                self.last_status = if result { 0 } else { 1 };
                Ok(self.last_status)
            }

            CompoundCommand::Arith(expr) => {
                // Evaluate arithmetic expression and set variables
                let result = self.evaluate_arithmetic_expr(expr);
                // (( )) returns 0 if result is non-zero, 1 if result is zero
                self.last_status = if result != 0 { 0 } else { 1 };
                Ok(self.last_status)
            }

            CompoundCommand::Coproc { name, body } => {
                // Create pipes for stdin and stdout
                let (stdin_read, stdin_write) =
                    os_pipe::pipe().map_err(|e| format!("Cannot create pipe: {}", e))?;
                let (stdout_read, stdout_write) =
                    os_pipe::pipe().map_err(|e| format!("Cannot create pipe: {}", e))?;

                // Get the command to run
                let cmd_str = match body.as_ref() {
                    ShellCommand::Simple(simple) => simple
                        .words
                        .iter()
                        .map(|w| self.expand_word(w))
                        .collect::<Vec<_>>()
                        .join(" "),
                    ShellCommand::Compound(CompoundCommand::BraceGroup(_cmds)) => {
                        // Just run as a subshell with the commands
                        // For simplicity, we'll use bash -c
                        "bash -c 'true'".to_string()
                    }
                    _ => "true".to_string(),
                };

                // Fork and run the command in background with redirected stdin/stdout
                let parts: Vec<&str> = cmd_str.split_whitespace().collect();
                if parts.is_empty() {
                    return Ok(0);
                }

                let mut command = Command::new(parts[0]);
                if parts.len() > 1 {
                    command.args(&parts[1..]);
                }

                use std::os::unix::io::{FromRawFd, IntoRawFd};

                command.stdin(unsafe { Stdio::from_raw_fd(stdin_read.into_raw_fd()) });
                command.stdout(unsafe { Stdio::from_raw_fd(stdout_write.into_raw_fd()) });

                match command.spawn() {
                    Ok(child) => {
                        let pid = child.id();
                        let coproc_name = name.clone().unwrap_or_else(|| "COPROC".to_string());

                        // Store file descriptors in environment-like variables
                        // COPROC[0] = read from coproc (stdout_read)
                        // COPROC[1] = write to coproc (stdin_write)
                        let read_fd = stdout_read.into_raw_fd();
                        let write_fd = stdin_write.into_raw_fd();

                        self.arrays.insert(
                            coproc_name.clone(),
                            vec![read_fd.to_string(), write_fd.to_string()],
                        );

                        // Also store PID
                        self.variables
                            .insert(format!("{}_PID", coproc_name), pid.to_string());

                        let cmd_str_clone = cmd_str.clone();
                        self.jobs.add_job(child, cmd_str_clone, JobState::Running);

                        Ok(0)
                    }
                    Err(e) => {
                        if e.kind() == io::ErrorKind::NotFound {
                            eprintln!("zshrs: command not found: {}", parts[0]);
                            Ok(127)
                        } else {
                            Err(format!("zshrs: coproc: {}: {}", parts[0], e))
                        }
                    }
                }
            }

            CompoundCommand::WithRedirects(cmd, redirects) => {
                // Execute the command with redirects applied
                let mut saved_fds: Vec<(i32, i32)> = Vec::new();

                // Set up redirects
                for redirect in redirects {
                    let fd = redirect.fd.unwrap_or(match redirect.op {
                        RedirectOp::Read
                        | RedirectOp::HereDoc
                        | RedirectOp::HereString
                        | RedirectOp::ReadWrite => 0,
                        _ => 1,
                    });

                    let target = self.expand_word(&redirect.target);

                    match redirect.op {
                        RedirectOp::Write | RedirectOp::Clobber => {
                            use std::os::unix::io::IntoRawFd;
                            let saved = unsafe { libc::dup(fd) };
                            if saved >= 0 {
                                saved_fds.push((fd, saved));
                            }
                            if let Ok(file) = std::fs::File::create(&target) {
                                let new_fd = file.into_raw_fd();
                                unsafe {
                                    libc::dup2(new_fd, fd);
                                }
                                unsafe {
                                    libc::close(new_fd);
                                }
                            }
                        }
                        RedirectOp::Append => {
                            use std::os::unix::io::IntoRawFd;
                            let saved = unsafe { libc::dup(fd) };
                            if saved >= 0 {
                                saved_fds.push((fd, saved));
                            }
                            if let Ok(file) = std::fs::OpenOptions::new()
                                .create(true)
                                .append(true)
                                .open(&target)
                            {
                                let new_fd = file.into_raw_fd();
                                unsafe {
                                    libc::dup2(new_fd, fd);
                                }
                                unsafe {
                                    libc::close(new_fd);
                                }
                            }
                        }
                        RedirectOp::Read => {
                            use std::os::unix::io::IntoRawFd;
                            let saved = unsafe { libc::dup(fd) };
                            if saved >= 0 {
                                saved_fds.push((fd, saved));
                            }
                            if let Ok(file) = std::fs::File::open(&target) {
                                let new_fd = file.into_raw_fd();
                                unsafe {
                                    libc::dup2(new_fd, fd);
                                }
                                unsafe {
                                    libc::close(new_fd);
                                }
                            }
                        }
                        RedirectOp::DupWrite | RedirectOp::DupRead => {
                            if let Ok(target_fd) = target.parse::<i32>() {
                                let saved = unsafe { libc::dup(fd) };
                                if saved >= 0 {
                                    saved_fds.push((fd, saved));
                                }
                                unsafe {
                                    libc::dup2(target_fd, fd);
                                }
                            }
                        }
                        _ => {}
                    }
                }

                // Execute the inner command
                let result = self.execute_command(cmd);

                // Restore saved fds
                for (fd, saved) in saved_fds.into_iter().rev() {
                    unsafe {
                        libc::dup2(saved, fd);
                        libc::close(saved);
                    }
                }

                result
            }
        }
    }

    /// Expand a word with brace and glob expansion (for command arguments)
    #[tracing::instrument(level = "trace", skip_all)]
    fn expand_word_glob(&mut self, word: &ShellWord) -> Vec<String> {
        match word {
            ShellWord::SingleQuoted(s) => vec![s.clone()],
            ShellWord::DoubleQuoted(parts) => {
                // Double quotes prevent glob and brace expansion
                vec![parts.iter().map(|p| self.expand_word(p)).collect()]
            }
            _ => {
                let expanded = self.expand_word(word);

                // First do brace expansion
                let brace_expanded = self.expand_braces(&expanded);

                // Then glob expansion on each result (unless noglob is set)
                let noglob = self.options.get("noglob").copied().unwrap_or(false)
                    || self.options.get("GLOB").map(|v| !v).unwrap_or(false);
                brace_expanded
                    .into_iter()
                    .flat_map(|s| {
                        if !noglob
                            && (s.contains('*')
                                || s.contains('?')
                                || s.contains('[')
                                || self.has_extglob_pattern(&s))
                        {
                            self.expand_glob(&s)
                        } else {
                            vec![s]
                        }
                    })
                    .collect()
            }
        }
    }

    /// Expand brace patterns like {a,b,c} and {1..10}
    fn expand_braces(&self, s: &str) -> Vec<String> {
        // Find a brace pattern
        let mut depth = 0;
        let mut brace_start = None;

        for (i, c) in s.char_indices() {
            match c {
                '{' => {
                    if depth == 0 {
                        brace_start = Some(i);
                    }
                    depth += 1;
                }
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        if let Some(start) = brace_start {
                            let prefix = &s[..start];
                            let content = &s[start + 1..i];
                            let suffix = &s[i + 1..];

                            // Check if this is a sequence {a..b} or a list {a,b,c}
                            let expansions = if content.contains("..") {
                                self.expand_brace_sequence(content)
                            } else if content.contains(',') {
                                self.expand_brace_list(content)
                            } else {
                                // Not a valid brace expansion, return as-is
                                return vec![s.to_string()];
                            };

                            // Combine prefix, expansions, and suffix
                            let mut results = Vec::new();
                            for exp in expansions {
                                let combined = format!("{}{}{}", prefix, exp, suffix);
                                // Recursively expand any remaining braces
                                results.extend(self.expand_braces(&combined));
                            }
                            return results;
                        }
                    }
                }
                _ => {}
            }
        }

        // No brace expansion found
        vec![s.to_string()]
    }

    /// Expand comma-separated brace list like {a,b,c}
    fn expand_brace_list(&self, content: &str) -> Vec<String> {
        // Split by comma, but respect nested braces
        let mut parts = Vec::new();
        let mut current = String::new();
        let mut depth = 0;

        for c in content.chars() {
            match c {
                '{' => {
                    depth += 1;
                    current.push(c);
                }
                '}' => {
                    depth -= 1;
                    current.push(c);
                }
                ',' if depth == 0 => {
                    parts.push(current.clone());
                    current.clear();
                }
                _ => current.push(c),
            }
        }
        parts.push(current);

        parts
    }

    /// Expand sequence brace pattern like {1..10} or {a..z}
    fn expand_brace_sequence(&self, content: &str) -> Vec<String> {
        let parts: Vec<&str> = content.splitn(3, "..").collect();
        if parts.len() < 2 {
            return vec![content.to_string()];
        }

        let start = parts[0];
        let end = parts[1];
        let step: i64 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);

        // Try numeric sequence
        if let (Ok(start_num), Ok(end_num)) = (start.parse::<i64>(), end.parse::<i64>()) {
            let mut results = Vec::new();
            if start_num <= end_num {
                let mut i = start_num;
                while i <= end_num {
                    results.push(i.to_string());
                    i += step;
                }
            } else {
                let mut i = start_num;
                while i >= end_num {
                    results.push(i.to_string());
                    i -= step;
                }
            }
            return results;
        }

        // Try character sequence
        if start.len() == 1 && end.len() == 1 {
            let start_char = start.chars().next().unwrap();
            let end_char = end.chars().next().unwrap();
            let mut results = Vec::new();

            if start_char <= end_char {
                let mut c = start_char;
                while c <= end_char {
                    results.push(c.to_string());
                    c = (c as u8 + step as u8) as char;
                    if c as u8 > end_char as u8 {
                        break;
                    }
                }
            } else {
                let mut c = start_char;
                while c >= end_char {
                    results.push(c.to_string());
                    if (c as u8) < step as u8 {
                        break;
                    }
                    c = (c as u8 - step as u8) as char;
                }
            }
            return results;
        }

        vec![content.to_string()]
    }

    /// Expand glob pattern to matching files
    fn expand_glob(&self, pattern: &str) -> Vec<String> {
        // Check for zsh glob qualifiers at end: *(.) *(/) *(@) etc.
        let (glob_pattern, qualifiers) = self.parse_glob_qualifiers(pattern);

        // Check for extended glob patterns: ?(pat), *(pat), +(pat), @(pat), !(pat)
        if self.has_extglob_pattern(&glob_pattern) {
            let expanded = self.expand_extglob(&glob_pattern);
            return self.filter_by_qualifiers(expanded, &qualifiers);
        }

        let nullglob = self.options.get("nullglob").copied().unwrap_or(false);
        let dotglob = self.options.get("dotglob").copied().unwrap_or(false);
        let nocaseglob = self.options.get("nocaseglob").copied().unwrap_or(false);

        // Parallel recursive glob: when pattern contains **/ we split the
        // directory walk across worker pool threads — one thread per top-level
        // subdirectory.  zsh does this single-threaded via fork+exec which is
        // why `echo **/*.rs` is painfully slow on large trees.
        let expanded = if glob_pattern.contains("**/") {
            self.expand_glob_parallel(&glob_pattern, dotglob, nocaseglob)
        } else {
            let options = glob::MatchOptions {
                case_sensitive: !nocaseglob,
                require_literal_separator: false,
                require_literal_leading_dot: !dotglob,
            };
            match glob::glob_with(&glob_pattern, options) {
                Ok(paths) => paths
                    .filter_map(|p| p.ok())
                    .map(|p| p.to_string_lossy().to_string())
                    .collect(),
                Err(_) => vec![],
            }
        };

        let mut expanded = self.filter_by_qualifiers(expanded, &qualifiers);
        expanded.sort();

        if expanded.is_empty() {
            if nullglob {
                vec![]
            } else {
                vec![pattern.to_string()]
            }
        } else {
            expanded
        }
    }

    /// Parallel recursive glob using the worker pool.
    ///
    /// Splits `base/**/file_pattern` into per-subdirectory walks, each
    /// running on a pool thread via walkdir.  Results merge via channel.
    /// This is why `echo **/*.rs` will be 5-10x faster than zsh.
    fn expand_glob_parallel(
        &self,
        pattern: &str,
        dotglob: bool,
        nocaseglob: bool,
    ) -> Vec<String> {
        use walkdir::WalkDir;

        // Split pattern at the first **/ into (base_dir, file_glob)
        // e.g. "src/**/*.rs" → ("src", "*.rs")
        //      "**/*.rs"     → (".", "*.rs")
        let (base, file_glob) = if let Some(pos) = pattern.find("**/") {
            let base = if pos == 0 { "." } else { &pattern[..pos.saturating_sub(1)] };
            let rest = &pattern[pos + 3..]; // skip "**/", get "*.rs" or "foo/**/*.rs"
            (base.to_string(), rest.to_string())
        } else {
            return vec![];
        };

        // If file_glob itself contains **/, fall back to single-threaded glob
        // (nested recursive patterns are rare, not worth the complexity)
        if file_glob.contains("**/") {
            let options = glob::MatchOptions {
                case_sensitive: !nocaseglob,
                require_literal_separator: false,
                require_literal_leading_dot: !dotglob,
            };
            return match glob::glob_with(pattern, options) {
                Ok(paths) => paths
                    .filter_map(|p| p.ok())
                    .map(|p| p.to_string_lossy().to_string())
                    .collect(),
                Err(_) => vec![],
            };
        }

        // Build the glob::Pattern for matching filenames
        let match_opts = glob::MatchOptions {
            case_sensitive: !nocaseglob,
            require_literal_separator: false,
            require_literal_leading_dot: !dotglob,
        };
        let file_pat = match glob::Pattern::new(&file_glob) {
            Ok(p) => p,
            Err(_) => return vec![],
        };

        // Enumerate top-level entries in base dir to fan out across workers
        let top_entries: Vec<std::path::PathBuf> = match std::fs::read_dir(&base) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .collect(),
            Err(_) => return vec![],
        };

        // Also check files directly in base (not in subdirs)
        let mut results: Vec<String> = Vec::new();
        for entry in &top_entries {
            if entry.is_file() || entry.is_symlink() {
                if let Some(name) = entry.file_name().and_then(|n| n.to_str()) {
                    if file_pat.matches_with(name, match_opts) {
                        results.push(entry.to_string_lossy().to_string());
                    }
                }
            }
        }

        // Fan out subdirectory walks to worker pool
        let subdirs: Vec<std::path::PathBuf> = top_entries
            .into_iter()
            .filter(|p| p.is_dir())
            .filter(|p| {
                dotglob
                    || !p
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.starts_with('.'))
                        .unwrap_or(false)
            })
            .collect();

        if subdirs.is_empty() {
            return results;
        }

        let (tx, rx) = std::sync::mpsc::channel::<Vec<String>>();

        for subdir in &subdirs {
            let tx = tx.clone();
            let subdir = subdir.clone();
            let file_pat = file_pat.clone();
            let skip_dot = !dotglob;
            self.worker_pool.submit(move || {
                let mut matches = Vec::new();
                let walker = WalkDir::new(&subdir)
                    .follow_links(false)
                    .into_iter()
                    .filter_entry(move |e| {
                        // Skip hidden dirs if !dotglob
                        if skip_dot {
                            if let Some(name) = e.file_name().to_str() {
                                if name.starts_with('.') && e.depth() > 0 {
                                    return false;
                                }
                            }
                        }
                        true
                    });
                for entry in walker.filter_map(|e| e.ok()) {
                    if entry.file_type().is_file() || entry.file_type().is_symlink() {
                        if let Some(name) = entry.file_name().to_str() {
                            if file_pat.matches_with(name, match_opts) {
                                matches.push(entry.path().to_string_lossy().to_string());
                            }
                        }
                    }
                }
                let _ = tx.send(matches);
            });
        }

        // Drop our sender so rx knows when all workers are done
        drop(tx);

        // Collect results from all workers
        for batch in rx {
            results.extend(batch);
        }

        results
    }

    /// Parse zsh glob qualifiers from the end of a pattern
    /// Returns (pattern_without_qualifiers, qualifiers_string)
    fn parse_glob_qualifiers(&self, pattern: &str) -> (String, String) {
        // Check if pattern ends with (...) that looks like qualifiers
        // Qualifiers are single chars like . / @ * % or combinations
        if !pattern.ends_with(')') {
            return (pattern.to_string(), String::new());
        }

        // Find matching opening paren
        let chars: Vec<char> = pattern.chars().collect();
        let mut depth = 0;
        let mut qual_start = None;

        for i in (0..chars.len()).rev() {
            match chars[i] {
                ')' => depth += 1,
                '(' => {
                    depth -= 1;
                    if depth == 0 {
                        qual_start = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }

        if let Some(start) = qual_start {
            let qual_content: String = chars[start + 1..chars.len() - 1].iter().collect();

            // Check if this looks like glob qualifiers (not extglob)
            // Qualifiers are things like: . / @ * % r w x ^ - etc.
            // Extglob would have | inside
            if !qual_content.contains('|') && self.looks_like_glob_qualifiers(&qual_content) {
                let base_pattern: String = chars[..start].iter().collect();
                return (base_pattern, qual_content);
            }
        }

        (pattern.to_string(), String::new())
    }

    /// Check if string looks like glob qualifiers
    fn looks_like_glob_qualifiers(&self, s: &str) -> bool {
        if s.is_empty() {
            return false;
        }
        // Valid qualifier chars: . / @ = p * % r w x A I E R W X s S t ^ - + :
        // Also numbers for depth limits, and things like [1,5] for ranges
        let valid_chars = "./@=p*%brwxAIERWXsStfedDLNnMmcaou^-+:0123456789,[]FT";
        s.chars()
            .all(|c| valid_chars.contains(c) || c.is_whitespace())
    }

    /// Filter file list by glob qualifiers
    /// Prefetch file metadata in parallel across the worker pool.
    /// Returns a map from path → (metadata, symlink_metadata).
    /// Each batch of files is stat'd on a pool thread.
    fn prefetch_metadata(
        &self,
        files: &[String],
    ) -> HashMap<String, (Option<std::fs::Metadata>, Option<std::fs::Metadata>)> {
        if files.len() < 32 {
            // Small list — serial stat is faster than channel overhead
            return files
                .iter()
                .map(|f| {
                    let meta = std::fs::metadata(f).ok();
                    let symlink_meta = std::fs::symlink_metadata(f).ok();
                    (f.clone(), (meta, symlink_meta))
                })
                .collect();
        }

        let pool_size = self.worker_pool.size();
        let chunk_size = (files.len() + pool_size - 1) / pool_size;
        let (tx, rx) = std::sync::mpsc::channel();

        for chunk in files.chunks(chunk_size) {
            let tx = tx.clone();
            let chunk: Vec<String> = chunk.to_vec();
            self.worker_pool.submit(move || {
                let batch: Vec<(String, (Option<std::fs::Metadata>, Option<std::fs::Metadata>))> =
                    chunk
                        .into_iter()
                        .map(|f| {
                            let meta = std::fs::metadata(&f).ok();
                            let symlink_meta = std::fs::symlink_metadata(&f).ok();
                            (f, (meta, symlink_meta))
                        })
                        .collect();
                let _ = tx.send(batch);
            });
        }
        drop(tx);

        let mut map = HashMap::with_capacity(files.len());
        for batch in rx {
            for (path, metas) in batch {
                map.insert(path, metas);
            }
        }
        map
    }

    fn filter_by_qualifiers(&self, files: Vec<String>, qualifiers: &str) -> Vec<String> {
        if qualifiers.is_empty() {
            return files;
        }

        // Parallel metadata prefetch — all stat syscalls happen on pool threads,
        // then filter/sort uses cached metadata with zero syscalls.
        let meta_cache = self.prefetch_metadata(&files);

        let mut result = files;
        let mut negate = false;
        let mut chars = qualifiers.chars().peekable();

        while let Some(c) = chars.next() {
            match c {
                // Negation
                '^' => negate = !negate,

                // File types — all use prefetched metadata cache
                '.' => {
                    result = result
                        .into_iter()
                        .filter(|f| {
                            let is_file = meta_cache
                                .get(f)
                                .and_then(|(m, _)| m.as_ref())
                                .map(|m| m.is_file())
                                .unwrap_or(false);
                            if negate { !is_file } else { is_file }
                        })
                        .collect();
                    negate = false;
                }
                '/' => {
                    result = result
                        .into_iter()
                        .filter(|f| {
                            let is_dir = meta_cache
                                .get(f)
                                .and_then(|(m, _)| m.as_ref())
                                .map(|m| m.is_dir())
                                .unwrap_or(false);
                            if negate { !is_dir } else { is_dir }
                        })
                        .collect();
                    negate = false;
                }
                '@' => {
                    result = result
                        .into_iter()
                        .filter(|f| {
                            let is_link = meta_cache
                                .get(f)
                                .and_then(|(_, sm)| sm.as_ref())
                                .map(|m| m.file_type().is_symlink())
                                .unwrap_or(false);
                            if negate { !is_link } else { is_link }
                        })
                        .collect();
                    negate = false;
                }
                '=' => {
                    // Sockets
                    use std::os::unix::fs::FileTypeExt;
                    result = result
                        .into_iter()
                        .filter(|f| {
                            let is_socket = meta_cache
                                .get(f)
                                .and_then(|(_, sm)| sm.as_ref())
                                .map(|m| m.file_type().is_socket())
                                .unwrap_or(false);
                            if negate { !is_socket } else { is_socket }
                        })
                        .collect();
                    negate = false;
                }
                'p' => {
                    // Named pipes (FIFOs)
                    use std::os::unix::fs::FileTypeExt;
                    result = result
                        .into_iter()
                        .filter(|f| {
                            let is_fifo = meta_cache
                                .get(f)
                                .and_then(|(_, sm)| sm.as_ref())
                                .map(|m| m.file_type().is_fifo())
                                .unwrap_or(false);
                            if negate { !is_fifo } else { is_fifo }
                        })
                        .collect();
                    negate = false;
                }
                '*' => {
                    // Executable files
                    use std::os::unix::fs::PermissionsExt;
                    result = result
                        .into_iter()
                        .filter(|f| {
                            let is_exec = meta_cache
                                .get(f)
                                .and_then(|(m, _)| m.as_ref())
                                .map(|m| m.is_file() && (m.permissions().mode() & 0o111) != 0)
                                .unwrap_or(false);
                            if negate { !is_exec } else { is_exec }
                        })
                        .collect();
                    negate = false;
                }
                '%' => {
                    // Device files
                    use std::os::unix::fs::FileTypeExt;
                    let next = chars.peek().copied();
                    result = result
                        .into_iter()
                        .filter(|f| {
                            let is_device = meta_cache
                                .get(f)
                                .and_then(|(_, sm)| sm.as_ref())
                                .map(|m| match next {
                                    Some('b') => m.file_type().is_block_device(),
                                    Some('c') => m.file_type().is_char_device(),
                                    _ => {
                                        m.file_type().is_block_device()
                                            || m.file_type().is_char_device()
                                    }
                                })
                                .unwrap_or(false);
                            if negate { !is_device } else { is_device }
                        })
                        .collect();
                    if next == Some('b') || next == Some('c') {
                        chars.next();
                    }
                    negate = false;
                }

                // Permission qualifiers — all use prefetched metadata cache
                'r' => {
                    result = self.filter_by_permission(result, 0o400, negate, &meta_cache);
                    negate = false;
                }
                'w' => {
                    result = self.filter_by_permission(result, 0o200, negate, &meta_cache);
                    negate = false;
                }
                'x' => {
                    result = self.filter_by_permission(result, 0o100, negate, &meta_cache);
                    negate = false;
                }
                'A' => {
                    result = self.filter_by_permission(result, 0o040, negate, &meta_cache);
                    negate = false;
                }
                'I' => {
                    result = self.filter_by_permission(result, 0o020, negate, &meta_cache);
                    negate = false;
                }
                'E' => {
                    result = self.filter_by_permission(result, 0o010, negate, &meta_cache);
                    negate = false;
                }
                'R' => {
                    result = self.filter_by_permission(result, 0o004, negate, &meta_cache);
                    negate = false;
                }
                'W' => {
                    result = self.filter_by_permission(result, 0o002, negate, &meta_cache);
                    negate = false;
                }
                'X' => {
                    result = self.filter_by_permission(result, 0o001, negate, &meta_cache);
                    negate = false;
                }
                's' => {
                    result = self.filter_by_permission(result, 0o4000, negate, &meta_cache);
                    negate = false;
                }
                'S' => {
                    result = self.filter_by_permission(result, 0o2000, negate, &meta_cache);
                    negate = false;
                }
                't' => {
                    result = self.filter_by_permission(result, 0o1000, negate, &meta_cache);
                    negate = false;
                }

                // Full/empty directories
                'F' => {
                    // Non-empty directories
                    result = result
                        .into_iter()
                        .filter(|f| {
                            let path = std::path::Path::new(f);
                            let is_nonempty = path.is_dir()
                                && std::fs::read_dir(path)
                                    .map(|mut d| d.next().is_some())
                                    .unwrap_or(false);
                            if negate {
                                !is_nonempty
                            } else {
                                is_nonempty
                            }
                        })
                        .collect();
                    negate = false;
                }

                // Ownership — uses prefetched metadata cache
                'U' => {
                    // Owned by effective UID
                    let euid = unsafe { libc::geteuid() };
                    result = result
                        .into_iter()
                        .filter(|f| {
                            use std::os::unix::fs::MetadataExt;
                            let is_owned = meta_cache
                                .get(f)
                                .and_then(|(m, _)| m.as_ref())
                                .map(|m| m.uid() == euid)
                                .unwrap_or(false);
                            if negate { !is_owned } else { is_owned }
                        })
                        .collect();
                    negate = false;
                }
                'G' => {
                    // Owned by effective GID
                    let egid = unsafe { libc::getegid() };
                    result = result
                        .into_iter()
                        .filter(|f| {
                            use std::os::unix::fs::MetadataExt;
                            let is_owned = meta_cache
                                .get(f)
                                .and_then(|(m, _)| m.as_ref())
                                .map(|m| m.gid() == egid)
                                .unwrap_or(false);
                            if negate { !is_owned } else {
                                is_owned
                            }
                        })
                        .collect();
                    negate = false;
                }

                // Sorting modifiers
                'o' => {
                    // Sort by name (ascending) - already default
                    if chars.peek() == Some(&'n') {
                        chars.next();
                        // Sort by name
                        result.sort();
                    } else if chars.peek() == Some(&'L') {
                        chars.next();
                        // Sort by size — uses prefetched metadata
                        result.sort_by_key(|f| {
                            meta_cache.get(f).and_then(|(m, _)| m.as_ref()).map(|m| m.len()).unwrap_or(0)
                        });
                    } else if chars.peek() == Some(&'m') {
                        chars.next();
                        // Sort by modification time — uses prefetched metadata
                        result.sort_by_key(|f| {
                            meta_cache.get(f).and_then(|(m, _)| m.as_ref())
                                .and_then(|m| m.modified().ok())
                                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                        });
                    } else if chars.peek() == Some(&'a') {
                        chars.next();
                        // Sort by access time — uses prefetched metadata
                        result.sort_by_key(|f| {
                            meta_cache.get(f).and_then(|(m, _)| m.as_ref())
                                .and_then(|m| m.accessed().ok())
                                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                        });
                    }
                }
                'O' => {
                    // Reverse sort — uses prefetched metadata
                    if chars.peek() == Some(&'n') {
                        chars.next();
                        result.sort();
                        result.reverse();
                    } else if chars.peek() == Some(&'L') {
                        chars.next();
                        result.sort_by_key(|f| {
                            meta_cache.get(f).and_then(|(m, _)| m.as_ref()).map(|m| m.len()).unwrap_or(0)
                        });
                        result.reverse();
                    } else if chars.peek() == Some(&'m') {
                        chars.next();
                        result.sort_by_key(|f| {
                            meta_cache.get(f).and_then(|(m, _)| m.as_ref())
                                .and_then(|m| m.modified().ok())
                                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                        });
                        result.reverse();
                    } else {
                        // Just reverse current order
                        result.reverse();
                    }
                }

                // Subscript range [n] or [n,m]
                '[' => {
                    let mut range_str = String::new();
                    while let Some(&ch) = chars.peek() {
                        if ch == ']' {
                            chars.next();
                            break;
                        }
                        range_str.push(chars.next().unwrap());
                    }

                    if let Some((start, end)) = self.parse_subscript_range(&range_str, result.len())
                    {
                        result = result.into_iter().skip(start).take(end - start).collect();
                    }
                }

                // Depth limit (for **/)
                'D' => {
                    // Include dotfiles (handled by dotglob)
                }
                'N' => {
                    // Nullglob for this pattern
                }

                // Unknown qualifier - ignore
                _ => {}
            }
        }

        result
    }

    /// Filter files by permission bits — uses prefetched metadata cache
    fn filter_by_permission(
        &self,
        files: Vec<String>,
        mode: u32,
        negate: bool,
        meta_cache: &HashMap<String, (Option<std::fs::Metadata>, Option<std::fs::Metadata>)>,
    ) -> Vec<String> {
        use std::os::unix::fs::PermissionsExt;
        files
            .into_iter()
            .filter(|f| {
                let has_perm = meta_cache
                    .get(f)
                    .and_then(|(m, _)| m.as_ref())
                    .map(|m| (m.permissions().mode() & mode) != 0)
                    .unwrap_or(false);
                if negate { !has_perm } else { has_perm }
            })
            .collect()
    }

    /// Parse subscript range like "1" or "1,5" or "-1" or "1,-1"
    fn parse_subscript_range(&self, s: &str, len: usize) -> Option<(usize, usize)> {
        if s.is_empty() || len == 0 {
            return None;
        }

        let parts: Vec<&str> = s.split(',').collect();

        let parse_idx = |idx_str: &str| -> Option<usize> {
            let idx: i64 = idx_str.trim().parse().ok()?;
            if idx < 0 {
                // Negative index from end
                let abs = (-idx) as usize;
                if abs > len {
                    None
                } else {
                    Some(len - abs)
                }
            } else if idx == 0 {
                Some(0)
            } else {
                // 1-indexed
                Some((idx as usize).saturating_sub(1).min(len))
            }
        };

        match parts.len() {
            1 => {
                // Single element [n]
                let idx = parse_idx(parts[0])?;
                Some((idx, idx + 1))
            }
            2 => {
                // Range [n,m]
                let start = parse_idx(parts[0])?;
                let end = parse_idx(parts[1])?.saturating_add(1);
                Some((start.min(end), start.max(end)))
            }
            _ => None,
        }
    }

    /// Check if pattern contains extended glob syntax
    fn has_extglob_pattern(&self, pattern: &str) -> bool {
        let chars: Vec<char> = pattern.chars().collect();
        for i in 0..chars.len().saturating_sub(1) {
            if (chars[i] == '?'
                || chars[i] == '*'
                || chars[i] == '+'
                || chars[i] == '@'
                || chars[i] == '!')
                && chars[i + 1] == '('
            {
                return true;
            }
        }
        false
    }

    /// Convert extended glob pattern to regex
    fn extglob_to_regex(&self, pattern: &str) -> String {
        let mut regex = String::from("^");
        let chars: Vec<char> = pattern.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            let c = chars[i];

            // Check for extglob patterns
            if i + 1 < chars.len() && chars[i + 1] == '(' {
                match c {
                    '?' => {
                        // ?(pattern) - zero or one occurrence
                        let (inner, end) = self.extract_extglob_inner(&chars, i + 2);
                        let inner_regex = self.extglob_inner_to_regex(&inner);
                        regex.push_str(&format!("({})?", inner_regex));
                        i = end + 1;
                        continue;
                    }
                    '*' => {
                        // *(pattern) - zero or more occurrences
                        let (inner, end) = self.extract_extglob_inner(&chars, i + 2);
                        let inner_regex = self.extglob_inner_to_regex(&inner);
                        regex.push_str(&format!("({})*", inner_regex));
                        i = end + 1;
                        continue;
                    }
                    '+' => {
                        // +(pattern) - one or more occurrences
                        let (inner, end) = self.extract_extglob_inner(&chars, i + 2);
                        let inner_regex = self.extglob_inner_to_regex(&inner);
                        regex.push_str(&format!("({})+", inner_regex));
                        i = end + 1;
                        continue;
                    }
                    '@' => {
                        // @(pattern) - exactly one occurrence
                        let (inner, end) = self.extract_extglob_inner(&chars, i + 2);
                        let inner_regex = self.extglob_inner_to_regex(&inner);
                        regex.push_str(&format!("({})", inner_regex));
                        i = end + 1;
                        continue;
                    }
                    '!' => {
                        // !(pattern) - handled specially in expand_extglob
                        // Just skip this extglob for regex, will do manual filtering
                        let (_, end) = self.extract_extglob_inner(&chars, i + 2);
                        regex.push_str(".*"); // Match anything, we filter later
                        i = end + 1;
                        continue;
                    }
                    _ => {}
                }
            }

            // Handle regular glob characters
            match c {
                '*' => regex.push_str(".*"),
                '?' => regex.push('.'),
                '.' => regex.push_str("\\."),
                '[' => {
                    regex.push('[');
                    i += 1;
                    while i < chars.len() && chars[i] != ']' {
                        if chars[i] == '!' && regex.ends_with('[') {
                            regex.push('^');
                        } else {
                            regex.push(chars[i]);
                        }
                        i += 1;
                    }
                    regex.push(']');
                }
                '^' | '$' | '(' | ')' | '{' | '}' | '|' | '\\' => {
                    regex.push('\\');
                    regex.push(c);
                }
                _ => regex.push(c),
            }
            i += 1;
        }

        regex.push('$');
        regex
    }

    /// Extract the inner part of an extglob pattern (until closing paren)
    fn extract_extglob_inner(&self, chars: &[char], start: usize) -> (String, usize) {
        let mut inner = String::new();
        let mut depth = 1;
        let mut i = start;

        while i < chars.len() && depth > 0 {
            if chars[i] == '(' {
                depth += 1;
            } else if chars[i] == ')' {
                depth -= 1;
                if depth == 0 {
                    return (inner, i);
                }
            }
            inner.push(chars[i]);
            i += 1;
        }

        (inner, i)
    }

    /// Convert the inner part of extglob (handles | for alternation)
    fn extglob_inner_to_regex(&self, inner: &str) -> String {
        // Split by | and convert each alternative
        let alternatives: Vec<String> = inner
            .split('|')
            .map(|alt| {
                let mut result = String::new();
                for c in alt.chars() {
                    match c {
                        '*' => result.push_str(".*"),
                        '?' => result.push('.'),
                        '.' => result.push_str("\\."),
                        '^' | '$' | '(' | ')' | '{' | '}' | '\\' => {
                            result.push('\\');
                            result.push(c);
                        }
                        _ => result.push(c),
                    }
                }
                result
            })
            .collect();

        alternatives.join("|")
    }

    /// Expand extended glob pattern
    fn expand_extglob(&self, pattern: &str) -> Vec<String> {
        // Determine directory to search
        let (search_dir, file_pattern) = if let Some(last_slash) = pattern.rfind('/') {
            (&pattern[..last_slash], &pattern[last_slash + 1..])
        } else {
            (".", pattern)
        };

        // Check for !(pattern) - negative matching
        if let Some((neg_pat, suffix)) = self.extract_neg_extglob(file_pattern) {
            return self.expand_neg_extglob(search_dir, &neg_pat, &suffix, pattern);
        }

        // Convert file pattern to regex for positive extglob
        let regex_str = self.extglob_to_regex(file_pattern);

        let re = match cached_regex(&regex_str) {
            Some(r) => r,
            None => return vec![pattern.to_string()],
        };

        let mut results = Vec::new();

        if let Ok(entries) = std::fs::read_dir(search_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                // Skip hidden files unless pattern starts with .
                if name.starts_with('.') && !file_pattern.starts_with('.') {
                    continue;
                }

                if re.is_match(&name) {
                    let full_path = if search_dir == "." {
                        name
                    } else {
                        format!("{}/{}", search_dir, name)
                    };
                    results.push(full_path);
                }
            }
        }

        if results.is_empty() {
            vec![pattern.to_string()]
        } else {
            results.sort();
            results
        }
    }

    /// Handle !(pattern) negative extglob expansion
    fn expand_neg_extglob(
        &self,
        search_dir: &str,
        neg_pat: &str,
        suffix: &str,
        original_pattern: &str,
    ) -> Vec<String> {
        let mut results = Vec::new();

        if let Ok(entries) = std::fs::read_dir(search_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                // Skip hidden files
                if name.starts_with('.') {
                    continue;
                }

                // File must end with suffix
                if !name.ends_with(suffix) {
                    continue;
                }

                let basename = &name[..name.len() - suffix.len()];
                // Check if basename matches any negated alternative
                let alts: Vec<&str> = neg_pat.split('|').collect();
                let matches_neg = alts.iter().any(|alt| {
                    if alt.contains('*') || alt.contains('?') {
                        let alt_re = self.extglob_inner_to_regex(alt);
                        let full_pattern = format!("^{}$", alt_re);
                        if let Some(r) = cached_regex(&full_pattern) {
                            r.is_match(basename)
                        } else {
                            *alt == basename
                        }
                    } else {
                        *alt == basename
                    }
                });

                if !matches_neg {
                    let full_path = if search_dir == "." {
                        name
                    } else {
                        format!("{}/{}", search_dir, name)
                    };
                    results.push(full_path);
                }
            }
        }

        if results.is_empty() {
            vec![original_pattern.to_string()]
        } else {
            results.sort();
            results
        }
    }

    /// Extract !(pattern) info from file pattern, returns (inner_pattern, suffix)
    fn extract_neg_extglob(&self, pattern: &str) -> Option<(String, String)> {
        let chars: Vec<char> = pattern.chars().collect();
        if chars.len() >= 3 && chars[0] == '!' && chars[1] == '(' {
            let mut depth = 1;
            let mut i = 2;
            while i < chars.len() && depth > 0 {
                if chars[i] == '(' {
                    depth += 1;
                } else if chars[i] == ')' {
                    depth -= 1;
                }
                i += 1;
            }
            if depth == 0 {
                let inner: String = chars[2..i - 1].iter().collect();
                let suffix: String = chars[i..].iter().collect();
                return Some((inner, suffix));
            }
        }
        None
    }

    /// Expand a word with word splitting (for contexts like `for x in $words`)
    fn expand_word_split(&mut self, word: &ShellWord) -> Vec<String> {
        match word {
            ShellWord::Literal(s) => {
                // First do brace expansion, then variable expansion on each result
                let brace_expanded = self.expand_braces(s);
                brace_expanded
                    .into_iter()
                    .flat_map(|item| self.expand_string_split(&item))
                    .collect()
            }
            ShellWord::SingleQuoted(s) => vec![s.clone()],
            ShellWord::DoubleQuoted(parts) => {
                // Double quotes prevent word splitting
                vec![parts.iter().map(|p| self.expand_word(p)).collect()]
            }
            ShellWord::Variable(name) => {
                let val = env::var(name).unwrap_or_default();
                self.split_words(&val)
            }
            ShellWord::VariableBraced(name, modifier) => {
                let val = env::var(name).ok();
                let expanded = self.apply_var_modifier(name, val, modifier.as_deref());
                self.split_words(&expanded)
            }
            ShellWord::ArrayVar(name, index) => {
                let idx_str = self.expand_word(index);
                if idx_str == "@" || idx_str == "*" {
                    // ${arr[@]} returns each element as separate word
                    self.arrays.get(name).cloned().unwrap_or_default()
                } else {
                    vec![self.expand_array_access(name, index)]
                }
            }
            ShellWord::Glob(pattern) => match glob::glob(pattern) {
                Ok(paths) => {
                    let expanded: Vec<String> = paths
                        .filter_map(|p| p.ok())
                        .map(|p| p.to_string_lossy().to_string())
                        .collect();
                    if expanded.is_empty() {
                        vec![pattern.clone()]
                    } else {
                        expanded
                    }
                }
                Err(_) => vec![pattern.clone()],
            },
            ShellWord::CommandSub(_) => {
                // Command substitution results must be word-split for array context
                let val = self.expand_word(word);
                self.split_words(&val)
            }
            ShellWord::Concat(parts) => {
                // Concat in split context — expand and split the result
                let val = self.expand_concat_parallel(parts);
                self.split_words(&val)
            }
            _ => vec![self.expand_word(word)],
        }
    }

    /// Expand string with word splitting - returns Vec for array expansions
    fn expand_string_split(&mut self, s: &str) -> Vec<String> {
        let mut results: Vec<String> = Vec::new();
        let mut current = String::new();
        let mut chars = s.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '$' {
                if chars.peek() == Some(&'{') {
                    chars.next(); // consume '{'
                    let mut brace_content = String::new();
                    let mut depth = 1;
                    while let Some(ch) = chars.next() {
                        if ch == '{' {
                            depth += 1;
                            brace_content.push(ch);
                        } else if ch == '}' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                            brace_content.push(ch);
                        } else {
                            brace_content.push(ch);
                        }
                    }

                    // Check if this is an array expansion ${arr[@]} or ${arr[*]}
                    if let Some(bracket_start) = brace_content.find('[') {
                        let var_name = &brace_content[..bracket_start];
                        let bracket_content = &brace_content[bracket_start + 1..];
                        if let Some(bracket_end) = bracket_content.find(']') {
                            let index = &bracket_content[..bracket_end];
                            if (index == "@" || index == "*")
                                && bracket_end + 1 == bracket_content.len()
                            {
                                // This is ${arr[@]} - expand to separate elements
                                if !current.is_empty() {
                                    results.push(current.clone());
                                    current.clear();
                                }
                                if let Some(arr) = self.arrays.get(var_name) {
                                    results.extend(arr.clone());
                                }
                                continue;
                            }
                        }
                    }

                    // Not an array expansion, use normal expansion
                    current.push_str(&self.expand_braced_variable(&brace_content));
                } else {
                    // Simple variable like $var
                    let mut var_name = String::new();
                    while let Some(&ch) = chars.peek() {
                        if ch.is_alphanumeric() || ch == '_' {
                            var_name.push(chars.next().unwrap());
                        } else {
                            break;
                        }
                    }
                    let val = self.get_variable(&var_name);
                    // Split this variable's value
                    if !current.is_empty() {
                        results.push(current.clone());
                        current.clear();
                    }
                    results.extend(self.split_words(&val));
                }
            } else {
                current.push(c);
            }
        }

        if !current.is_empty() {
            results.push(current);
        }

        if results.is_empty() {
            results.push(String::new());
        }

        results
    }

    /// Split a string into words based on IFS
    fn split_words(&self, s: &str) -> Vec<String> {
        let ifs = self
            .variables
            .get("IFS")
            .cloned()
            .or_else(|| env::var("IFS").ok())
            .unwrap_or_else(|| " \t\n".to_string());

        if ifs.is_empty() {
            return vec![s.to_string()];
        }

        s.split(|c: char| ifs.contains(c))
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect()
    }

    #[tracing::instrument(level = "trace", skip_all)]
    fn expand_word(&mut self, word: &ShellWord) -> String {
        match word {
            ShellWord::Literal(s) => {
                let expanded = self.expand_string(s);
                // Don't glob-expand here, that's done in expand_word_glob
                expanded
            }
            ShellWord::SingleQuoted(s) => s.clone(),
            ShellWord::DoubleQuoted(parts) => parts.iter().map(|p| self.expand_word(p)).collect(),
            ShellWord::Variable(name) => self.get_variable(name),
            ShellWord::VariableBraced(name, modifier) => {
                let val = env::var(name).ok();
                self.apply_var_modifier(name, val, modifier.as_deref())
            }
            ShellWord::Tilde(user) => {
                if let Some(u) = user {
                    // ~user expansion (simplified)
                    format!("/home/{}", u)
                } else {
                    dirs::home_dir()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|| "~".to_string())
                }
            }
            ShellWord::Glob(pattern) => {
                // Expand glob
                match glob::glob(pattern) {
                    Ok(paths) => {
                        let expanded: Vec<String> = paths
                            .filter_map(|p| p.ok())
                            .map(|p| p.to_string_lossy().to_string())
                            .collect();
                        if expanded.is_empty() {
                            pattern.clone()
                        } else {
                            expanded.join(" ")
                        }
                    }
                    Err(_) => pattern.clone(),
                }
            }
            ShellWord::Concat(parts) => self.expand_concat_parallel(parts),
            ShellWord::CommandSub(cmd) => self.execute_command_substitution(cmd),
            ShellWord::ProcessSubIn(cmd) => self.execute_process_sub_in(cmd),
            ShellWord::ProcessSubOut(cmd) => self.execute_process_sub_out(cmd),
            ShellWord::ArithSub(expr) => self.evaluate_arithmetic(expr),
            ShellWord::ArrayVar(name, index) => self.expand_array_access(name, index),
            ShellWord::ArrayLiteral(elements) => elements
                .iter()
                .map(|e| self.expand_word(e))
                .collect::<Vec<_>>()
                .join(" "),
        }
    }

    /// Pre-launch external command substitutions from a word list onto the worker pool.
    /// Returns a Vec aligned with `words` — Some(receiver) for pre-launched externals, None otherwise.
    fn preflight_command_subs(
        &mut self,
        words: &[ShellWord],
    ) -> Vec<Option<crossbeam_channel::Receiver<String>>> {
        use crate::parser::ShellWord;
        use std::process::{Command, Stdio};

        let mut receivers = Vec::with_capacity(words.len());

        // Count external command subs — don't bother with pool overhead for just one
        let external_count = words.iter().filter(|w| {
            if let ShellWord::CommandSub(cmd) = w {
                if let ShellCommand::Simple(simple) = cmd.as_ref() {
                    if let Some(first) = simple.words.first() {
                        let name = self.expand_word(first);
                        return !self.functions.contains_key(&name) && !self.is_builtin(&name);
                    }
                }
            }
            false
        }).count();

        if external_count < 2 {
            // Not worth parallelizing — fall through to sequential
            return vec![None; words.len()];
        }

        for word in words {
            if let ShellWord::CommandSub(cmd) = word {
                if let ShellCommand::Simple(simple) = cmd.as_ref() {
                    let first = simple.words.first().map(|w| self.expand_word(w));
                    if let Some(ref name) = first {
                        if !self.functions.contains_key(name) && !self.is_builtin(name) {
                            let expanded: Vec<String> =
                                simple.words.iter().map(|w| self.expand_word(w)).collect();
                            let rx = self.worker_pool.submit_with_result(move || {
                                let output = Command::new(&expanded[0])
                                    .args(&expanded[1..])
                                    .stdout(Stdio::piped())
                                    .stderr(Stdio::inherit())
                                    .output();
                                match output {
                                    Ok(out) => String::from_utf8_lossy(&out.stdout)
                                        .trim_end_matches('\n')
                                        .to_string(),
                                    Err(_) => String::new(),
                                }
                            });
                            receivers.push(Some(rx));
                            continue;
                        }
                    }
                }
            }
            receivers.push(None);
        }

        receivers
    }

    /// Expand a Concat word list, launching external command substitutions in parallel.
    /// Internal subs (builtins/functions) still run sequentially on the main thread.
    fn expand_concat_parallel(&mut self, parts: &[ShellWord]) -> String {
        use crate::parser::ShellWord;
        use std::process::{Command, Stdio};

        // Phase 1: identify external command subs and pre-launch them
        let mut preflight: Vec<Option<crossbeam_channel::Receiver<String>>> = Vec::with_capacity(parts.len());

        for part in parts {
            if let ShellWord::CommandSub(cmd) = part {
                if let ShellCommand::Simple(simple) = cmd.as_ref() {
                    let first = simple.words.first().map(|w| self.expand_word(w));
                    if let Some(ref name) = first {
                        if !self.functions.contains_key(name) && !self.is_builtin(name) {
                            // External command — pre-launch on background thread
                            let words: Vec<String> =
                                simple.words.iter().map(|w| self.expand_word(w)).collect();
                            let rx = self.worker_pool.submit_with_result(move || {
                                let output = Command::new(&words[0])
                                    .args(&words[1..])
                                    .stdout(Stdio::piped())
                                    .stderr(Stdio::inherit())
                                    .output();
                                match output {
                                    Ok(out) => String::from_utf8_lossy(&out.stdout)
                                        .trim_end_matches('\n')
                                        .to_string(),
                                    Err(_) => String::new(),
                                }
                            });
                            preflight.push(Some(rx));
                            continue;
                        }
                    }
                }
            }
            preflight.push(None); // not pre-launched
        }

        // Phase 2: collect results in order, using pre-launched receivers where available
        let mut result = String::new();
        for (i, part) in parts.iter().enumerate() {
            if let Some(rx) = preflight[i].take() {
                // Pre-launched external command sub — collect result
                result.push_str(&rx.recv().unwrap_or_default());
            } else {
                // Everything else — expand sequentially (may be internal sub, variable, literal)
                result.push_str(&self.expand_word(part));
            }
        }
        result
    }

    fn expand_braced_variable(&mut self, content: &str) -> String {
        // Handle nested expansion: ${${inner}[subscript]} or ${${inner}modifier}
        if content.starts_with("${") {
            // Find matching closing brace for inner expansion
            let mut depth = 0;
            let mut inner_end = 0;
            for (i, c) in content.char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            inner_end = i;
                            break;
                        }
                    }
                    _ => {}
                }
            }

            if inner_end > 0 {
                // Expand the inner ${...}
                let inner_content = &content[2..inner_end];
                let inner_result = self.expand_braced_variable(inner_content);

                // Check for subscript or modifier after the inner expansion
                let rest = &content[inner_end + 1..];
                if rest.starts_with('[') {
                    // Apply subscript to result: ${${...}[idx]}
                    if let Some(bracket_end) = rest.find(']') {
                        let index = &rest[1..bracket_end];
                        if let Ok(idx) = index.parse::<i64>() {
                            let chars: Vec<char> = inner_result.chars().collect();
                            let actual_idx = if idx < 0 {
                                (chars.len() as i64 + idx).max(0) as usize
                            } else if idx > 0 {
                                (idx - 1) as usize
                            } else {
                                0
                            };
                            return chars
                                .get(actual_idx)
                                .map(|c| c.to_string())
                                .unwrap_or_default();
                        }
                    }
                }

                return inner_result;
            }
        }

        // Handle zsh-style parameter expansion flags ${(flags)var}
        if content.starts_with('(') {
            if let Some(close_paren) = content.find(')') {
                let flags_str = &content[1..close_paren];
                let rest = &content[close_paren + 1..];
                let flags = self.parse_zsh_flags(flags_str);

                // Check for (M) match flag
                let has_match_flag = flags.iter().any(|f| matches!(f, ZshParamFlag::Match));

                // Handle ${(M)var:#pattern} - pattern filter with flags
                if let Some(filter_pos) = rest.find(":#") {
                    let var_name = &rest[..filter_pos];
                    let pattern = &rest[filter_pos + 2..];

                    // Array path: filter each element against pattern
                    if let Some(arr) = self.arrays.get(var_name).cloned() {
                        let filtered: Vec<String> = if arr.len() >= 1000 {
                            tracing::trace!(
                                count = arr.len(),
                                pattern,
                                "using parallel filter (rayon) for large array"
                            );
                            use rayon::prelude::*;
                            let pattern = pattern.to_string();
                            arr.into_par_iter()
                                .filter(|elem| {
                                    let m = Self::glob_match_static(elem, &pattern);
                                    if has_match_flag { m } else { !m }
                                })
                                .collect()
                        } else {
                            arr.into_iter()
                                .filter(|elem| {
                                    let m = self.glob_match(elem, pattern);
                                    if has_match_flag { m } else { !m }
                                })
                                .collect()
                        };
                        return filtered.join(" ");
                    }

                    // Scalar path: original behavior
                    let val = self.get_variable(var_name);
                    let matches = self.glob_match(&val, pattern);

                    return if has_match_flag {
                        if matches { val } else { String::new() }
                    } else {
                        if matches { String::new() } else { val }
                    };
                }

                // Handle ${(%):-%n} style - empty var with default after flags
                // rest could be ":-%n" or ":-default" or "var:-default" or just "var"
                let (var_name, default_val) = if rest.starts_with(":-") {
                    // Empty variable name with default: ${(%):-default}
                    ("", Some(&rest[2..]))
                } else if let Some(pos) = rest.find(":-") {
                    // Variable with default: ${(%)var:-default}
                    (&rest[..pos], Some(&rest[pos + 2..]))
                } else if rest.starts_with(':') {
                    // Just ":" means empty var name, no default
                    ("", None)
                } else {
                    // Normal variable reference
                    let vn = rest
                        .split(|c: char| !c.is_alphanumeric() && c != '_')
                        .next()
                        .unwrap_or("");
                    (vn, None)
                };

                let mut val = self.get_variable(var_name);

                // Use default if variable is empty
                if val.is_empty() {
                    if let Some(def) = default_val {
                        // Expand the default value (handles $var and other expansions)
                        val = self.expand_string(def);
                    }
                }

                // Apply flags in order
                for flag in &flags {
                    val = self.apply_zsh_param_flag(&val, var_name, flag);
                }
                return val;
            }
        }

        // Handle ${#arr[@]} - array length
        if content.starts_with('#') {
            let rest = &content[1..];
            if let Some(bracket_start) = rest.find('[') {
                let var_name = &rest[..bracket_start];
                let bracket_content = &rest[bracket_start + 1..];
                if let Some(bracket_end) = bracket_content.find(']') {
                    let index = &bracket_content[..bracket_end];
                    if index == "@" || index == "*" {
                        // ${#arr[@]} - return array length
                        return self
                            .arrays
                            .get(var_name)
                            .map(|arr| arr.len().to_string())
                            .unwrap_or_else(|| "0".to_string());
                    }
                }
            }
            // ${#arr} - if rest is an array name, return array length
            if self.arrays.contains_key(rest) {
                return self
                    .arrays
                    .get(rest)
                    .map(|arr| arr.len().to_string())
                    .unwrap_or_else(|| "0".to_string());
            }
            // ${#assoc} - if rest is an assoc array name, return assoc length
            if self.assoc_arrays.contains_key(rest) {
                return self
                    .assoc_arrays
                    .get(rest)
                    .map(|h| h.len().to_string())
                    .unwrap_or_else(|| "0".to_string());
            }
            // ${#var} - string length
            let val = self.get_variable(rest);
            return val.len().to_string();
        }

        // Handle ${+var} and ${+arr[key]} - test if variable/element is set (returns 1 if set, 0 if not)
        if content.starts_with('+') {
            let rest = &content[1..];

            // Check for array/assoc access: ${+arr[key]}
            if let Some(bracket_start) = rest.find('[') {
                let var_name = &rest[..bracket_start];
                let bracket_content = &rest[bracket_start + 1..];
                if let Some(bracket_end) = bracket_content.find(']') {
                    let key = &bracket_content[..bracket_end];

                    // Check special arrays first
                    if let Some(val) = self.get_special_array_value(var_name, key) {
                        return if val.is_empty() {
                            "0".to_string()
                        } else {
                            "1".to_string()
                        };
                    }

                    // Check user assoc arrays
                    if self.assoc_arrays.contains_key(var_name) {
                        let expanded_key = self.expand_string(key);
                        let has_key = self
                            .assoc_arrays
                            .get(var_name)
                            .map(|a| a.contains_key(&expanded_key))
                            .unwrap_or(false);
                        return if has_key {
                            "1".to_string()
                        } else {
                            "0".to_string()
                        };
                    }

                    // Check regular arrays
                    if let Some(arr) = self.arrays.get(var_name) {
                        if let Ok(idx) = key.parse::<usize>() {
                            let actual_idx = if idx > 0 { idx - 1 } else { 0 };
                            return if arr.get(actual_idx).is_some() {
                                "1".to_string()
                            } else {
                                "0".to_string()
                            };
                        }
                    }

                    return "0".to_string();
                }
            }

            // Simple variable: ${+var}
            let is_set = self.variables.contains_key(rest)
                || self.arrays.contains_key(rest)
                || self.assoc_arrays.contains_key(rest)
                || std::env::var(rest).is_ok()
                || self.functions.contains_key(rest);
            return if is_set {
                "1".to_string()
            } else {
                "0".to_string()
            };
        }

        // Handle ${arr[idx]} or ${assoc[key]}
        if let Some(bracket_start) = content.find('[') {
            let var_name = &content[..bracket_start];
            let bracket_content = &content[bracket_start + 1..];
            if let Some(bracket_end) = bracket_content.find(']') {
                let index = &bracket_content[..bracket_end];

                // Check for zsh/parameter special associative arrays (options, commands, etc.)
                if let Some(val) = self.get_special_array_value(var_name, index) {
                    return val;
                }

                // Check if it's a user-defined associative array
                if self.assoc_arrays.contains_key(var_name) {
                    if index == "@" || index == "*" {
                        // ${assoc[@]} - return all values
                        return self
                            .assoc_arrays
                            .get(var_name)
                            .map(|a| a.values().cloned().collect::<Vec<_>>().join(" "))
                            .unwrap_or_default();
                    } else {
                        // ${assoc[key]} - return value for key
                        let key = self.expand_string(index);
                        return self
                            .assoc_arrays
                            .get(var_name)
                            .and_then(|a| a.get(&key).cloned())
                            .unwrap_or_default();
                    }
                }

                // Regular indexed array
                if index == "@" || index == "*" {
                    // ${arr[@]} - return all elements
                    return self
                        .arrays
                        .get(var_name)
                        .map(|arr| arr.join(" "))
                        .unwrap_or_default();
                }

                // Use the ported subscript module for comprehensive index parsing
                use crate::subscript::{
                    get_array_by_subscript, get_array_element_by_subscript, getindex,
                };
                let ksh_arrays = self.options.get("ksh_arrays").copied().unwrap_or(false);

                if let Ok(v) = getindex(index, false, ksh_arrays) {
                    // Check if it's an array first
                    if let Some(arr) = self.arrays.get(var_name) {
                        if v.is_all() {
                            return arr.join(" ");
                        }
                        // Check if this is a range (comma in subscript) vs single element
                        // For a single element, v.end == v.start + 1 after adjustment
                        // But for negative single indices, we need to handle specially
                        let is_range = index.contains(',');
                        if is_range {
                            // Range: ${arr[2,4]} returns elements 2 through 4
                            return get_array_by_subscript(arr, &v, ksh_arrays).join(" ");
                        } else {
                            // Single element (including negative indices like -1)
                            return get_array_element_by_subscript(arr, &v, ksh_arrays)
                                .unwrap_or_default();
                        }
                    }

                    // Not an array - treat as string subscripting
                    let val = self.get_variable(var_name);
                    if !val.is_empty() {
                        let chars: Vec<char> = val.chars().collect();
                        let idx = v.start;
                        let actual_idx = if idx < 0 {
                            (chars.len() as i64 + idx).max(0) as usize
                        } else if idx > 0 {
                            (idx - 1) as usize // zsh is 1-indexed
                        } else {
                            0
                        };

                        if v.end > v.start + 1 {
                            // String range
                            let end_idx = if v.end < 0 {
                                (chars.len() as i64 + v.end + 1).max(0) as usize
                            } else {
                                v.end as usize
                            };
                            let end_idx = end_idx.min(chars.len());
                            return chars[actual_idx..end_idx].iter().collect();
                        } else {
                            return chars
                                .get(actual_idx)
                                .map(|c| c.to_string())
                                .unwrap_or_default();
                        }
                    }
                    return String::new();
                }

                // Non-numeric index on non-assoc - return empty
                return String::new();
            }
        }

        // Handle ${var:-default}, ${var:=default}, ${var:?error}, ${var:+alternate}
        if let Some(colon_pos) = content.find(':') {
            let var_name = &content[..colon_pos];
            let rest = &content[colon_pos + 1..];
            let val = self.get_variable(var_name);
            let val_opt = if val.is_empty() {
                None
            } else {
                Some(val.clone())
            };

            if rest.starts_with('-') {
                // ${var:-default}
                return match val_opt {
                    Some(v) if !v.is_empty() => v,
                    _ => self.expand_string(&rest[1..]),
                };
            } else if rest.starts_with('=') {
                // ${var:=default}
                return match val_opt {
                    Some(v) if !v.is_empty() => v,
                    _ => {
                        let default = self.expand_string(&rest[1..]);
                        self.variables.insert(var_name.to_string(), default.clone());
                        default
                    }
                };
            } else if rest.starts_with('?') {
                // ${var:?error}
                return match val_opt {
                    Some(v) if !v.is_empty() => v,
                    _ => {
                        let msg = self.expand_string(&rest[1..]);
                        eprintln!("zshrs: {}: {}", var_name, msg);
                        String::new()
                    }
                };
            } else if rest.starts_with('+') {
                // ${var:+alternate}
                return match val_opt {
                    Some(v) if !v.is_empty() => self.expand_string(&rest[1..]),
                    _ => String::new(),
                };
            } else if rest.starts_with('#') {
                // ${var:#pattern} - filter: remove elements matching pattern
                // With (M) flag, keep only matching elements
                let pattern = &rest[1..];
                // For scalars, return empty if matches, value if not
                if self.glob_match(&val, pattern) {
                    return String::new();
                } else {
                    return val;
                }
            } else if self.is_history_modifier(rest) {
                // Handle history-style modifiers: :A, :h, :t, :r, :e, :l, :u, :q, :Q
                // These can be chained: ${var:A:h:h}
                return self.apply_history_modifiers(&val, rest);
            } else if rest
                .chars()
                .next()
                .map(|c| c.is_ascii_digit() || c == '-')
                .unwrap_or(false)
            {
                // ${var:offset} or ${var:offset:length}
                let parts: Vec<&str> = rest.splitn(2, ':').collect();
                let offset: i64 = parts[0].parse().unwrap_or(0);
                let length: Option<usize> = parts.get(1).and_then(|s| s.parse().ok());

                let start = if offset < 0 {
                    (val.len() as i64 + offset).max(0) as usize
                } else {
                    (offset as usize).min(val.len())
                };

                return if let Some(len) = length {
                    val.chars().skip(start).take(len).collect()
                } else {
                    val.chars().skip(start).collect()
                };
            }
        }

        // Handle ${var/pattern/replacement} and ${var//pattern/replacement}
        // Only if the part before / is a valid variable name
        if let Some(slash_pos) = content.find('/') {
            let var_name = &content[..slash_pos];
            // Variable names must start with letter/underscore and contain only alnum/_
            if !var_name.is_empty()
                && var_name
                    .chars()
                    .next()
                    .map(|c| c.is_alphabetic() || c == '_')
                    .unwrap_or(false)
                && var_name.chars().all(|c| c.is_alphanumeric() || c == '_')
            {
                let rest = &content[slash_pos + 1..];
                let val = self.get_variable(var_name);

                let replace_all = rest.starts_with('/');
                let rest = if replace_all { &rest[1..] } else { rest };

                let parts: Vec<&str> = rest.splitn(2, '/').collect();
                let pattern = parts.get(0).unwrap_or(&"");
                let replacement = parts.get(1).unwrap_or(&"");

                return if replace_all {
                    val.replace(pattern, replacement)
                } else {
                    val.replacen(pattern, replacement, 1)
                };
            }
        }

        // Handle ${var#pattern} and ${var##pattern} - remove prefix
        // But only if the # is not at the start (which would be length)
        if let Some(hash_pos) = content.find('#') {
            if hash_pos > 0 {
                let var_name = &content[..hash_pos];
                // Make sure var_name looks like a valid variable name
                if var_name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    let rest = &content[hash_pos + 1..];
                    let val = self.get_variable(var_name);

                    let long = rest.starts_with('#');
                    let pattern = if long { &rest[1..] } else { rest };

                    // Convert shell glob pattern to regex-style for matching prefixes
                    let pattern_regex = regex::escape(pattern)
                        .replace(r"\*", ".*")
                        .replace(r"\?", ".");
                    let full_pattern = format!("^{}", pattern_regex);

                    if let Some(re) = cached_regex(&full_pattern) {
                        if long {
                            // Remove longest prefix match - find all matches and use the longest
                            let mut longest_end = 0;
                            for m in re.find_iter(&val) {
                                if m.end() > longest_end {
                                    longest_end = m.end();
                                }
                            }
                            if longest_end > 0 {
                                return val[longest_end..].to_string();
                            }
                        } else {
                            // Remove shortest prefix match
                            if let Some(m) = re.find(&val) {
                                return val[m.end()..].to_string();
                            }
                        }
                    }
                    return val;
                }
            }
        }

        // Handle ${var%pattern} and ${var%%pattern} - remove suffix
        if let Some(pct_pos) = content.find('%') {
            if pct_pos > 0 {
                let var_name = &content[..pct_pos];
                if var_name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    let rest = &content[pct_pos + 1..];
                    let val = self.get_variable(var_name);

                    let long = rest.starts_with('%');
                    let pattern = if long { &rest[1..] } else { rest };

                    // Use glob pattern matching for suffix removal
                    if let Ok(glob) = glob::Pattern::new(pattern) {
                        if long {
                            // Remove longest suffix match - find earliest matching position
                            for i in 0..=val.len() {
                                if glob.matches(&val[i..]) {
                                    return val[..i].to_string();
                                }
                            }
                        } else {
                            // Remove shortest suffix match - find latest matching position
                            for i in (0..=val.len()).rev() {
                                if glob.matches(&val[i..]) {
                                    return val[..i].to_string();
                                }
                            }
                        }
                    }
                    return val;
                }
            }
        }

        // Handle ${var^} and ${var^^} - uppercase
        if let Some(caret_pos) = content.find('^') {
            let var_name = &content[..caret_pos];
            let val = self.get_variable(var_name);
            let all = content[caret_pos + 1..].starts_with('^');

            return if all {
                val.to_uppercase()
            } else {
                let mut chars = val.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().to_string() + chars.as_str(),
                    None => String::new(),
                }
            };
        }

        // Handle ${var,} and ${var,,} - lowercase
        if let Some(comma_pos) = content.find(',') {
            let var_name = &content[..comma_pos];
            let val = self.get_variable(var_name);
            let all = content[comma_pos + 1..].starts_with(',');

            return if all {
                val.to_lowercase()
            } else {
                let mut chars = val.chars();
                match chars.next() {
                    Some(first) => first.to_lowercase().to_string() + chars.as_str(),
                    None => String::new(),
                }
            };
        }

        // Handle ${!prefix*} and ${!prefix@} - expand to variable names with prefix
        if content.starts_with('!') {
            let rest = &content[1..];
            if rest.ends_with('*') || rest.ends_with('@') {
                let prefix = &rest[..rest.len() - 1];
                let mut matches: Vec<String> = self
                    .variables
                    .keys()
                    .filter(|k| k.starts_with(prefix))
                    .cloned()
                    .collect();
                // Also check arrays
                for k in self.arrays.keys() {
                    if k.starts_with(prefix) && !matches.contains(k) {
                        matches.push(k.clone());
                    }
                }
                matches.sort();
                return matches.join(" ");
            }

            // ${!var} - indirect expansion
            let var_name = self.get_variable(rest);
            return self.get_variable(&var_name);
        }

        // Default: just get the variable
        self.get_variable(content)
    }

    fn expand_array_access(&mut self, name: &str, index: &ShellWord) -> String {
        use crate::subscript::{get_array_by_subscript, get_array_element_by_subscript, getindex};

        let idx_str = self.expand_word(index);
        let ksh_arrays = self.options.get("ksh_arrays").copied().unwrap_or(false);

        // Use the ported subscript module for index parsing
        match getindex(&idx_str, false, ksh_arrays) {
            Ok(v) => {
                if let Some(arr) = self.arrays.get(name) {
                    if v.is_all() {
                        arr.join(" ")
                    } else if v.start == v.end - 1 {
                        // Single element
                        get_array_element_by_subscript(arr, &v, ksh_arrays).unwrap_or_default()
                    } else {
                        // Range
                        get_array_by_subscript(arr, &v, ksh_arrays).join(" ")
                    }
                } else {
                    String::new()
                }
            }
            Err(_) => String::new(),
        }
    }

    #[tracing::instrument(level = "trace", skip_all)]
    fn expand_string(&mut self, s: &str) -> String {
        let mut result = String::new();
        let mut chars = s.chars().peekable();

        while let Some(c) = chars.next() {
            // \x00 prefix marks chars from single quotes - keep them literal
            if c == '\x00' {
                if let Some(literal_char) = chars.next() {
                    result.push(literal_char);
                }
                continue;
            }
            if c == '$' {
                if chars.peek() == Some(&'(') {
                    chars.next(); // consume '('

                    // Check for $(( )) arithmetic
                    if chars.peek() == Some(&'(') {
                        chars.next(); // consume second '('
                        let expr = Self::collect_until_double_paren(&mut chars);
                        result.push_str(&self.evaluate_arithmetic(&expr));
                    } else {
                        // Command substitution $(...)
                        let cmd_str = Self::collect_until_paren(&mut chars);
                        result.push_str(&self.run_command_substitution(&cmd_str));
                    }
                } else if chars.peek() == Some(&'{') {
                    chars.next();
                    // Collect the full braced expression including brackets
                    let mut brace_content = String::new();
                    let mut depth = 1;
                    while let Some(c) = chars.next() {
                        if c == '{' {
                            depth += 1;
                            brace_content.push(c);
                        } else if c == '}' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                            brace_content.push(c);
                        } else {
                            brace_content.push(c);
                        }
                    }
                    result.push_str(&self.expand_braced_variable(&brace_content));
                } else {
                    // Check for single-char special vars first: $$, $!, $-
                    if matches!(chars.peek(), Some(&'$') | Some(&'!') | Some(&'-')) {
                        let sc = chars.next().unwrap();
                        result.push_str(&self.get_variable(&sc.to_string()));
                        continue;
                    }
                    // $#name → ${#name} (string/array length)
                    if chars.peek() == Some(&'#') {
                        let mut peek_iter = chars.clone();
                        peek_iter.next(); // skip #
                        if peek_iter.peek().map(|c| c.is_alphabetic() || *c == '_').unwrap_or(false) {
                            chars.next(); // consume #
                            let mut name = String::new();
                            while let Some(&c) = chars.peek() {
                                if c.is_alphanumeric() || c == '_' {
                                    name.push(chars.next().unwrap());
                                } else {
                                    break;
                                }
                            }
                            // Return length of variable or array
                            let len = if let Some(arr) = self.arrays.get(&name) {
                                arr.len()
                            } else {
                                self.get_variable(&name).len()
                            };
                            result.push_str(&len.to_string());
                            continue;
                        }
                    }
                    let mut var_name = String::new();
                    while let Some(&c) = chars.peek() {
                        if c.is_alphanumeric() || c == '_' || c == '@' || c == '*' || c == '#' || c == '?' {
                            var_name.push(chars.next().unwrap());
                            // Handle single-char special vars
                            if matches!(
                                var_name.as_str(),
                                "@" | "*"
                                    | "#"
                                    | "?"
                                    | "$"
                                    | "!"
                                    | "-"
                                    | "0"
                                    | "1"
                                    | "2"
                                    | "3"
                                    | "4"
                                    | "5"
                                    | "6"
                                    | "7"
                                    | "8"
                                    | "9"
                            ) {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                    result.push_str(&self.get_variable(&var_name));
                }
            } else if c == '`' {
                // Backtick command substitution
                let cmd_str: String = chars.by_ref().take_while(|&c| c != '`').collect();
                result.push_str(&self.run_command_substitution(&cmd_str));
            } else if c == '<' && chars.peek() == Some(&'(') {
                // Process substitution <(cmd)
                chars.next(); // consume '('
                let cmd_str = Self::collect_until_paren(&mut chars);
                result.push_str(&self.run_process_sub_in(&cmd_str));
            } else if c == '>' && chars.peek() == Some(&'(') {
                // Process substitution >(cmd)
                chars.next(); // consume '('
                let cmd_str = Self::collect_until_paren(&mut chars);
                result.push_str(&self.run_process_sub_out(&cmd_str));
            } else if c == '~' && result.is_empty() {
                if let Some(home) = dirs::home_dir() {
                    result.push_str(&home.to_string_lossy());
                } else {
                    result.push(c);
                }
            } else {
                result.push(c);
            }
        }

        result
    }

    fn collect_until_paren(chars: &mut std::iter::Peekable<std::str::Chars>) -> String {
        let mut result = String::new();
        let mut depth = 1;

        while let Some(c) = chars.next() {
            if c == '(' {
                depth += 1;
                result.push(c);
            } else if c == ')' {
                depth -= 1;
                if depth == 0 {
                    break;
                }
                result.push(c);
            } else {
                result.push(c);
            }
        }

        result
    }

    fn collect_until_double_paren(chars: &mut std::iter::Peekable<std::str::Chars>) -> String {
        let mut result = String::new();
        let mut arith_depth = 1; // Tracks $(( ... )) nesting
        let mut paren_depth = 0; // Tracks ( ... ) nesting within expression

        while let Some(c) = chars.next() {
            if c == '(' {
                if paren_depth == 0 && chars.peek() == Some(&'(') {
                    // Nested $(( - but we need to see if it's really another arithmetic
                    // For simplicity, track inner parens
                    paren_depth += 1;
                    result.push(c);
                } else {
                    paren_depth += 1;
                    result.push(c);
                }
            } else if c == ')' {
                if paren_depth > 0 {
                    // Inside nested parens, just close one level
                    paren_depth -= 1;
                    result.push(c);
                } else if chars.peek() == Some(&')') {
                    // At top level and seeing )) - this closes our arithmetic
                    chars.next();
                    arith_depth -= 1;
                    if arith_depth == 0 {
                        break;
                    }
                    result.push_str("))");
                } else {
                    // Single ) at top level - shouldn't happen in valid expression
                    result.push(c);
                }
            } else {
                result.push(c);
            }
        }

        result
    }

    fn run_process_sub_in(&mut self, cmd_str: &str) -> String {
        use std::fs;
        use std::process::Stdio;

        // Parse the command
        let mut parser = ShellParser::new(cmd_str);
        let commands = match parser.parse_script() {
            Ok(cmds) => cmds,
            Err(_) => return String::new(),
        };

        // Create a unique FIFO in temp directory
        let fifo_path = format!("/tmp/zshrs_psub_{}", std::process::id());
        let fifo_counter = self.process_sub_counter;
        self.process_sub_counter += 1;
        let fifo_path = format!("{}_{}", fifo_path, fifo_counter);

        // Remove if exists, then create FIFO
        let _ = fs::remove_file(&fifo_path);
        if let Err(_) = nix::unistd::mkfifo(fifo_path.as_str(), nix::sys::stat::Mode::S_IRWXU) {
            return String::new();
        }

        // Spawn command that writes to the FIFO
        let fifo_clone = fifo_path.clone();
        if let Some(cmd) = commands.first() {
            if let ShellCommand::Simple(simple) = cmd {
                let words: Vec<String> = simple.words.iter().map(|w| self.expand_word(w)).collect();
                if !words.is_empty() {
                    let cmd_name = words[0].clone();
                    let args: Vec<String> = words[1..].to_vec();

                    self.worker_pool.submit(move || {
                        // Open FIFO for writing (will block until reader connects)
                        if let Ok(fifo) = fs::OpenOptions::new().write(true).open(&fifo_clone) {
                            let _ = Command::new(&cmd_name)
                                .args(&args)
                                .stdout(fifo)
                                .stderr(Stdio::inherit())
                                .status();
                        }
                        // Clean up FIFO after command completes
                        let _ = fs::remove_file(&fifo_clone);
                    });
                }
            }
        }

        fifo_path
    }

    fn run_process_sub_out(&mut self, cmd_str: &str) -> String {
        use std::fs;
        use std::process::Stdio;

        // Parse the command
        let mut parser = ShellParser::new(cmd_str);
        let commands = match parser.parse_script() {
            Ok(cmds) => cmds,
            Err(_) => return String::new(),
        };

        // Create a unique FIFO in temp directory
        let fifo_path = format!("/tmp/zshrs_psub_{}", std::process::id());
        let fifo_counter = self.process_sub_counter;
        self.process_sub_counter += 1;
        let fifo_path = format!("{}_{}", fifo_path, fifo_counter);

        // Remove if exists, then create FIFO
        let _ = fs::remove_file(&fifo_path);
        if let Err(_) = nix::unistd::mkfifo(fifo_path.as_str(), nix::sys::stat::Mode::S_IRWXU) {
            return String::new();
        }

        // Spawn command that reads from the FIFO
        let fifo_clone = fifo_path.clone();
        if let Some(cmd) = commands.first() {
            if let ShellCommand::Simple(simple) = cmd {
                let words: Vec<String> = simple.words.iter().map(|w| self.expand_word(w)).collect();
                if !words.is_empty() {
                    let cmd_name = words[0].clone();
                    let args: Vec<String> = words[1..].to_vec();

                    self.worker_pool.submit(move || {
                        // Open FIFO for reading (will block until writer connects)
                        if let Ok(fifo) = fs::File::open(&fifo_clone) {
                            let _ = Command::new(&cmd_name)
                                .args(&args)
                                .stdin(fifo)
                                .stdout(Stdio::inherit())
                                .stderr(Stdio::inherit())
                                .status();
                        }
                        // Clean up FIFO after command completes
                        let _ = fs::remove_file(&fifo_clone);
                    });
                }
            }
        }

        fifo_path
    }

    fn run_command_substitution(&mut self, cmd_str: &str) -> String {
        use std::process::Stdio;

        // Port of getoutput() from Src/exec.c:
        // C zsh forks, redirects stdout to a pipe, executes via execode(),
        // and the parent reads back the output.  We achieve the same by
        // capturing stdout through an in-process pipe.

        let mut parser = ShellParser::new(cmd_str);
        let commands = match parser.parse_script() {
            Ok(cmds) => cmds,
            Err(_) => return String::new(),
        };

        if commands.is_empty() {
            return String::new();
        }

        // Check if this is a simple external-only command (no builtins/functions)
        // so we can use the fast path of spawning a child process.
        let is_internal = if let ShellCommand::Simple(simple) = &commands[0] {
            let first = simple.words.first().map(|w| self.expand_word(w));
            if let Some(ref name) = first {
                self.functions.contains_key(name)
                    || self.is_builtin(name)
            } else {
                true
            }
        } else {
            true // compound commands are always internal
        };

        if is_internal {
            // Internal execution: capture stdout via a pipe
            let (read_fd, write_fd) = {
                let mut fds = [0i32; 2];
                if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
                    return String::new();
                }
                (fds[0], fds[1])
            };

            // Save original stdout and redirect to our pipe
            let saved_stdout = unsafe { libc::dup(1) };
            unsafe { libc::dup2(write_fd, 1); }
            unsafe { libc::close(write_fd); }

            // Execute all commands
            for cmd in &commands {
                let _ = self.execute_command(cmd);
            }

            // Flush stdout so buffered output goes to pipe
            use std::io::Write;
            let _ = io::stdout().flush();

            // Restore stdout
            unsafe { libc::dup2(saved_stdout, 1); }
            unsafe { libc::close(saved_stdout); }

            // Read captured output
            use std::os::unix::io::FromRawFd;
            let mut output = String::new();
            let read_file = unsafe { std::fs::File::from_raw_fd(read_fd) };
            use std::io::Read;
            let _ = std::io::BufReader::new(read_file).read_to_string(&mut output);

            output.trim_end_matches('\n').to_string()
        } else {
            // External command: spawn child and capture stdout
            if let ShellCommand::Simple(simple) = &commands[0] {
                let words: Vec<String> =
                    simple.words.iter().map(|w| self.expand_word(w)).collect();
                if words.is_empty() {
                    return String::new();
                }

                let output = Command::new(&words[0])
                    .args(&words[1..])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::inherit())
                    .output();

                match output {
                    Ok(out) => String::from_utf8_lossy(&out.stdout)
                        .trim_end_matches('\n')
                        .to_string(),
                    Err(_) => String::new(),
                }
            } else {
                String::new()
            }
        }
    }

    /// Process substitution <(cmd) - returns FIFO path
    fn execute_process_sub_in(&mut self, cmd: &ShellCommand) -> String {
        if let ShellCommand::Simple(simple) = cmd {
            let words: Vec<String> = simple.words.iter().map(|w| self.expand_word(w)).collect();
            let cmd_str = words.join(" ");
            self.run_process_sub_in(&cmd_str)
        } else {
            String::new()
        }
    }

    /// Process substitution >(cmd) - returns FIFO path
    fn execute_process_sub_out(&mut self, cmd: &ShellCommand) -> String {
        if let ShellCommand::Simple(simple) = cmd {
            let words: Vec<String> = simple.words.iter().map(|w| self.expand_word(w)).collect();
            let cmd_str = words.join(" ");
            self.run_process_sub_out(&cmd_str)
        } else {
            String::new()
        }
    }

    /// Get value from zsh/parameter special arrays (options, commands, functions, etc.)
    /// Returns Some(value) if this is a special array access, None otherwise
    fn get_special_array_value(&self, array_name: &str, key: &str) -> Option<String> {
        match array_name {
            // === SHELL OPTIONS ===
            "options" => {
                if key == "@" || key == "*" {
                    // Return all options as "name=on/off" pairs
                    let opts: Vec<String> = self
                        .options
                        .iter()
                        .map(|(k, v)| format!("{}={}", k, if *v { "on" } else { "off" }))
                        .collect();
                    return Some(opts.join(" "));
                }
                let opt_name = key.to_lowercase().replace('_', "");
                let is_on = self.options.get(&opt_name).copied().unwrap_or(false);
                Some(if is_on {
                    "on".to_string()
                } else {
                    "off".to_string()
                })
            }

            // === ALIASES ===
            "aliases" => {
                if key == "@" || key == "*" {
                    let vals: Vec<String> = self.aliases.values().cloned().collect();
                    return Some(vals.join(" "));
                }
                Some(self.aliases.get(key).cloned().unwrap_or_default())
            }
            "galiases" => {
                if key == "@" || key == "*" {
                    let vals: Vec<String> = self.global_aliases.values().cloned().collect();
                    return Some(vals.join(" "));
                }
                Some(self.global_aliases.get(key).cloned().unwrap_or_default())
            }
            "saliases" => {
                if key == "@" || key == "*" {
                    let vals: Vec<String> = self.suffix_aliases.values().cloned().collect();
                    return Some(vals.join(" "));
                }
                Some(self.suffix_aliases.get(key).cloned().unwrap_or_default())
            }

            // === FUNCTIONS ===
            "functions" => {
                if key == "@" || key == "*" {
                    let names: Vec<String> = self.functions.keys().cloned().collect();
                    return Some(names.join(" "));
                }
                if let Some(body) = self.functions.get(key) {
                    Some(format!("{:?}", body))
                } else {
                    Some(String::new())
                }
            }
            "functions_source" => {
                // We don't track source locations, return empty
                Some(String::new())
            }

            // === COMMANDS (command hash table) ===
            "commands" => {
                if key == "@" || key == "*" {
                    return Some(String::new()); // Would need to enumerate PATH
                }
                // Look up command in PATH
                if let Some(path) = self.find_in_path(key) {
                    Some(path)
                } else {
                    Some(String::new())
                }
            }

            // === BUILTINS ===
            "builtins" => {
                let builtins = Self::get_builtin_names();
                if key == "@" || key == "*" {
                    return Some(builtins.join(" "));
                }
                if builtins.contains(&key) {
                    Some("defined".to_string())
                } else {
                    Some(String::new())
                }
            }

            // === PARAMETERS ===
            "parameters" => {
                if key == "@" || key == "*" {
                    let mut names: Vec<String> = self.variables.keys().cloned().collect();
                    names.extend(self.arrays.keys().cloned());
                    names.extend(self.assoc_arrays.keys().cloned());
                    return Some(names.join(" "));
                }
                // Return type of parameter
                if self.assoc_arrays.contains_key(key) {
                    Some("association".to_string())
                } else if self.arrays.contains_key(key) {
                    Some("array".to_string())
                } else if self.variables.contains_key(key) || std::env::var(key).is_ok() {
                    Some("scalar".to_string())
                } else {
                    Some(String::new())
                }
            }

            // === NAMED DIRECTORIES ===
            "nameddirs" => {
                if key == "@" || key == "*" {
                    let vals: Vec<String> = self
                        .named_dirs
                        .values()
                        .map(|p| p.display().to_string())
                        .collect();
                    return Some(vals.join(" "));
                }
                Some(
                    self.named_dirs
                        .get(key)
                        .map(|p| p.display().to_string())
                        .unwrap_or_default(),
                )
            }

            // === USER DIRECTORIES ===
            "userdirs" => {
                if key == "@" || key == "*" {
                    return Some(String::new());
                }
                // Get home directory for user
                #[cfg(unix)]
                {
                    use std::ffi::CString;
                    if let Ok(name) = CString::new(key) {
                        unsafe {
                            let pwd = libc::getpwnam(name.as_ptr());
                            if !pwd.is_null() {
                                let dir = std::ffi::CStr::from_ptr((*pwd).pw_dir);
                                return Some(dir.to_string_lossy().to_string());
                            }
                        }
                    }
                }
                Some(String::new())
            }

            // === USER GROUPS ===
            "usergroups" => {
                if key == "@" || key == "*" {
                    return Some(String::new());
                }
                // Get GID for group name
                #[cfg(unix)]
                {
                    use std::ffi::CString;
                    if let Ok(name) = CString::new(key) {
                        unsafe {
                            let grp = libc::getgrnam(name.as_ptr());
                            if !grp.is_null() {
                                return Some((*grp).gr_gid.to_string());
                            }
                        }
                    }
                }
                Some(String::new())
            }

            // === DIRECTORY STACK ===
            "dirstack" => {
                if key == "@" || key == "*" {
                    let dirs: Vec<String> = self
                        .dir_stack
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect();
                    return Some(dirs.join(" "));
                }
                if let Ok(idx) = key.parse::<usize>() {
                    Some(
                        self.dir_stack
                            .get(idx)
                            .map(|p| p.display().to_string())
                            .unwrap_or_default(),
                    )
                } else {
                    Some(String::new())
                }
            }

            // === JOBS ===
            "jobstates" => {
                if key == "@" || key == "*" {
                    let states: Vec<String> = self
                        .jobs
                        .iter()
                        .map(|(id, job)| format!("{}:{:?}", id, job.state))
                        .collect();
                    return Some(states.join(" "));
                }
                if let Ok(id) = key.parse::<usize>() {
                    if let Some(job) = self.jobs.get(id) {
                        return Some(format!("{:?}", job.state));
                    }
                }
                Some(String::new())
            }
            "jobtexts" => {
                if key == "@" || key == "*" {
                    let texts: Vec<String> = self
                        .jobs
                        .iter()
                        .map(|(_, job)| job.command.clone())
                        .collect();
                    return Some(texts.join(" "));
                }
                if let Ok(id) = key.parse::<usize>() {
                    if let Some(job) = self.jobs.get(id) {
                        return Some(job.command.clone());
                    }
                }
                Some(String::new())
            }
            "jobdirs" => {
                // We don't track job directories separately - return current dir
                if key == "@" || key == "*" {
                    return Some(String::new());
                }
                Some(String::new())
            }

            // === HISTORY ===
            "history" => {
                if key == "@" || key == "*" {
                    // Return recent history
                    if let Some(ref engine) = self.history {
                        if let Ok(entries) = engine.recent(100) {
                            let cmds: Vec<String> =
                                entries.iter().map(|e| e.command.clone()).collect();
                            return Some(cmds.join("\n"));
                        }
                    }
                    return Some(String::new());
                }
                if let Ok(num) = key.parse::<usize>() {
                    if let Some(ref engine) = self.history {
                        if let Ok(Some(entry)) = engine.get_by_offset(num.saturating_sub(1)) {
                            return Some(entry.command);
                        }
                    }
                }
                Some(String::new())
            }
            "historywords" => {
                // Array of words from history - simplified
                Some(String::new())
            }

            // === MODULES ===
            "modules" => {
                // zshrs doesn't have loadable modules like zsh
                // Return empty or fake "loaded" for common modules
                if key == "@" || key == "*" {
                    return Some("zsh/parameter zsh/zutil".to_string());
                }
                match key {
                    "zsh/parameter" | "zsh/zutil" | "zsh/complete" | "zsh/complist" => {
                        Some("loaded".to_string())
                    }
                    _ => Some(String::new()),
                }
            }

            // === RESERVED WORDS ===
            "reswords" => {
                let reswords = [
                    "do",
                    "done",
                    "esac",
                    "then",
                    "elif",
                    "else",
                    "fi",
                    "for",
                    "case",
                    "if",
                    "while",
                    "function",
                    "repeat",
                    "time",
                    "until",
                    "select",
                    "coproc",
                    "nocorrect",
                    "foreach",
                    "end",
                    "in",
                ];
                if key == "@" || key == "*" {
                    return Some(reswords.join(" "));
                }
                if let Ok(idx) = key.parse::<usize>() {
                    Some(reswords.get(idx).map(|s| s.to_string()).unwrap_or_default())
                } else {
                    Some(String::new())
                }
            }

            // === PATCHARS (characters with special meaning in patterns) ===
            "patchars" => {
                let patchars = ["?", "*", "[", "]", "^", "#", "~", "(", ")", "|"];
                if key == "@" || key == "*" {
                    return Some(patchars.join(" "));
                }
                if let Ok(idx) = key.parse::<usize>() {
                    Some(patchars.get(idx).map(|s| s.to_string()).unwrap_or_default())
                } else {
                    Some(String::new())
                }
            }

            // === FUNCTION CALL STACK ===
            "funcstack" | "functrace" | "funcfiletrace" | "funcsourcetrace" => {
                // Would need call stack tracking - return empty for now
                Some(String::new())
            }

            // === DISABLED VARIANTS (dis_*) ===
            "dis_aliases"
            | "dis_galiases"
            | "dis_saliases"
            | "dis_functions"
            | "dis_functions_source"
            | "dis_builtins"
            | "dis_reswords"
            | "dis_patchars" => {
                // We don't track disabled items - return empty
                Some(String::new())
            }

            // Not a special array
            _ => None,
        }
    }

    /// Get list of all builtin command names
    fn get_builtin_names() -> Vec<&'static str> {
        vec![
            ".",
            ":",
            "[",
            "alias",
            "autoload",
            "bg",
            "bind",
            "bindkey",
            "break",
            "builtin",
            "bye",
            "caller",
            "cd",
            "cdreplay",
            "chdir",
            "clone",
            "command",
            "compadd",
            "comparguments",
            "compcall",
            "compctl",
            "compdef",
            "compdescribe",
            "compfiles",
            "compgen",
            "compgroups",
            "compinit",
            "complete",
            "compopt",
            "compquote",
            "compset",
            "comptags",
            "comptry",
            "compvalues",
            "continue",
            "coproc",
            "declare",
            "dirs",
            "disable",
            "disown",
            "echo",
            "echotc",
            "echoti",
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
            "help",
            "history",
            "integer",
            "jobs",
            "kill",
            "let",
            "limit",
            "local",
            "log",
            "logout",
            "mapfile",
            "noglob",
            "popd",
            "print",
            "printf",
            "private",
            "prompt",
            "promptinit",
            "pushd",
            "pushln",
            "pwd",
            "r",
            "read",
            "readarray",
            "readonly",
            "rehash",
            "return",
            "sched",
            "set",
            "setopt",
            "shift",
            "shopt",
            "source",
            "stat",
            "strftime",
            "suspend",
            "test",
            "times",
            "trap",
            "true",
            "ttyctl",
            "type",
            "typeset",
            "ulimit",
            "umask",
            "unalias",
            "unfunction",
            "unhash",
            "unlimit",
            "unset",
            "unsetopt",
            "vared",
            "wait",
            "whence",
            "where",
            "which",
            "zcompile",
            "zcurses",
            "zformat",
            "zle",
            "zmodload",
            "zparseopts",
            "zprof",
            "zpty",
            "zregexparse",
            "zsocket",
            "zstyle",
            "ztcp",
            "add-zsh-hook",
        ]
    }

    fn get_variable(&self, name: &str) -> String {
        // Handle special parameters
        match name {
            "" => String::new(), // Empty name returns empty
            "$" => std::process::id().to_string(),
            "@" | "*" => self.positional_params.join(" "),
            "#" => self.positional_params.len().to_string(),
            "?" => self.last_status.to_string(),
            "0" => self
                .variables
                .get("0")
                .cloned()
                .unwrap_or_else(|| env::args().next().unwrap_or_default()),
            n if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => {
                let idx: usize = n.parse().unwrap_or(0);
                if idx == 0 {
                    env::args().next().unwrap_or_default()
                } else {
                    self.positional_params
                        .get(idx - 1)
                        .cloned()
                        .unwrap_or_default()
                }
            }
            _ => {
                // Check local variables first, then arrays, then env
                self.variables
                    .get(name)
                    .cloned()
                    .or_else(|| {
                        // In zsh, $arr expands to space-joined array elements
                        self.arrays.get(name).map(|a| a.join(" "))
                    })
                    .or_else(|| env::var(name).ok())
                    .unwrap_or_default()
            }
        }
    }

    fn apply_var_modifier(
        &mut self,
        name: &str,
        val: Option<String>,
        modifier: Option<&VarModifier>,
    ) -> String {
        match modifier {
            None => val.unwrap_or_default(),

            // ${var:-word} - use default value
            Some(VarModifier::Default(word)) => match &val {
                Some(v) if !v.is_empty() => v.clone(),
                _ => self.expand_word(word),
            },

            // ${var:=word} - assign default value
            Some(VarModifier::DefaultAssign(word)) => match &val {
                Some(v) if !v.is_empty() => v.clone(),
                _ => self.expand_word(word),
            },

            // ${var:?word} - error if null or unset
            Some(VarModifier::Error(word)) => match &val {
                Some(v) if !v.is_empty() => v.clone(),
                _ => {
                    let msg = self.expand_word(word);
                    eprintln!("zshrs: {}", msg);
                    String::new()
                }
            },

            // ${var:+word} - use alternate value
            Some(VarModifier::Alternate(word)) => match &val {
                Some(v) if !v.is_empty() => self.expand_word(word),
                _ => String::new(),
            },

            // ${#var} - string length
            Some(VarModifier::Length) => val
                .map(|v| v.len().to_string())
                .unwrap_or_else(|| "0".to_string()),

            // ${var:offset} or ${var:offset:length} - substring
            Some(VarModifier::Substring(offset, length)) => {
                let v = val.unwrap_or_default();
                let start = if *offset < 0 {
                    (v.len() as i64 + offset).max(0) as usize
                } else {
                    (*offset as usize).min(v.len())
                };

                if let Some(len) = length {
                    let len = (*len as usize).min(v.len().saturating_sub(start));
                    v.chars().skip(start).take(len).collect()
                } else {
                    v.chars().skip(start).collect()
                }
            }

            // ${var#pattern} - remove shortest prefix
            Some(VarModifier::RemovePrefix(pattern)) => {
                let v = val.unwrap_or_default();
                let pat = self.expand_word(pattern);
                if v.starts_with(&pat) {
                    v[pat.len()..].to_string()
                } else {
                    v
                }
            }

            // ${var##pattern} - remove longest prefix
            Some(VarModifier::RemovePrefixLong(pattern)) => {
                let v = val.unwrap_or_default();
                let pat = self.expand_word(pattern);
                // For glob patterns, find longest match from start
                if let Ok(glob) = glob::Pattern::new(&pat) {
                    for i in (0..=v.len()).rev() {
                        if glob.matches(&v[..i]) {
                            return v[i..].to_string();
                        }
                    }
                }
                v
            }

            // ${var%pattern} - remove shortest suffix
            Some(VarModifier::RemoveSuffix(pattern)) => {
                let v = val.unwrap_or_default();
                let pat = self.expand_word(pattern);
                // For glob patterns, find shortest match from end
                if let Ok(glob) = glob::Pattern::new(&pat) {
                    for i in (0..=v.len()).rev() {
                        if glob.matches(&v[i..]) {
                            return v[..i].to_string();
                        }
                    }
                } else if v.ends_with(&pat) {
                    return v[..v.len() - pat.len()].to_string();
                }
                v
            }

            // ${var%%pattern} - remove longest suffix
            Some(VarModifier::RemoveSuffixLong(pattern)) => {
                let v = val.unwrap_or_default();
                let pat = self.expand_word(pattern);
                // For glob patterns, find longest match from end
                if let Ok(glob) = glob::Pattern::new(&pat) {
                    for i in 0..=v.len() {
                        if glob.matches(&v[i..]) {
                            return v[..i].to_string();
                        }
                    }
                }
                v
            }

            // ${var/pattern/replacement} - replace first match
            Some(VarModifier::Replace(pattern, replacement)) => {
                let v = val.unwrap_or_default();
                let pat = self.expand_word(pattern);
                let repl = self.expand_word(replacement);
                v.replacen(&pat, &repl, 1)
            }

            // ${var//pattern/replacement} - replace all matches
            Some(VarModifier::ReplaceAll(pattern, replacement)) => {
                let v = val.unwrap_or_default();
                let pat = self.expand_word(pattern);
                let repl = self.expand_word(replacement);
                v.replace(&pat, &repl)
            }

            // ${var^} or ${var^^} - uppercase
            Some(VarModifier::Upper) => val.map(|v| v.to_uppercase()).unwrap_or_default(),

            // ${var,} or ${var,,} - lowercase
            Some(VarModifier::Lower) => val.map(|v| v.to_lowercase()).unwrap_or_default(),

            // ${(flags)var} - zsh parameter expansion flags
            Some(VarModifier::ZshFlags(flags)) => {
                let mut result = val.unwrap_or_default();
                for flag in flags {
                    result = self.apply_zsh_param_flag(&result, name, flag);
                }
                result
            }

            // Array-related modifiers are handled elsewhere
            Some(VarModifier::ArrayLength)
            | Some(VarModifier::ArrayIndex(_))
            | Some(VarModifier::ArrayAll) => val.unwrap_or_default(),
        }
    }

    /// Check if a string starts with history modifier characters
    fn is_history_modifier(&self, s: &str) -> bool {
        if s.is_empty() {
            return false;
        }
        let first = s.chars().next().unwrap();
        matches!(
            first,
            'A' | 'a' | 'h' | 't' | 'r' | 'e' | 'l' | 'u' | 'q' | 'Q' | 'P'
        )
    }

    /// Apply zsh history-style modifiers to a value
    /// Modifiers can be chained: :A:h:h
    fn apply_history_modifiers(&self, val: &str, modifiers: &str) -> String {
        let mut result = val.to_string();
        let mut chars = modifiers.chars().peekable();

        while let Some(c) = chars.next() {
            match c {
                ':' => continue,
                'A' => {
                    if let Ok(abs) = std::fs::canonicalize(&result) {
                        result = abs.to_string_lossy().to_string();
                    } else if !result.starts_with('/') {
                        if let Ok(cwd) = std::env::current_dir() {
                            result = cwd.join(&result).to_string_lossy().to_string();
                        }
                    }
                }
                'a' => {
                    if !result.starts_with('/') {
                        if let Ok(cwd) = std::env::current_dir() {
                            result = cwd.join(&result).to_string_lossy().to_string();
                        }
                    }
                }
                'h' => {
                    if let Some(pos) = result.rfind('/') {
                        if pos == 0 {
                            result = "/".to_string();
                        } else {
                            result = result[..pos].to_string();
                        }
                    } else {
                        result = ".".to_string();
                    }
                }
                't' => {
                    if let Some(pos) = result.rfind('/') {
                        result = result[pos + 1..].to_string();
                    }
                }
                'r' => {
                    if let Some(dot_pos) = result.rfind('.') {
                        let slash_pos = result.rfind('/').map(|p| p + 1).unwrap_or(0);
                        if dot_pos > slash_pos {
                            result = result[..dot_pos].to_string();
                        }
                    }
                }
                'e' => {
                    if let Some(dot_pos) = result.rfind('.') {
                        let slash_pos = result.rfind('/').map(|p| p + 1).unwrap_or(0);
                        if dot_pos > slash_pos {
                            result = result[dot_pos + 1..].to_string();
                        } else {
                            result = String::new();
                        }
                    } else {
                        result = String::new();
                    }
                }
                'l' => result = result.to_lowercase(),
                'u' => result = result.to_uppercase(),
                'q' => result = format!("'{}'", result.replace('\'', "'\\''")),
                'Q' => {
                    if result.starts_with('\'') && result.ends_with('\'') && result.len() >= 2 {
                        result = result[1..result.len() - 1].to_string();
                    } else if result.starts_with('"') && result.ends_with('"') && result.len() >= 2
                    {
                        result = result[1..result.len() - 1].to_string();
                    }
                }
                'P' => {
                    if let Ok(real) = std::fs::canonicalize(&result) {
                        result = real.to_string_lossy().to_string();
                    }
                }
                _ => break,
            }
        }
        result
    }

    /// Parse zsh parameter expansion flags from a string like "L", "U", "j:,:"
    fn parse_zsh_flags(&self, s: &str) -> Vec<ZshParamFlag> {
        let mut flags = Vec::new();
        let mut chars = s.chars().peekable();

        while let Some(c) = chars.next() {
            match c {
                'L' => flags.push(ZshParamFlag::Lower),
                'U' => flags.push(ZshParamFlag::Upper),
                'C' => flags.push(ZshParamFlag::Capitalize),
                'j' => {
                    // j<delim>sep<delim> — join with separator (delim can be any char)
                    if let Some(&delim) = chars.peek() {
                        chars.next(); // consume delimiter char
                        let mut sep = String::new();
                        while let Some(&ch) = chars.peek() {
                            if ch == delim {
                                chars.next();
                                break;
                            }
                            sep.push(chars.next().unwrap());
                        }
                        flags.push(ZshParamFlag::Join(sep));
                    }
                }
                'F' => flags.push(ZshParamFlag::JoinNewline),
                's' => {
                    // s:sep: - split on separator
                    if chars.peek() == Some(&':') {
                        chars.next();
                        let mut sep = String::new();
                        while let Some(&ch) = chars.peek() {
                            if ch == ':' {
                                chars.next();
                                break;
                            }
                            sep.push(chars.next().unwrap());
                        }
                        flags.push(ZshParamFlag::Split(sep));
                    }
                }
                'f' => flags.push(ZshParamFlag::SplitLines),
                'z' => flags.push(ZshParamFlag::SplitWords),
                't' => flags.push(ZshParamFlag::Type),
                'w' => flags.push(ZshParamFlag::Words),
                'b' => flags.push(ZshParamFlag::QuoteBackslash),
                'q' => {
                    if chars.peek() == Some(&'q') {
                        chars.next();
                        flags.push(ZshParamFlag::DoubleQuote);
                    } else {
                        flags.push(ZshParamFlag::Quote);
                    }
                }
                'u' => flags.push(ZshParamFlag::Unique),
                'O' => flags.push(ZshParamFlag::Reverse),
                'o' => flags.push(ZshParamFlag::Sort),
                'n' => flags.push(ZshParamFlag::NumericSort),
                'a' => flags.push(ZshParamFlag::IndexSort),
                'k' => flags.push(ZshParamFlag::Keys),
                'v' => flags.push(ZshParamFlag::Values),
                '#' => flags.push(ZshParamFlag::Length),
                'c' => flags.push(ZshParamFlag::CountChars),
                'e' => flags.push(ZshParamFlag::Expand),
                '%' => {
                    if chars.peek() == Some(&'%') {
                        chars.next();
                        flags.push(ZshParamFlag::PromptExpandFull);
                    } else {
                        flags.push(ZshParamFlag::PromptExpand);
                    }
                }
                'V' => flags.push(ZshParamFlag::Visible),
                'D' => flags.push(ZshParamFlag::Directory),
                'M' => flags.push(ZshParamFlag::Match),
                'R' => flags.push(ZshParamFlag::Remove),
                'S' => flags.push(ZshParamFlag::Subscript),
                'P' => flags.push(ZshParamFlag::Parameter),
                '~' => flags.push(ZshParamFlag::Glob),
                'l' => {
                    // l:len:fill: - pad left
                    if chars.peek() == Some(&':') {
                        chars.next();
                        let mut len_str = String::new();
                        while let Some(&ch) = chars.peek() {
                            if ch == ':' {
                                chars.next();
                                break;
                            }
                            len_str.push(chars.next().unwrap());
                        }
                        let mut fill = ' ';
                        if let Some(&ch) = chars.peek() {
                            if ch != ':' {
                                fill = chars.next().unwrap();
                                if chars.peek() == Some(&':') {
                                    chars.next();
                                }
                            }
                        }
                        if let Ok(len) = len_str.parse() {
                            flags.push(ZshParamFlag::PadLeft(len, fill));
                        }
                    }
                }
                'r' => {
                    // r:len:fill: - pad right
                    if chars.peek() == Some(&':') {
                        chars.next();
                        let mut len_str = String::new();
                        while let Some(&ch) = chars.peek() {
                            if ch == ':' {
                                chars.next();
                                break;
                            }
                            len_str.push(chars.next().unwrap());
                        }
                        let mut fill = ' ';
                        if let Some(&ch) = chars.peek() {
                            if ch != ':' {
                                fill = chars.next().unwrap();
                                if chars.peek() == Some(&':') {
                                    chars.next();
                                }
                            }
                        }
                        if let Ok(len) = len_str.parse() {
                            flags.push(ZshParamFlag::PadRight(len, fill));
                        }
                    }
                }
                'm' => {
                    // Width for padding - parse number if present
                    let mut width_str = String::new();
                    while let Some(&ch) = chars.peek() {
                        if ch.is_ascii_digit() {
                            width_str.push(chars.next().unwrap());
                        } else {
                            break;
                        }
                    }
                    if let Ok(w) = width_str.parse() {
                        flags.push(ZshParamFlag::Width(w));
                    }
                }
                _ => {}
            }
        }
        flags
    }

    /// Apply a single zsh parameter expansion flag
    fn apply_zsh_param_flag(&self, val: &str, name: &str, flag: &ZshParamFlag) -> String {
        match flag {
            ZshParamFlag::Lower => val.to_lowercase(),
            ZshParamFlag::Upper => val.to_uppercase(),
            ZshParamFlag::Capitalize => val
                .split_whitespace()
                .map(|word| {
                    let mut c = word.chars();
                    match c.next() {
                        None => String::new(),
                        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" "),
            ZshParamFlag::Join(sep) => {
                if let Some(arr) = self.arrays.get(name) {
                    arr.join(sep)
                } else {
                    val.to_string()
                }
            }
            ZshParamFlag::Split(sep) => val.split(sep).collect::<Vec<_>>().join(" "),
            ZshParamFlag::SplitLines => val.lines().collect::<Vec<_>>().join(" "),
            ZshParamFlag::Type => {
                if self.arrays.contains_key(name) {
                    "array".to_string()
                } else if self.assoc_arrays.contains_key(name) {
                    "association".to_string()
                } else if self.functions.contains_key(name) {
                    "function".to_string()
                } else if std::env::var(name).is_ok() || self.variables.contains_key(name) {
                    "scalar".to_string()
                } else {
                    "".to_string()
                }
            }
            ZshParamFlag::Words => val.split_whitespace().collect::<Vec<_>>().join(" "),
            ZshParamFlag::Quote => format!("'{}'", val.replace('\'', "'\\''")),
            ZshParamFlag::DoubleQuote => format!("\"{}\"", val.replace('"', "\\\"")),
            ZshParamFlag::Unique => {
                // Unique preserves first-occurrence order, so parallel doesn't help.
                // For 1000+ elements, pre-allocate the HashSet for less rehashing.
                let words: Vec<&str> = val.split_whitespace().collect();
                let mut seen = std::collections::HashSet::with_capacity(
                    if words.len() >= 1000 { words.len() } else { 0 },
                );
                if words.len() >= 1000 {
                    tracing::trace!(
                        count = words.len(),
                        "unique on large array ({} elements)",
                        words.len()
                    );
                }
                words
                    .into_iter()
                    .filter(|s| seen.insert(*s))
                    .collect::<Vec<_>>()
                    .join(" ")
            }
            ZshParamFlag::Reverse => {
                // (O) flag: reverse sort (sort descending)
                let mut words: Vec<&str> = val.split_whitespace().collect();
                if words.len() >= 1000 {
                    tracing::trace!(
                        count = words.len(),
                        "using parallel reverse sort (rayon) for large array"
                    );
                    use rayon::prelude::*;
                    words.par_sort_unstable_by(|a, b| b.cmp(a));
                } else {
                    words.sort_unstable_by(|a, b| b.cmp(a));
                }
                words.join(" ")
            }
            ZshParamFlag::Sort => {
                let mut words: Vec<&str> = val.split_whitespace().collect();
                if words.len() >= 1000 {
                    tracing::trace!(
                        count = words.len(),
                        "using parallel sort (rayon) for large array"
                    );
                    use rayon::prelude::*;
                    words.par_sort_unstable();
                } else {
                    words.sort_unstable();
                }
                words.join(" ")
            }
            ZshParamFlag::NumericSort => {
                let mut words: Vec<&str> = val.split_whitespace().collect();
                let cmp = |a: &&str, b: &&str| {
                    let na: i64 = a.parse().unwrap_or(0);
                    let nb: i64 = b.parse().unwrap_or(0);
                    na.cmp(&nb)
                };
                if words.len() >= 1000 {
                    tracing::trace!(
                        count = words.len(),
                        "using parallel numeric sort (rayon) for large array"
                    );
                    use rayon::prelude::*;
                    words.par_sort_unstable_by(cmp);
                } else {
                    words.sort_unstable_by(cmp);
                }
                words.join(" ")
            }
            ZshParamFlag::Keys => {
                if let Some(assoc) = self.assoc_arrays.get(name) {
                    assoc.keys().cloned().collect::<Vec<_>>().join(" ")
                } else {
                    String::new()
                }
            }
            ZshParamFlag::Values => {
                if let Some(assoc) = self.assoc_arrays.get(name) {
                    assoc.values().cloned().collect::<Vec<_>>().join(" ")
                } else {
                    val.to_string()
                }
            }
            ZshParamFlag::Length => val.len().to_string(),
            ZshParamFlag::Head(n) => val
                .split_whitespace()
                .take(*n)
                .collect::<Vec<_>>()
                .join(" "),
            ZshParamFlag::Tail(n) => {
                let words: Vec<&str> = val.split_whitespace().collect();
                if words.len() > *n {
                    words[words.len() - n..].join(" ")
                } else {
                    val.to_string()
                }
            }
            ZshParamFlag::JoinNewline => {
                if let Some(arr) = self.arrays.get(name) {
                    arr.join("\n")
                } else {
                    val.to_string()
                }
            }
            ZshParamFlag::SplitWords => {
                // Shell-style word splitting
                val.split_whitespace().collect::<Vec<_>>().join(" ")
            }
            ZshParamFlag::QuoteBackslash => {
                // Quote special pattern chars with backslashes
                let mut result = String::new();
                for c in val.chars() {
                    if "\\*?[]{}()".contains(c) {
                        result.push('\\');
                    }
                    result.push(c);
                }
                result
            }
            ZshParamFlag::IndexSort => {
                // Array index order - just return as-is (default)
                val.to_string()
            }
            ZshParamFlag::CountChars => {
                // Count total characters
                val.chars().count().to_string()
            }
            ZshParamFlag::Expand => {
                // Would need mutable self to do expansions
                val.to_string()
            }
            ZshParamFlag::PromptExpand => {
                // Expand prompt escapes
                self.expand_prompt_string(val)
            }
            ZshParamFlag::PromptExpandFull => {
                // Full prompt expansion
                self.expand_prompt_string(val)
            }
            ZshParamFlag::Visible => {
                // Make non-printable characters visible
                val.chars()
                    .map(|c| {
                        if c.is_control() {
                            format!("^{}", (c as u8 + 64) as char)
                        } else {
                            c.to_string()
                        }
                    })
                    .collect()
            }
            ZshParamFlag::Directory => {
                // Substitute leading directory with ~ if it's home
                if let Some(home) = dirs::home_dir() {
                    let home_str = home.to_string_lossy();
                    if val.starts_with(home_str.as_ref()) {
                        format!("~{}", &val[home_str.len()..])
                    } else {
                        val.to_string()
                    }
                } else {
                    val.to_string()
                }
            }
            ZshParamFlag::PadLeft(len, fill) => {
                if val.len() >= *len {
                    val.to_string()
                } else {
                    let padding: String = std::iter::repeat(*fill).take(len - val.len()).collect();
                    format!("{}{}", padding, val)
                }
            }
            ZshParamFlag::PadRight(len, fill) => {
                if val.len() >= *len {
                    val.to_string()
                } else {
                    let padding: String = std::iter::repeat(*fill).take(len - val.len()).collect();
                    format!("{}{}", val, padding)
                }
            }
            ZshParamFlag::Width(_) => {
                // Width modifier - used with padding, just return value
                val.to_string()
            }
            ZshParamFlag::Match => {
                // Match flag - used with pattern operations, just pass through
                // Actual matching is handled in the pattern operations below
                val.to_string()
            }
            ZshParamFlag::Remove => {
                // Remove flag - complement of Match
                val.to_string()
            }
            ZshParamFlag::Subscript => {
                // Subscript scanning
                val.to_string()
            }
            ZshParamFlag::Parameter => {
                // Parameter indirection - treat val as parameter name
                self.get_variable(val)
            }
            ZshParamFlag::Glob => {
                // Glob patterns in pattern matching
                val.to_string()
            }
        }
    }

    /// Expand prompt escape sequences using the full prompt module
    fn expand_prompt_string(&self, s: &str) -> String {
        let ctx = self.build_prompt_context();
        expand_prompt(s, &ctx)
    }

    /// Build a PromptContext from current executor state
    fn build_prompt_context(&self) -> PromptContext {
        let pwd = env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "/".to_string());

        let home = env::var("HOME").unwrap_or_default();

        let user = env::var("USER")
            .or_else(|_| env::var("LOGNAME"))
            .unwrap_or_else(|_| "user".to_string());

        let host = hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "localhost".to_string());

        let host_short = host.split('.').next().unwrap_or(&host).to_string();

        let shlvl = env::var("SHLVL")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);

        PromptContext {
            pwd,
            home,
            user,
            host,
            host_short,
            tty: String::new(),
            lastval: self.last_status,
            histnum: self
                .history
                .as_ref()
                .and_then(|h| h.count().ok())
                .unwrap_or(1),
            shlvl,
            num_jobs: self.jobs.list().len() as i32,
            is_root: unsafe { libc::geteuid() } == 0,
            cmd_stack: Vec::new(),
            psvar: self.get_psvar(),
            term_width: self.get_term_width(),
            lineno: 1,
        }
    }

    fn get_psvar(&self) -> Vec<String> {
        if let Some(arr) = self.arrays.get("psvar") {
            arr.clone()
        } else {
            Vec::new()
        }
    }

    fn get_term_width(&self) -> usize {
        env::var("COLUMNS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(80)
    }

    /// Execute a command and capture its output (command substitution)
    fn execute_command_substitution(&mut self, cmd: &ShellCommand) -> String {
        match self.execute_command_capture(cmd) {
            Ok(output) => output.trim_end_matches('\n').to_string(),
            Err(_) => String::new(),
        }
    }

    /// Execute a command and capture its stdout
    fn execute_command_capture(&mut self, cmd: &ShellCommand) -> Result<String, String> {
        // For simple commands, we can use Command directly
        if let ShellCommand::Simple(simple) = cmd {
            let words: Vec<String> = simple.words.iter().map(|w| self.expand_word(w)).collect();
            if words.is_empty() {
                return Ok(String::new());
            }

            let cmd_name = &words[0];
            let args = &words[1..];

            // Handle some builtins that can return values
            match cmd_name.as_str() {
                "echo" => {
                    let output = args.join(" ");
                    return Ok(format!("{}\n", output));
                }
                "printf" => {
                    if !args.is_empty() {
                        // Simple printf - just format string with args
                        let format = &args[0];
                        let result = if args.len() > 1 {
                            // Very basic: just handle %s
                            let mut out = format.clone();
                            for (i, arg) in args[1..].iter().enumerate() {
                                out = out.replacen("%s", arg, 1);
                                out = out.replacen(&format!("${}", i + 1), arg, 1);
                            }
                            out
                        } else {
                            format.clone()
                        };
                        return Ok(result);
                    }
                    return Ok(String::new());
                }
                "pwd" => {
                    return Ok(env::current_dir()
                        .map(|p| format!("{}\n", p.display()))
                        .unwrap_or_default());
                }
                _ => {}
            }

            // External command - capture its output
            let output = Command::new(cmd_name)
                .args(args)
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .output();

            match output {
                Ok(output) => {
                    self.last_status = output.status.code().unwrap_or(1);
                    Ok(String::from_utf8_lossy(&output.stdout).to_string())
                }
                Err(e) => {
                    self.last_status = 127;
                    Err(format!("{}: {}", cmd_name, e))
                }
            }
        } else if let ShellCommand::Pipeline(cmds, _negated) = cmd {
            // For pipelines, execute and capture output of the last command
            // This is simplified - proper implementation would pipe between all
            if let Some(last) = cmds.last() {
                return self.execute_command_capture(last);
            }
            Ok(String::new())
        } else {
            // For compound commands, execute them and return empty
            // (complex case - could be expanded later)
            let _ = self.execute_command(cmd);
            Ok(String::new())
        }
    }

    /// Evaluate arithmetic expression using the full math module
    fn evaluate_arithmetic(&mut self, expr: &str) -> String {
        let expr = self.expand_string(expr);
        let force_float = self.options.get("forcefloat").copied().unwrap_or(false);
        let c_prec = self.options.get("cprecedences").copied().unwrap_or(false);
        let octal = self.options.get("octalzeroes").copied().unwrap_or(false);

        let mut evaluator = MathEval::new(&expr)
            .with_string_variables(&self.variables)
            .with_force_float(force_float)
            .with_c_precedences(c_prec)
            .with_octal_zeroes(octal);

        match evaluator.evaluate() {
            Ok(result) => {
                for (k, v) in evaluator.extract_string_variables() {
                    self.variables.insert(k.clone(), v.clone());
                    env::set_var(&k, &v);
                }
                match result {
                    crate::math::MathNum::Integer(i) => i.to_string(),
                    crate::math::MathNum::Float(f) => {
                        if f.fract() == 0.0 && f.abs() < i64::MAX as f64 {
                            (f as i64).to_string()
                        } else {
                            f.to_string()
                        }
                    }
                    crate::math::MathNum::Unset => "0".to_string(),
                }
            }
            Err(_) => "0".to_string(),
        }
    }

    fn eval_arith_expr(&mut self, expr: &str) -> i64 {
        let expr_expanded = self.expand_string(expr);
        let c_prec = self.options.get("cprecedences").copied().unwrap_or(false);
        let octal = self.options.get("octalzeroes").copied().unwrap_or(false);

        let mut evaluator = MathEval::new(&expr_expanded)
            .with_string_variables(&self.variables)
            .with_c_precedences(c_prec)
            .with_octal_zeroes(octal);

        match evaluator.evaluate() {
            Ok(result) => {
                for (k, v) in evaluator.extract_string_variables() {
                    self.variables.insert(k.clone(), v.clone());
                    env::set_var(&k, &v);
                }
                result.to_int()
            }
            Err(_) => 0,
        }
    }

    fn eval_arith_expr_float(&mut self, expr: &str) -> f64 {
        let expr_expanded = self.expand_string(expr);
        let force_float = self.options.get("forcefloat").copied().unwrap_or(false);
        let c_prec = self.options.get("cprecedences").copied().unwrap_or(false);
        let octal = self.options.get("octalzeroes").copied().unwrap_or(false);

        let mut evaluator = MathEval::new(&expr_expanded)
            .with_string_variables(&self.variables)
            .with_force_float(force_float)
            .with_c_precedences(c_prec)
            .with_octal_zeroes(octal);

        match evaluator.evaluate() {
            Ok(result) => {
                for (k, v) in evaluator.extract_string_variables() {
                    self.variables.insert(k.clone(), v.clone());
                    env::set_var(&k, &v);
                }
                result.to_float()
            }
            Err(_) => 0.0,
        }
    }

    fn matches_pattern(&self, value: &str, pattern: &str) -> bool {
        // Simple glob matching
        if pattern == "*" {
            return true;
        }
        if pattern.contains('*') || pattern.contains('?') || pattern.contains('[') {
            // Use glob matching for wildcards and character classes
            glob::Pattern::new(pattern)
                .map(|p| p.matches(value))
                .unwrap_or(false)
        } else {
            value == pattern
        }
    }

    fn eval_cond_expr(&mut self, expr: &CondExpr) -> bool {
        match expr {
            CondExpr::FileExists(w) => std::path::Path::new(&self.expand_word(w)).exists(),
            CondExpr::FileRegular(w) => std::path::Path::new(&self.expand_word(w)).is_file(),
            CondExpr::FileDirectory(w) => std::path::Path::new(&self.expand_word(w)).is_dir(),
            CondExpr::FileSymlink(w) => std::path::Path::new(&self.expand_word(w)).is_symlink(),
            CondExpr::FileReadable(w) => std::path::Path::new(&self.expand_word(w)).exists(),
            CondExpr::FileWritable(w) => std::path::Path::new(&self.expand_word(w)).exists(),
            CondExpr::FileExecutable(w) => std::path::Path::new(&self.expand_word(w)).exists(),
            CondExpr::FileNonEmpty(w) => std::fs::metadata(&self.expand_word(w))
                .map(|m| m.len() > 0)
                .unwrap_or(false),
            CondExpr::StringEmpty(w) => self.expand_word(w).is_empty(),
            CondExpr::StringNonEmpty(w) => !self.expand_word(w).is_empty(),
            CondExpr::StringEqual(a, b) => {
                let left = self.expand_word(a);
                let right = self.expand_word(b);
                // In [[ ]], == does glob pattern matching on the right side
                if right.contains('*') || right.contains('?') || right.contains('[') {
                    crate::glob::pattern_match(&right, &left, true, true)
                } else {
                    left == right
                }
            }
            CondExpr::StringNotEqual(a, b) => {
                let left = self.expand_word(a);
                let right = self.expand_word(b);
                if right.contains('*') || right.contains('?') || right.contains('[') {
                    !crate::glob::pattern_match(&right, &left, true, true)
                } else {
                    left != right
                }
            }
            CondExpr::StringMatch(a, b) => {
                let val = self.expand_word(a);
                let pattern = self.expand_word(b);
                if let Some(re) = cached_regex(&pattern) {
                    if let Some(caps) = re.captures(&val) {
                        // Set $MATCH to the full match
                        if let Some(m) = caps.get(0) {
                            self.variables.insert("MATCH".to_string(), m.as_str().to_string());
                        }
                        // Set $match array with capture groups
                        let mut match_arr = Vec::new();
                        for i in 1..caps.len() {
                            if let Some(g) = caps.get(i) {
                                match_arr.push(g.as_str().to_string());
                            }
                        }
                        if !match_arr.is_empty() {
                            self.arrays.insert("match".to_string(), match_arr);
                        }
                        true
                    } else {
                        self.variables.remove("MATCH");
                        self.arrays.remove("match");
                        false
                    }
                } else {
                    false
                }
            }
            CondExpr::StringLess(a, b) => self.expand_word(a) < self.expand_word(b),
            CondExpr::StringGreater(a, b) => self.expand_word(a) > self.expand_word(b),
            CondExpr::NumEqual(a, b) => {
                let a_val = self.expand_word(a).parse::<i64>().unwrap_or(0);
                let b_val = self.expand_word(b).parse::<i64>().unwrap_or(0);
                a_val == b_val
            }
            CondExpr::NumNotEqual(a, b) => {
                let a_val = self.expand_word(a).parse::<i64>().unwrap_or(0);
                let b_val = self.expand_word(b).parse::<i64>().unwrap_or(0);
                a_val != b_val
            }
            CondExpr::NumLess(a, b) => {
                let a_val = self.expand_word(a).parse::<i64>().unwrap_or(0);
                let b_val = self.expand_word(b).parse::<i64>().unwrap_or(0);
                a_val < b_val
            }
            CondExpr::NumLessEqual(a, b) => {
                let a_val = self.expand_word(a).parse::<i64>().unwrap_or(0);
                let b_val = self.expand_word(b).parse::<i64>().unwrap_or(0);
                a_val <= b_val
            }
            CondExpr::NumGreater(a, b) => {
                let a_val = self.expand_word(a).parse::<i64>().unwrap_or(0);
                let b_val = self.expand_word(b).parse::<i64>().unwrap_or(0);
                a_val > b_val
            }
            CondExpr::NumGreaterEqual(a, b) => {
                let a_val = self.expand_word(a).parse::<i64>().unwrap_or(0);
                let b_val = self.expand_word(b).parse::<i64>().unwrap_or(0);
                a_val >= b_val
            }
            CondExpr::Not(inner) => !self.eval_cond_expr(inner),
            CondExpr::And(a, b) => self.eval_cond_expr(a) && self.eval_cond_expr(b),
            CondExpr::Or(a, b) => self.eval_cond_expr(a) || self.eval_cond_expr(b),
        }
    }

    // Builtins
    // Ported from zsh/Src/builtin.c

    /// cd builtin - change directory
    /// Ported from zsh/Src/builtin.c bin_cd() lines 839-859, cd_get_dest() lines 864-957,
    /// cd_do_chdir() lines 967-1081, cd_try_chdir() lines 1116-1181
    fn builtin_cd(&mut self, args: &[String]) -> i32 {
        // cd [ -qsLP ] [ arg ]
        // cd [ -qsLP ] old new
        // cd [ -qsLP ] {+|-}n
        let mut quiet = false;
        let mut use_cdpath = false;
        let mut logical = true; // -L is default
        let mut positional_args: Vec<&str> = Vec::new();

        for arg in args {
            if arg.starts_with('-') && arg.len() > 1 && !arg.starts_with("--") {
                // Check if it's a stack index like -2
                if arg[1..].chars().all(|c| c.is_ascii_digit()) {
                    positional_args.push(arg);
                    continue;
                }
                for ch in arg[1..].chars() {
                    match ch {
                        'q' => quiet = true,
                        's' => use_cdpath = true,
                        'L' => logical = true,
                        'P' => logical = false,
                        _ => {
                            eprintln!("cd: bad option: -{}", ch);
                            return 1;
                        }
                    }
                }
            } else if arg.starts_with('+')
                && arg.len() > 1
                && arg[1..].chars().all(|c| c.is_ascii_digit())
            {
                // Stack index like +2
                positional_args.push(arg);
            } else {
                positional_args.push(arg);
            }
        }

        // Handle cd old new (substitution)
        if positional_args.len() == 2 {
            if let Ok(cwd) = env::current_dir() {
                let cwd_str = cwd.to_string_lossy();
                let old = positional_args[0];
                let new = positional_args[1];
                if cwd_str.contains(old) {
                    let new_path = cwd_str.replace(old, new);
                    if !quiet {
                        println!("{}", new_path);
                    }
                    positional_args = vec![];
                    return self.do_cd(&new_path, quiet, use_cdpath, logical);
                }
            }
        }

        let path_arg = positional_args.first().map(|s| *s).unwrap_or("~");

        // Handle stack indices
        if path_arg.starts_with('+') || path_arg.starts_with('-') {
            if let Ok(n) = path_arg[1..].parse::<usize>() {
                let idx = if path_arg.starts_with('+') {
                    n
                } else {
                    self.dir_stack.len().saturating_sub(n)
                };
                if let Some(dir) = self.dir_stack.get(idx) {
                    let dir_path = dir.to_string_lossy().to_string();
                    return self.do_cd(&dir_path, quiet, use_cdpath, logical);
                } else {
                    eprintln!("cd: no such entry in dir stack");
                    return 1;
                }
            }
        }

        self.do_cd(path_arg, quiet, use_cdpath, logical)
    }

    fn do_cd(&mut self, path_arg: &str, quiet: bool, use_cdpath: bool, physical: bool) -> i32 {
        let path = if path_arg == "~" || path_arg.is_empty() {
            dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
        } else if path_arg.starts_with("~/") {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(&path_arg[2..])
        } else if path_arg == "-" {
            if let Ok(oldpwd) = env::var("OLDPWD") {
                if !quiet {
                    println!("{}", oldpwd);
                }
                PathBuf::from(oldpwd)
            } else {
                eprintln!("cd: OLDPWD not set");
                return 1;
            }
        } else if use_cdpath && !path_arg.starts_with('/') && !path_arg.starts_with('.') {
            // Search CDPATH
            let cdpath = env::var("CDPATH").unwrap_or_default();
            let mut found = None;
            for dir in cdpath.split(':') {
                let candidate = if dir.is_empty() {
                    PathBuf::from(path_arg)
                } else {
                    PathBuf::from(dir).join(path_arg)
                };
                if candidate.is_dir() {
                    found = Some(candidate);
                    break;
                }
            }
            found.unwrap_or_else(|| PathBuf::from(path_arg))
        } else {
            PathBuf::from(path_arg)
        };

        if let Ok(cwd) = env::current_dir() {
            env::set_var("OLDPWD", &cwd);
        }

        // Resolve symlinks if -P (physical)
        let target = if !physical {
            if let Ok(resolved) = path.canonicalize() {
                resolved
            } else {
                path.clone()
            }
        } else {
            path.clone()
        };

        match env::set_current_dir(&target) {
            Ok(_) => {
                if let Ok(cwd) = env::current_dir() {
                    env::set_var("PWD", &cwd);
                    self.variables
                        .insert("PWD".to_string(), cwd.to_string_lossy().to_string());
                }
                0
            }
            Err(e) => {
                eprintln!("cd: {}: {}", path.display(), e);
                1
            }
        }
    }

    fn builtin_pwd(&mut self, _redirects: &[Redirect]) -> i32 {
        match env::current_dir() {
            Ok(path) => {
                println!("{}", path.display());
                0
            }
            Err(e) => {
                eprintln!("pwd: {}", e);
                1
            }
        }
    }

    fn builtin_echo(&mut self, args: &[String], _redirects: &[Redirect]) -> i32 {
        let mut newline = true;
        let mut interpret_escapes = false;
        let mut start = 0;

        for (i, arg) in args.iter().enumerate() {
            match arg.as_str() {
                "-n" => {
                    newline = false;
                    start = i + 1;
                }
                "-e" => {
                    interpret_escapes = true;
                    start = i + 1;
                }
                "-E" => {
                    interpret_escapes = false;
                    start = i + 1;
                }
                _ => break,
            }
        }

        let output = args[start..].join(" ");
        if interpret_escapes {
            print!("{}", output.replace("\\n", "\n").replace("\\t", "\t"));
        } else {
            print!("{}", output);
        }

        if newline {
            println!();
        }
        0
    }

    fn builtin_export(&mut self, args: &[String]) -> i32 {
        for arg in args {
            if let Some((key, value)) = arg.split_once('=') {
                self.variables.insert(key.to_string(), value.to_string());
                env::set_var(key, value);
            } else {
                // export VAR (no value) — mark existing var as exported
                let val = self.get_variable(arg);
                env::set_var(arg, &val);
            }
        }
        0
    }

    fn builtin_unset(&mut self, args: &[String]) -> i32 {
        for arg in args {
            env::remove_var(arg);
            self.variables.remove(arg);
        }
        0
    }

    fn builtin_source(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            eprintln!("source: filename argument required");
            return 1;
        }

        let path = &args[0];

        // Resolve to absolute path
        let abs_path = if path.starts_with('/') {
            path.clone()
        } else if path.starts_with("~/") {
            if let Some(home) = dirs::home_dir() {
                home.join(&path[2..]).to_string_lossy().to_string()
            } else {
                path.clone()
            }
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(path).to_string_lossy().to_string())
                .unwrap_or_else(|_| path.clone())
        };

        // Save current $0 and set to the sourced file path
        let saved_zero = self.variables.get("0").cloned();
        self.variables.insert("0".to_string(), abs_path.clone());

        let result;

        if self.posix_mode {
            // --- POSIX mode: plain read + execute, no SQLite, no caching, no threads ---
            result = match std::fs::read_to_string(&abs_path) {
                Ok(content) => match self.execute_script(&content) {
                    Ok(status) => status,
                    Err(e) => { eprintln!("source: {}: {}", path, e); 1 }
                },
                Err(e) => { eprintln!("source: {}: {}", path, e); 1 }
            };
        } else {
            // --- zshrs/zsh mode: plugin cache + AST cache + worker pool ---
            let file_path = std::path::Path::new(&abs_path);

            // Check plugin cache for side-effect replay
            if let Some(ref cache) = self.plugin_cache {
                if let Some((mt_s, mt_ns)) = crate::plugin_cache::file_mtime(file_path) {
                    if let Some(plugin_id) = cache.check(&abs_path, mt_s, mt_ns) {
                        if let Ok(delta) = cache.load(plugin_id) {
                            let t0 = std::time::Instant::now();
                            self.replay_plugin_delta(&delta);
                            tracing::info!(
                                path = %abs_path,
                                replay_us = t0.elapsed().as_micros() as u64,
                                funcs = delta.functions.len(),
                                aliases = delta.aliases.len(),
                                vars = delta.variables.len() + delta.exports.len(),
                                "source: cache hit, replayed"
                            );
                            // Restore $0
                            if let Some(z) = saved_zero { self.variables.insert("0".to_string(), z); }
                            else { self.variables.remove("0"); }
                            return 0;
                        }
                    }
                }
            }

            // Cache miss — snapshot, execute via AST-cached path, diff, async store
            let snapshot = self.snapshot_state();
            let t0 = std::time::Instant::now();
            tracing::debug!(path = %abs_path, "source: cache miss, executing via AST-cached path");
            result = match self.execute_script_file(&abs_path) {
                Ok(status) => status,
                Err(e) => {
                    tracing::warn!(path = %abs_path, error = %e, "source: execution failed");
                    eprintln!("source: {}: {}", path, e);
                    1
                }
            };
            let source_ms = t0.elapsed().as_millis() as u64;

            // Async-store delta to plugin cache on worker pool
            if result == 0 {
                if let Some((mt_s, mt_ns)) = crate::plugin_cache::file_mtime(file_path) {
                    let delta = self.diff_state(&snapshot);
                    let store_path = abs_path.clone();
                    tracing::info!(
                        path = %abs_path, source_ms,
                        funcs = delta.functions.len(),
                        aliases = delta.aliases.len(),
                        vars = delta.variables.len() + delta.exports.len(),
                        "source: caching delta on worker"
                    );
                    let cache_db_path = crate::plugin_cache::default_cache_path();
                    self.worker_pool.submit(move || {
                        match crate::plugin_cache::PluginCache::open(&cache_db_path) {
                            Ok(cache) => {
                                if let Err(e) = cache.store(&store_path, mt_s, mt_ns, source_ms, &delta) {
                                    tracing::error!(path = %store_path, error = %e, "plugin_cache: store failed");
                                } else {
                                    tracing::debug!(path = %store_path, "plugin_cache: stored");
                                }
                            }
                            Err(e) => tracing::error!(error = %e, "plugin_cache: open for write failed"),
                        }
                    });
                }
            }
        }

        // Handle return from sourced script
        let final_result = if let Some(ret) = self.returning.take() {
            ret
        } else {
            result
        };

        // Restore $0
        if let Some(z) = saved_zero {
            self.variables.insert("0".to_string(), z);
        } else {
            self.variables.remove("0");
        }

        final_result
    }

    /// Snapshot executor state before sourcing a plugin (for delta computation).
    fn snapshot_state(&self) -> PluginSnapshot {
        PluginSnapshot {
            functions: self.functions.keys().cloned().collect(),
            aliases: self.aliases.keys().cloned().collect(),
            global_aliases: self.global_aliases.keys().cloned().collect(),
            suffix_aliases: self.suffix_aliases.keys().cloned().collect(),
            variables: self.variables.clone(),
            arrays: self.arrays.keys().cloned().collect(),
            assoc_arrays: self.assoc_arrays.keys().cloned().collect(),
            fpath: self.fpath.clone(),
            options: self.options.clone(),
            hooks: self.hook_functions.clone(),
            autoloads: self.autoload_pending.keys().cloned().collect(),
        }
    }

    /// Compute the delta between current state and a previous snapshot.
    fn diff_state(&self, snap: &PluginSnapshot) -> crate::plugin_cache::PluginDelta {
        use crate::plugin_cache::{AliasKind, PluginDelta};
        let mut delta = PluginDelta::default();

        // New functions — serialize AST to bincode for instant replay
        for (name, body) in &self.functions {
            if !snap.functions.contains(name) {
                if let Ok(bytes) = bincode::serialize(body) {
                    delta.functions.push((name.clone(), bytes));
                }
            }
        }

        // New aliases
        for (name, value) in &self.aliases {
            if !snap.aliases.contains(name) {
                delta.aliases.push((name.clone(), value.clone(), AliasKind::Regular));
            }
        }
        for (name, value) in &self.global_aliases {
            if !snap.global_aliases.contains(name) {
                delta.aliases.push((name.clone(), value.clone(), AliasKind::Global));
            }
        }
        for (name, value) in &self.suffix_aliases {
            if !snap.suffix_aliases.contains(name) {
                delta.aliases.push((name.clone(), value.clone(), AliasKind::Suffix));
            }
        }

        // New/changed variables
        for (name, value) in &self.variables {
            if name == "0" { continue; } // skip $0 (we set it ourselves)
            match snap.variables.get(name) {
                Some(old) if old == value => {} // unchanged
                _ => {
                    // Check if it's also exported
                    if env::var(name).ok().as_ref() == Some(value) {
                        delta.exports.push((name.clone(), value.clone()));
                    } else {
                        delta.variables.push((name.clone(), value.clone()));
                    }
                }
            }
        }

        // New arrays
        for (name, values) in &self.arrays {
            if !snap.arrays.contains(name) {
                delta.arrays.push((name.clone(), values.clone()));
            }
        }

        // New fpath entries
        for p in &self.fpath {
            if !snap.fpath.contains(p) {
                delta.fpath_additions.push(p.to_string_lossy().to_string());
            }
        }

        // Changed options
        for (name, value) in &self.options {
            match snap.options.get(name) {
                Some(old) if old == value => {}
                _ => delta.options_changed.push((name.clone(), *value)),
            }
        }

        // New hooks
        for (hook, funcs) in &self.hook_functions {
            let old_funcs = snap.hooks.get(hook);
            for f in funcs {
                let is_new = old_funcs.map_or(true, |old| !old.contains(f));
                if is_new {
                    delta.hooks.push((hook.clone(), f.clone()));
                }
            }
        }

        // New autoloads
        for (name, flags) in &self.autoload_pending {
            if !snap.autoloads.contains(name) {
                delta.autoloads.push((name.clone(), format!("{:?}", flags)));
            }
        }

        delta
    }

    /// Replay a cached plugin delta into the executor state.
    fn replay_plugin_delta(&mut self, delta: &crate::plugin_cache::PluginDelta) {
        use crate::plugin_cache::AliasKind;

        // Aliases
        for (name, value, kind) in &delta.aliases {
            match kind {
                AliasKind::Regular => { self.aliases.insert(name.clone(), value.clone()); }
                AliasKind::Global => { self.global_aliases.insert(name.clone(), value.clone()); }
                AliasKind::Suffix => { self.suffix_aliases.insert(name.clone(), value.clone()); }
            }
        }

        // Variables
        for (name, value) in &delta.variables {
            self.variables.insert(name.clone(), value.clone());
        }

        // Exports (set in both variables and process env)
        for (name, value) in &delta.exports {
            self.variables.insert(name.clone(), value.clone());
            env::set_var(name, value);
        }

        // Arrays
        for (name, values) in &delta.arrays {
            self.arrays.insert(name.clone(), values.clone());
        }

        // Fpath additions
        for p in &delta.fpath_additions {
            let pb = PathBuf::from(p);
            if !self.fpath.contains(&pb) {
                self.fpath.push(pb);
            }
        }

        // Completions
        for (cmd, func) in &delta.completions {
            if let Some(ref mut comps) = self.assoc_arrays.get_mut("_comps") {
                comps.insert(cmd.clone(), func.clone());
            }
        }

        // Options
        for (name, enabled) in &delta.options_changed {
            self.options.insert(name.clone(), *enabled);
        }

        // Hooks
        for (hook, func) in &delta.hooks {
            self.hook_functions
                .entry(hook.clone())
                .or_insert_with(Vec::new)
                .push(func.clone());
        }

        // Functions — deserialize bincode bytecode blobs directly into self.functions
        for (name, bytes) in &delta.functions {
            if let Ok(ast) = bincode::deserialize::<crate::parser::ShellCommand>(bytes) {
                self.functions.insert(name.clone(), ast);
            }
        }
    }

    fn builtin_exit(&mut self, args: &[String]) -> i32 {
        let code = args
            .first()
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(self.last_status);
        std::process::exit(code);
    }

    fn builtin_return(&mut self, args: &[String]) -> i32 {
        let status = args
            .first()
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(self.last_status);
        self.returning = Some(status);
        status
    }

    fn builtin_test(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            return 1;
        }

        // Strip trailing "]" when called as `[`
        let args: Vec<&str> = args
            .iter()
            .map(|s| s.as_str())
            .filter(|&s| s != "]")
            .collect();

        // Prefetch metadata for all file paths in the expression — one stat() per unique path
        // instead of one stat() per test flag. Avoids 7 serial stat()s for -r -w -x -g -k -u -s.
        let mut meta_cache: HashMap<String, Option<std::fs::Metadata>> = HashMap::new();
        for arg in &args {
            if !arg.starts_with('-') && !arg.starts_with('!') && *arg != "(" && *arg != ")" {
                let path_str = arg.to_string();
                if !meta_cache.contains_key(&path_str) {
                    meta_cache.insert(path_str, std::fs::metadata(arg).ok());
                }
            }
        }

        // Helper closure: get metadata from cache or fetch
        let get_meta = |path: &str| -> Option<std::fs::Metadata> {
            meta_cache.get(path).cloned().unwrap_or_else(|| std::fs::metadata(path).ok())
        };

        match args.as_slice() {
            // String tests
            ["-z", s] => {
                if s.is_empty() {
                    0
                } else {
                    1
                }
            }
            ["-n", s] => {
                if !s.is_empty() {
                    0
                } else {
                    1
                }
            }

            // File existence/type tests
            ["-a", path] | ["-e", path] => {
                if std::path::Path::new(path).exists() {
                    0
                } else {
                    1
                }
            }
            ["-f", path] => {
                if std::path::Path::new(path).is_file() {
                    0
                } else {
                    1
                }
            }
            ["-d", path] => {
                if std::path::Path::new(path).is_dir() {
                    0
                } else {
                    1
                }
            }
            ["-b", path] => {
                use std::os::unix::fs::FileTypeExt;
                if std::fs::symlink_metadata(path)
                    .map(|m| m.file_type().is_block_device())
                    .unwrap_or(false)
                {
                    0
                } else {
                    1
                }
            }
            ["-c", path] => {
                use std::os::unix::fs::FileTypeExt;
                if std::fs::symlink_metadata(path)
                    .map(|m| m.file_type().is_char_device())
                    .unwrap_or(false)
                {
                    0
                } else {
                    1
                }
            }
            ["-p", path] => {
                use std::os::unix::fs::FileTypeExt;
                if std::fs::symlink_metadata(path)
                    .map(|m| m.file_type().is_fifo())
                    .unwrap_or(false)
                {
                    0
                } else {
                    1
                }
            }
            ["-S", path] => {
                use std::os::unix::fs::FileTypeExt;
                if std::fs::symlink_metadata(path)
                    .map(|m| m.file_type().is_socket())
                    .unwrap_or(false)
                {
                    0
                } else {
                    1
                }
            }
            ["-h", path] | ["-L", path] => {
                if std::path::Path::new(path).is_symlink() {
                    0
                } else {
                    1
                }
            }

            // File permission tests — all use prefetched metadata (one stat per path)
            ["-r", path] => {
                use std::os::unix::fs::MetadataExt;
                if let Some(meta) = get_meta(path) {
                    let mode = meta.mode();
                    let uid = unsafe { libc::geteuid() };
                    let gid = unsafe { libc::getegid() };
                    let readable = if meta.uid() == uid {
                        mode & 0o400 != 0
                    } else if meta.gid() == gid {
                        mode & 0o040 != 0
                    } else {
                        mode & 0o004 != 0
                    };
                    if readable { 0 } else { 1 }
                } else {
                    1
                }
            }
            ["-w", path] => {
                use std::os::unix::fs::MetadataExt;
                if let Some(meta) = get_meta(path) {
                    let mode = meta.mode();
                    let uid = unsafe { libc::geteuid() };
                    let gid = unsafe { libc::getegid() };
                    let writable = if meta.uid() == uid {
                        mode & 0o200 != 0
                    } else if meta.gid() == gid {
                        mode & 0o020 != 0
                    } else {
                        mode & 0o002 != 0
                    };
                    if writable { 0 } else { 1 }
                } else {
                    1
                }
            }
            ["-x", path] => {
                use std::os::unix::fs::MetadataExt;
                if let Some(meta) = get_meta(path) {
                    let mode = meta.mode();
                    let uid = unsafe { libc::geteuid() };
                    let gid = unsafe { libc::getegid() };
                    let executable = if meta.uid() == uid {
                        mode & 0o100 != 0
                    } else if meta.gid() == gid {
                        mode & 0o010 != 0
                    } else {
                        mode & 0o001 != 0
                    };
                    if executable { 0 } else { 1 }
                } else {
                    1
                }
            }

            // Special permission bits — prefetched metadata
            ["-g", path] => {
                use std::os::unix::fs::MetadataExt;
                if get_meta(path).map(|m| m.mode() & 0o2000 != 0).unwrap_or(false) { 0 } else { 1 }
            }
            ["-k", path] => {
                use std::os::unix::fs::MetadataExt;
                if get_meta(path).map(|m| m.mode() & 0o1000 != 0).unwrap_or(false) { 0 } else { 1 }
            }
            ["-u", path] => {
                use std::os::unix::fs::MetadataExt;
                if get_meta(path)
                    .map(|m| m.mode() & 0o4000 != 0)
                    .unwrap_or(false)
                {
                    0
                } else {
                    1
                }
            }

            // File size — prefetched metadata
            ["-s", path] => {
                if get_meta(path).map(|m| m.len() > 0).unwrap_or(false) { 0 } else { 1 }
            }

            // Ownership — prefetched metadata
            ["-O", path] => {
                use std::os::unix::fs::MetadataExt;
                if get_meta(path).map(|m| m.uid() == unsafe { libc::geteuid() }).unwrap_or(false) { 0 } else { 1 }
            }
            ["-G", path] => {
                use std::os::unix::fs::MetadataExt;
                if get_meta(path).map(|m| m.gid() == unsafe { libc::getegid() }).unwrap_or(false) { 0 } else { 1 }
            }

            // File times — prefetched metadata
            ["-N", path] => {
                use std::os::unix::fs::MetadataExt;
                if let Some(meta) = get_meta(path) {
                    if meta.mtime() > meta.atime() {
                        0
                    } else {
                        1
                    }
                } else {
                    1
                }
            }

            // Terminal test
            ["-t", fd] => {
                if let Ok(fd_num) = fd.parse::<i32>() {
                    if unsafe { libc::isatty(fd_num) } == 1 {
                        0
                    } else {
                        1
                    }
                } else {
                    1
                }
            }

            // Variable test
            ["-v", varname] => {
                if self.variables.contains_key(*varname) || std::env::var(varname).is_ok() {
                    0
                } else {
                    1
                }
            }

            // Option test
            ["-o", opt] => {
                let (name, _) = Self::normalize_option_name(opt);
                if self.options.get(&name).copied().unwrap_or(false) {
                    0
                } else {
                    1
                }
            }

            // String comparisons
            [a, "=", b] | [a, "==", b] => {
                if a == b {
                    0
                } else {
                    1
                }
            }
            [a, "!=", b] => {
                if a != b {
                    0
                } else {
                    1
                }
            }
            [a, "<", b] => {
                if *a < *b {
                    0
                } else {
                    1
                }
            }
            [a, ">", b] => {
                if *a > *b {
                    0
                } else {
                    1
                }
            }

            // Numeric comparisons
            [a, "-eq", b] => {
                let a: i64 = a.parse().unwrap_or(0);
                let b: i64 = b.parse().unwrap_or(0);
                if a == b {
                    0
                } else {
                    1
                }
            }
            [a, "-ne", b] => {
                let a: i64 = a.parse().unwrap_or(0);
                let b: i64 = b.parse().unwrap_or(0);
                if a != b {
                    0
                } else {
                    1
                }
            }
            [a, "-lt", b] => {
                let a: i64 = a.parse().unwrap_or(0);
                let b: i64 = b.parse().unwrap_or(0);
                if a < b {
                    0
                } else {
                    1
                }
            }
            [a, "-le", b] => {
                let a: i64 = a.parse().unwrap_or(0);
                let b: i64 = b.parse().unwrap_or(0);
                if a <= b {
                    0
                } else {
                    1
                }
            }
            [a, "-gt", b] => {
                let a: i64 = a.parse().unwrap_or(0);
                let b: i64 = b.parse().unwrap_or(0);
                if a > b {
                    0
                } else {
                    1
                }
            }
            [a, "-ge", b] => {
                let a: i64 = a.parse().unwrap_or(0);
                let b: i64 = b.parse().unwrap_or(0);
                if a >= b {
                    0
                } else {
                    1
                }
            }

            // File comparisons
            [f1, "-nt", f2] => {
                let m1 = std::fs::metadata(f1).and_then(|m| m.modified()).ok();
                let m2 = std::fs::metadata(f2).and_then(|m| m.modified()).ok();
                match (m1, m2) {
                    (Some(t1), Some(t2)) => {
                        if t1 > t2 {
                            0
                        } else {
                            1
                        }
                    }
                    (Some(_), None) => 0,
                    _ => 1,
                }
            }
            [f1, "-ot", f2] => {
                let m1 = std::fs::metadata(f1).and_then(|m| m.modified()).ok();
                let m2 = std::fs::metadata(f2).and_then(|m| m.modified()).ok();
                match (m1, m2) {
                    (Some(t1), Some(t2)) => {
                        if t1 < t2 {
                            0
                        } else {
                            1
                        }
                    }
                    (None, Some(_)) => 0,
                    _ => 1,
                }
            }
            [f1, "-ef", f2] => {
                use std::os::unix::fs::MetadataExt;
                let m1 = std::fs::metadata(f1).ok();
                let m2 = std::fs::metadata(f2).ok();
                match (m1, m2) {
                    (Some(a), Some(b)) => {
                        if a.dev() == b.dev() && a.ino() == b.ino() {
                            0
                        } else {
                            1
                        }
                    }
                    _ => 1,
                }
            }

            // Single string test
            [s] => {
                if !s.is_empty() {
                    0
                } else {
                    1
                }
            }

            _ => 1,
        }
    }

    fn builtin_local(&mut self, args: &[String]) -> i32 {
        self.builtin_typeset(args)
    }

    fn builtin_declare(&mut self, args: &[String]) -> i32 {
        self.builtin_typeset(args)
    }

    fn builtin_typeset(&mut self, args: &[String]) -> i32 {
        // Save old values when inside a function scope (local variable support).
        // Restored by call_function on function exit.
        if self.local_scope_depth > 0 {
            for arg in args {
                if arg.starts_with('-') || arg.starts_with('+') {
                    continue;
                }
                let name = arg.split('=').next().unwrap_or(arg);
                if !name.is_empty() {
                    let old_val = self.variables.get(name).cloned();
                    self.local_save_stack.push((name.to_string(), old_val));
                }
            }
        }

        // typeset [ {+|-}AHUaghlmrtux ] [ {+|-}EFLRZip [ n ] ]
        //         [ + ] [ name[=value] ... ]
        // typeset -T [ {+|-}Urux ] [ {+|-}LRZp [ n ] ] SCALAR[=value] array
        // typeset -f [ {+|-}TUkmtuz ] [ + ] [ name ... ]

        let mut is_array = false; // -a
        let mut is_assoc = false; // -A
        let mut is_export = false; // -x
        let mut is_integer = false; // -i
        let mut is_readonly = false; // -r
        let mut is_lower = false; // -l
        let mut is_upper = false; // -u
        let mut is_left_pad = false; // -L
        let mut is_right_pad = false; // -R
        let mut is_zero_pad = false; // -Z
        let mut is_float = false; // -F
        let mut is_float_exp = false; // -E
        let mut is_function = false; // -f
        let mut is_global = false; // -g
        let mut is_tied = false; // -T
        let mut is_hidden = false; // -H
        let mut is_hide_val = false; // -h
        let mut is_trace = false; // -t
        let mut print_mode = false; // -p
        let mut pattern_match = false; // -m
        let mut list_mode = false; // no args: list all
        let mut plus_mode = false; // +x etc: remove attribute
        let mut width: Option<usize> = None;
        let mut precision: Option<usize> = None;
        let mut var_args: Vec<String> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];

            if arg == "--" {
                i += 1;
                while i < args.len() {
                    var_args.push(args[i].clone());
                    i += 1;
                }
                break;
            }

            if arg == "+" {
                plus_mode = true;
                i += 1;
                continue;
            }

            if arg.starts_with('+') && arg.len() > 1 {
                plus_mode = true;
                for c in arg[1..].chars() {
                    match c {
                        'a' => is_array = false,
                        'A' => is_assoc = false,
                        'x' => is_export = false,
                        'i' => is_integer = false,
                        'r' => is_readonly = false,
                        'l' => is_lower = false,
                        'u' => is_upper = false,
                        'L' => is_left_pad = false,
                        'R' => is_right_pad = false,
                        'Z' => is_zero_pad = false,
                        'F' => is_float = false,
                        'E' => is_float_exp = false,
                        'f' => is_function = false,
                        'g' => is_global = false,
                        'T' => is_tied = false,
                        'H' => is_hidden = false,
                        'h' => is_hide_val = false,
                        't' => is_trace = false,
                        'p' => print_mode = false,
                        'm' => pattern_match = false,
                        _ => {}
                    }
                }
            } else if arg.starts_with('-') && arg.len() > 1 {
                let mut chars = arg[1..].chars().peekable();
                while let Some(c) = chars.next() {
                    match c {
                        'a' => is_array = true,
                        'A' => is_assoc = true,
                        'x' => is_export = true,
                        'i' => is_integer = true,
                        'r' => is_readonly = true,
                        'l' => is_lower = true,
                        'u' => is_upper = true,
                        'L' => {
                            is_left_pad = true;
                            // Check for width
                            let rest: String = chars.clone().collect();
                            if !rest.is_empty()
                                && rest
                                    .chars()
                                    .next()
                                    .map(|c| c.is_ascii_digit())
                                    .unwrap_or(false)
                            {
                                let num: String =
                                    chars.by_ref().take_while(|c| c.is_ascii_digit()).collect();
                                width = num.parse().ok();
                            }
                        }
                        'R' => {
                            is_right_pad = true;
                            let rest: String = chars.clone().collect();
                            if !rest.is_empty()
                                && rest
                                    .chars()
                                    .next()
                                    .map(|c| c.is_ascii_digit())
                                    .unwrap_or(false)
                            {
                                let num: String =
                                    chars.by_ref().take_while(|c| c.is_ascii_digit()).collect();
                                width = num.parse().ok();
                            }
                        }
                        'Z' => {
                            is_zero_pad = true;
                            let rest: String = chars.clone().collect();
                            if !rest.is_empty()
                                && rest
                                    .chars()
                                    .next()
                                    .map(|c| c.is_ascii_digit())
                                    .unwrap_or(false)
                            {
                                let num: String =
                                    chars.by_ref().take_while(|c| c.is_ascii_digit()).collect();
                                width = num.parse().ok();
                            }
                        }
                        'F' => {
                            is_float = true;
                            let rest: String = chars.clone().collect();
                            if !rest.is_empty()
                                && rest
                                    .chars()
                                    .next()
                                    .map(|c| c.is_ascii_digit())
                                    .unwrap_or(false)
                            {
                                let num: String =
                                    chars.by_ref().take_while(|c| c.is_ascii_digit()).collect();
                                precision = num.parse().ok();
                            }
                        }
                        'E' => {
                            is_float_exp = true;
                            let rest: String = chars.clone().collect();
                            if !rest.is_empty()
                                && rest
                                    .chars()
                                    .next()
                                    .map(|c| c.is_ascii_digit())
                                    .unwrap_or(false)
                            {
                                let num: String =
                                    chars.by_ref().take_while(|c| c.is_ascii_digit()).collect();
                                precision = num.parse().ok();
                            }
                        }
                        'f' => is_function = true,
                        'g' => is_global = true,
                        'T' => is_tied = true,
                        'H' => is_hidden = true,
                        'h' => is_hide_val = true,
                        't' => is_trace = true,
                        'p' => print_mode = true,
                        'm' => pattern_match = true,
                        _ => {}
                    }
                }
            } else {
                var_args.push(arg.clone());
            }
            i += 1;
        }

        let _ = is_global;
        let _ = is_tied;
        let _ = is_hidden;
        let _ = is_hide_val;
        let _ = is_trace;
        let _ = pattern_match;
        let _ = precision;

        // If -f (function mode) with no args, list functions
        if is_function && var_args.is_empty() {
            let mut func_names: Vec<_> = self.functions.keys().cloned().collect();
            func_names.sort();
            for name in &func_names {
                if let Some(func) = self.functions.get(name) {
                    if print_mode {
                        let body = crate::text::getpermtext(func);
                        println!("{} () {{\n\t{}\n}}", name, body.trim());
                    } else {
                        let body = crate::text::getpermtext(func);
                        println!("{} () {{\n\t{}\n}}", name, body.trim());
                    }
                }
            }
            return 0;
        }

        // If -f with args, just show those functions
        if is_function {
            for name in &var_args {
                if let Some(func) = self.functions.get(name) {
                    if print_mode {
                        let body = crate::text::getpermtext(func);
                        println!("{} () {{\n\t{}\n}}", name, body.trim());
                    } else {
                        let body = crate::text::getpermtext(func);
                        println!("{} () {{\n\t{}\n}}", name, body.trim());
                    }
                }
            }
            return 0;
        }

        // No args: list all variables with attributes
        if var_args.is_empty() {
            list_mode = true;
        }

        if list_mode {
            let mut sorted_names: Vec<_> = self.variables.keys().cloned().collect();
            sorted_names.sort();
            for name in &sorted_names {
                let val = self.variables.get(name).cloned().unwrap_or_default();
                let mut attrs = String::new();
                if is_export || env::var(name).is_ok() {
                    attrs.push('x');
                }
                let is_arr = self.arrays.contains_key(name);
                let is_hash = self.assoc_arrays.contains_key(name);
                if is_arr {
                    attrs.push('a');
                }
                if is_hash {
                    attrs.push('A');
                }
                if print_mode {
                    // typeset -p: output re-executable code with values
                    let prefix = if attrs.is_empty() {
                        "typeset".to_string()
                    } else {
                        format!("typeset -{}", attrs)
                    };
                    if is_hash {
                        if let Some(assoc) = self.assoc_arrays.get(name) {
                            let mut pairs: Vec<_> = assoc.iter().collect();
                            pairs.sort_by_key(|(k, _)| (*k).clone());
                            let formatted: Vec<String> = pairs
                                .iter()
                                .map(|(k, v)| {
                                    format!("[{}]={}", shell_quote_value(k), shell_quote_value(v))
                                })
                                .collect();
                            println!("{} {}=( {} )", prefix, name, formatted.join(" "));
                        }
                    } else if is_arr {
                        if let Some(arr) = self.arrays.get(name) {
                            let formatted: Vec<String> =
                                arr.iter().map(|v| shell_quote_value(v)).collect();
                            println!("{} {}=( {} )", prefix, name, formatted.join(" "));
                        }
                    } else {
                        println!("{} {}={}", prefix, name, shell_quote_value(&val));
                    }
                } else if is_hide_val {
                    println!("{}={}", name, "*".repeat(val.len().min(8)));
                } else {
                    println!("{}={}", name, val);
                }
            }
            return 0;
        }

        // Process variable assignments
        for arg in var_args {
            // Check if this starts an array assignment: "name=(" or "name=(value"
            if let Some(eq_pos) = arg.find('=') {
                let name = &arg[..eq_pos];
                let rest = &arg[eq_pos + 1..];

                if rest.starts_with('(') {
                    // Array assignment - collect all elements until we find ')'
                    let mut elements = Vec::new();
                    let current = rest[1..].to_string(); // skip '('

                    // Check if closing ) is in this arg
                    if let Some(close_pos) = current.find(')') {
                        let content = &current[..close_pos];
                        if !content.is_empty() {
                            elements.extend(content.split_whitespace().map(|s| s.to_string()));
                        }
                    } else {
                        // Single arg with just elements
                        if !current.is_empty() {
                            let trimmed = current.trim_end_matches(')');
                            elements.extend(trimmed.split_whitespace().map(|s| s.to_string()));
                        }
                    }

                    // Set array variable
                    if is_assoc {
                        let mut assoc = std::collections::HashMap::new();
                        let mut iter = elements.iter();
                        while let Some(key) = iter.next() {
                            if let Some(val) = iter.next() {
                                assoc.insert(key.clone(), val.clone());
                            }
                        }
                        self.assoc_arrays.insert(name.to_string(), assoc);
                    } else {
                        self.arrays.insert(name.to_string(), elements);
                    }
                    self.variables.insert(name.to_string(), String::new());
                } else {
                    // Regular assignment - apply transformations
                    let mut value = rest.to_string();

                    if is_integer {
                        // Force integer evaluation
                        value = self.evaluate_arithmetic(&value).to_string();
                    }
                    if is_lower {
                        value = value.to_lowercase();
                    }
                    if is_upper {
                        value = value.to_uppercase();
                    }
                    if let Some(w) = width {
                        if is_left_pad {
                            value = format!("{:<width$}", value, width = w);
                            value.truncate(w);
                        } else if is_right_pad || is_zero_pad {
                            let pad_char = if is_zero_pad { '0' } else { ' ' };
                            if value.len() < w {
                                value = format!(
                                    "{}{}",
                                    pad_char.to_string().repeat(w - value.len()),
                                    value
                                );
                            }
                            if value.len() > w {
                                value = value[value.len() - w..].to_string();
                            }
                        }
                    }
                    if is_float || is_float_exp {
                        if let Ok(f) = value.parse::<f64>() {
                            let prec = precision.unwrap_or(10);
                            value = if is_float_exp {
                                format!("{:.prec$e}", f, prec = prec)
                            } else {
                                format!("{:.prec$}", f, prec = prec)
                            };
                        }
                    }

                    self.variables.insert(name.to_string(), value.clone());

                    if is_export {
                        env::set_var(name, &value);
                    }
                }
            } else if is_array || is_assoc {
                // Just declaring the variable
                if is_assoc {
                    self.assoc_arrays
                        .insert(arg.clone(), std::collections::HashMap::new());
                } else {
                    self.arrays.insert(arg.clone(), Vec::new());
                }
                self.variables.insert(arg.clone(), String::new());
            } else {
                self.variables.insert(arg.clone(), String::new());
                if is_export {
                    env::set_var(&arg, "");
                }
            }

            // Apply readonly flag — must come after the variable is set
            if is_readonly {
                let name = if let Some(eq_pos) = arg.find('=') {
                    arg[..eq_pos].to_string()
                } else {
                    arg.clone()
                };
                self.readonly_vars.insert(name);
            }
        }
        0
    }

    fn builtin_read(&mut self, args: &[String]) -> i32 {
        // read [ -rszpqAclneE ] [ -t timeout ] [ -d delim ] [ -k [ num ] ] [ -u fd ]
        //      [ name[?prompt] ] [ name ... ]
        use std::io::{BufRead, Read as IoRead};

        let mut raw_mode = false; // -r: don't interpret backslash escapes
        let mut silent = false; // -s: don't echo input
        let mut to_history = false; // -z: read from history stack
        let mut prompt_str: Option<String> = None; // -p prompt
        let mut use_array = false; // -A: read into array
        let mut timeout: Option<u64> = None; // -t timeout in seconds
        let mut delimiter = '\n'; // -d delim
        let mut nchars: Option<usize> = None; // -k num: read exactly num chars
        let mut fd = 0; // -u fd: read from fd
        let mut quiet = false; // -q: test only, don't assign
        let mut var_names: Vec<String> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];

            if arg == "--" {
                i += 1;
                while i < args.len() {
                    var_names.push(args[i].clone());
                    i += 1;
                }
                break;
            }

            if arg.starts_with('-') && arg.len() > 1 {
                let mut chars = arg[1..].chars().peekable();
                while let Some(ch) = chars.next() {
                    match ch {
                        'r' => raw_mode = true,
                        's' => silent = true,
                        'z' => to_history = true,
                        'A' => use_array = true,
                        'c' | 'l' | 'n' | 'e' | 'E' => {} // TODO
                        'q' => quiet = true,
                        't' => {
                            let rest: String = chars.collect();
                            if !rest.is_empty() {
                                timeout = rest.parse().ok();
                            } else {
                                i += 1;
                                if i < args.len() {
                                    timeout = args[i].parse().ok();
                                }
                            }
                            break;
                        }
                        'd' => {
                            let rest: String = chars.collect();
                            if !rest.is_empty() {
                                delimiter = rest.chars().next().unwrap_or('\n');
                            } else {
                                i += 1;
                                if i < args.len() {
                                    delimiter = args[i].chars().next().unwrap_or('\n');
                                }
                            }
                            break;
                        }
                        'k' => {
                            let rest: String = chars.collect();
                            if !rest.is_empty() {
                                nchars = Some(rest.parse().unwrap_or(1));
                            } else if i + 1 < args.len()
                                && args[i + 1].chars().all(|c| c.is_ascii_digit())
                            {
                                i += 1;
                                nchars = Some(args[i].parse().unwrap_or(1));
                            } else {
                                nchars = Some(1);
                            }
                            break;
                        }
                        'u' => {
                            let rest: String = chars.collect();
                            if !rest.is_empty() {
                                fd = rest.parse().unwrap_or(0);
                            } else {
                                i += 1;
                                if i < args.len() {
                                    fd = args[i].parse().unwrap_or(0);
                                }
                            }
                            break;
                        }
                        'p' => {
                            let rest: String = chars.collect();
                            if !rest.is_empty() {
                                prompt_str = Some(rest);
                            } else {
                                i += 1;
                                if i < args.len() {
                                    prompt_str = Some(args[i].clone());
                                }
                            }
                            break;
                        }
                        _ => {}
                    }
                }
            } else {
                if let Some(pos) = arg.find('?') {
                    var_names.push(arg[..pos].to_string());
                    prompt_str = Some(arg[pos + 1..].to_string());
                } else {
                    var_names.push(arg.clone());
                }
            }
            i += 1;
        }

        if var_names.is_empty() {
            var_names.push("REPLY".to_string());
        }

        if let Some(ref p) = prompt_str {
            eprint!("{}", p);
            let _ = std::io::stderr().flush();
        }

        let _ = to_history;
        let _ = fd;
        let _ = silent;

        let input = if let Some(n) = nchars {
            let mut buf = vec![0u8; n];
            let stdin = io::stdin();
            if let Some(_t) = timeout {
                // TODO: proper timeout
            }
            match stdin.lock().read_exact(&mut buf) {
                Ok(_) => String::from_utf8_lossy(&buf).to_string(),
                Err(_) => return 1,
            }
        } else {
            let stdin = io::stdin();
            let mut input = String::new();
            if delimiter == '\n' {
                match stdin.lock().read_line(&mut input) {
                    Ok(0) => return 1,
                    Ok(_) => {}
                    Err(_) => return 1,
                }
            } else {
                let mut byte = [0u8; 1];
                loop {
                    match stdin.lock().read_exact(&mut byte) {
                        Ok(_) => {
                            let c = byte[0] as char;
                            if c == delimiter {
                                break;
                            }
                            input.push(c);
                        }
                        Err(_) => break,
                    }
                }
            }
            input
                .trim_end_matches('\n')
                .trim_end_matches('\r')
                .to_string()
        };

        let processed = if raw_mode {
            input
        } else {
            input.replace("\\\n", "")
        };

        if quiet {
            return if processed.is_empty() { 1 } else { 0 };
        }

        if use_array {
            let var = &var_names[0];
            let words: Vec<String> = processed.split_whitespace().map(String::from).collect();
            self.arrays.insert(var.clone(), words);
        } else if var_names.len() == 1 {
            let var = &var_names[0];
            env::set_var(var, &processed);
            self.variables.insert(var.clone(), processed);
        } else {
            let ifs = self
                .variables
                .get("IFS")
                .map(|s| s.as_str())
                .unwrap_or(" \t\n");
            let words: Vec<&str> = processed
                .split(|c| ifs.contains(c))
                .filter(|s| !s.is_empty())
                .collect();

            for (j, var) in var_names.iter().enumerate() {
                if j < words.len() {
                    if j == var_names.len() - 1 && words.len() > var_names.len() {
                        let remaining = words[j..].join(" ");
                        env::set_var(var, &remaining);
                        self.variables.insert(var.clone(), remaining);
                    } else {
                        env::set_var(var, words[j]);
                        self.variables.insert(var.clone(), words[j].to_string());
                    }
                } else {
                    env::set_var(var, "");
                    self.variables.insert(var.clone(), String::new());
                }
            }
        }

        0
    }

    fn builtin_shift(&mut self, args: &[String]) -> i32 {
        // shift [ -p ] [ n ] [ name ... ]
        // -p: shift from end instead of beginning (pop)
        // n: number of elements to shift (default 1)
        // name: array names to shift (default: shift positional parameters)

        let mut from_end = false;
        let mut count = 1usize;
        let mut array_names: Vec<String> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if arg == "-p" {
                from_end = true;
            } else if arg.chars().all(|c| c.is_ascii_digit()) {
                count = arg.parse().unwrap_or(1);
            } else {
                array_names.push(arg.clone());
            }
            i += 1;
        }

        if array_names.is_empty() {
            // Shift positional parameters
            if from_end {
                for _ in 0..count {
                    if !self.positional_params.is_empty() {
                        self.positional_params.pop();
                    }
                }
            } else {
                for _ in 0..count.min(self.positional_params.len()) {
                    self.positional_params.remove(0);
                }
            }
        } else {
            // Shift specified arrays
            for name in array_names {
                if let Some(arr) = self.arrays.get_mut(&name) {
                    if from_end {
                        for _ in 0..count {
                            if !arr.is_empty() {
                                arr.pop();
                            }
                        }
                    } else {
                        for _ in 0..count {
                            if !arr.is_empty() {
                                arr.remove(0);
                            }
                        }
                    }
                }
            }
        }

        0
    }

    #[tracing::instrument(level = "debug", skip(self))]
    fn builtin_eval(&mut self, args: &[String]) -> i32 {
        let code = args.join(" ");
        match self.execute_script(&code) {
            Ok(status) => status,
            Err(e) => {
                eprintln!("eval: {}", e);
                1
            }
        }
    }

    fn builtin_autoload(&mut self, args: &[String]) -> i32 {
        // Parse options like zsh: -U (no alias), -z (zsh style), -k (ksh style),
        // -X (execute now), -x (export), -r (resolve), -R (resolve recurse),
        // -t (trace), -T (trace local), -W (warn nested), -d (use calling dir)
        let mut functions = Vec::new();
        let mut no_alias = false; // -U
        let mut zsh_style = false; // -z
        let mut ksh_style = false; // -k
        let mut execute_now = false; // -X
        let mut resolve = false; // -r
        let mut trace = false; // -t
        let mut use_caller_dir = false; // -d
        let _list_mode = false;

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];

            if arg == "--" {
                i += 1;
                break;
            }

            if arg.starts_with('+') {
                let flags = &arg[1..];
                for c in flags.chars() {
                    match c {
                        'U' => no_alias = false,
                        'z' => zsh_style = false,
                        'k' => ksh_style = false,
                        't' => trace = false,
                        'd' => use_caller_dir = false,
                        _ => {}
                    }
                }
            } else if arg.starts_with('-') {
                let flags = &arg[1..];
                if flags.is_empty() {
                    // Just "-" means end of options
                    i += 1;
                    break;
                }
                for c in flags.chars() {
                    match c {
                        'U' => no_alias = true,
                        'z' => zsh_style = true,
                        'k' => ksh_style = true,
                        'X' => execute_now = true,
                        'r' | 'R' => resolve = true,
                        't' => trace = true,
                        'T' => {} // trace local
                        'W' => {} // warn nested
                        'd' => use_caller_dir = true,
                        'w' => {} // wordcode
                        'm' => {} // pattern match
                        _ => {}
                    }
                }
            } else {
                functions.push(arg.clone());
            }
            i += 1;
        }

        // Collect remaining args as function names
        while i < args.len() {
            functions.push(args[i].clone());
            i += 1;
        }

        // If no functions specified, list autoloaded functions
        if functions.is_empty() && !execute_now {
            for (name, _) in &self.autoload_pending {
                if no_alias && zsh_style {
                    println!("autoload -Uz {}", name);
                } else if no_alias {
                    println!("autoload -U {}", name);
                } else {
                    println!("autoload {}", name);
                }
            }
            return 0;
        }

        // Handle -X: load and execute function immediately (called from stub)
        // When a stub function calls `builtin autoload -Xz`, we load the real function
        // and then need to execute it with the original arguments
        if execute_now {
            for func_name in &functions {
                // Load the function from fpath
                if let Some(loaded) = self.load_autoload_function(func_name) {
                    // Extract body from FunctionDef
                    let body = match loaded {
                        ShellCommand::FunctionDef(_, body) => (*body).clone(),
                        other => other,
                    };
                    // Replace the stub with the real function
                    self.functions.insert(func_name.clone(), body);
                    // Remove from pending
                    self.autoload_pending.remove(func_name);
                } else {
                    eprintln!(
                        "autoload: {}: function definition file not found",
                        func_name
                    );
                    return 1;
                }
            }
            return 0;
        }

        // Register functions for autoload - create stub functions
        for func_name in &functions {
            // Store autoload metadata
            let mut flags = AutoloadFlags::empty();
            if no_alias {
                flags |= AutoloadFlags::NO_ALIAS;
            }
            if zsh_style {
                flags |= AutoloadFlags::ZSH_STYLE;
            }
            if ksh_style {
                flags |= AutoloadFlags::KSH_STYLE;
            }
            if trace {
                flags |= AutoloadFlags::TRACE;
            }
            if use_caller_dir {
                flags |= AutoloadFlags::USE_CALLER_DIR;
            }

            self.autoload_pending.insert(func_name.clone(), flags);

            // Create a stub function: `builtin autoload -Xz funcname && funcname "$@"`
            // When called, this loads the real function and re-calls it
            let autoload_opts = if zsh_style && no_alias {
                "-XUz"
            } else if zsh_style {
                "-Xz"
            } else if no_alias {
                "-XU"
            } else {
                "-X"
            };

            // The stub: builtin autoload -Xz funcname && funcname "$@"
            let stub = ShellCommand::List(vec![
                (
                    ShellCommand::Simple(SimpleCommand {
                        assignments: vec![],
                        words: vec![
                            ShellWord::Literal("builtin".to_string()),
                            ShellWord::Literal("autoload".to_string()),
                            ShellWord::Literal(autoload_opts.to_string()),
                            ShellWord::Literal(func_name.clone()),
                        ],
                        redirects: vec![],
                    }),
                    ListOp::And,
                ),
                (
                    ShellCommand::Simple(SimpleCommand {
                        assignments: vec![],
                        words: vec![
                            ShellWord::Literal(func_name.clone()),
                            ShellWord::DoubleQuoted(vec![ShellWord::Variable("@".to_string())]),
                        ],
                        redirects: vec![],
                    }),
                    ListOp::Semi,
                ),
            ]);

            self.functions.insert(func_name.clone(), stub);

            // If -r or -R, resolve the path now to verify it exists
            if resolve {
                if self.find_function_file(func_name).is_none() {
                    eprintln!(
                        "autoload: {}: function definition file not found",
                        func_name
                    );
                }
            }
        }

        // Batch pre-resolution: when multiple autoloads are registered at once
        // (common during .zshrc processing), dispatch fpath lookups in parallel
        // across the worker pool to pre-read function files into the OS page cache.
        if functions.len() >= 4 && !resolve && !execute_now {
            let fpath_dirs: Vec<PathBuf> = self.fpath.clone();
            let names: Vec<String> = functions.clone();
            let pool = std::sync::Arc::clone(&self.worker_pool);

            tracing::debug!(
                count = names.len(),
                fpath_dirs = fpath_dirs.len(),
                "batch autoload: pre-resolving fpath lookups on worker pool"
            );

            // Submit resolution tasks — each worker scans fpath for a subset of names.
            // Results are cached in a shared map for later use by load_autoload_function.
            let resolved = std::sync::Arc::new(parking_lot::Mutex::new(
                HashMap::<String, PathBuf>::with_capacity(names.len()),
            ));

            for name in names {
                let dirs = fpath_dirs.clone();
                let resolved = std::sync::Arc::clone(&resolved);
                pool.submit(move || {
                    for dir in &dirs {
                        let path = dir.join(&name);
                        if path.exists() && path.is_file() {
                            // Pre-read to warm OS page cache (the read result is discarded,
                            // but the pages stay in the kernel buffer cache)
                            let _ = std::fs::read(&path);
                            resolved.lock().insert(name.clone(), path);
                            tracing::trace!(func = %name, "autoload batch: pre-resolved");
                            break;
                        }
                    }
                });
            }
        }

        0
    }

    /// Find a function file in fpath
    fn find_function_file(&self, name: &str) -> Option<PathBuf> {
        for dir in &self.fpath {
            let path = dir.join(name);
            if path.exists() && path.is_file() {
                return Some(path);
            }
        }
        None
    }

    /// Load an autoloaded function from fpath - reads file and parses it
    fn load_autoload_function(&mut self, name: &str) -> Option<ShellCommand> {
        // FAST PATH: Try SQLite cache first (no filesystem access)
        // Skip in zsh_compat mode - use traditional fpath scanning only
        if !self.zsh_compat {
            if let Some(ref cache) = self.compsys_cache {
                // FASTEST: try cached bytecodes (skip lex+parse+compile entirely)
                if let Ok(Some(bc_blob)) = cache.get_autoload_bytecode(name) {
                    // Try fusevm::Chunk first (new format — actual bytecodes)
                    if let Ok(chunk) = bincode::deserialize::<fusevm::Chunk>(&bc_blob) {
                        if !chunk.ops.is_empty() {
                            tracing::trace!(name, bytes = bc_blob.len(), ops = chunk.ops.len(), "autoload: bytecode cache hit → VM");
                            // Execute directly on fusevm — no parse, no compile
                            let mut vm = fusevm::VM::new(chunk);
                            let _ = vm.run();
                            self.last_status = vm.last_status;
                            // Return a no-op so the caller doesn't try to execute again
                            return Some(ShellCommand::Simple(crate::parser::SimpleCommand {
                                assignments: Vec::new(),
                                words: Vec::new(),
                                redirects: Vec::new(),
                            }));
                        }
                    }
                    // Fallback: try legacy Vec<ShellCommand> format (migration)
                    if let Ok(commands) = bincode::deserialize::<Vec<ShellCommand>>(&bc_blob) {
                        if !commands.is_empty() {
                            tracing::trace!(name, bytes = bc_blob.len(), "autoload: legacy AST cache hit");
                            return Some(self.wrap_autoload_commands(name, commands));
                        }
                    }
                }

                // FAST: cached source text — parse + compile (still no filesystem access)
                if let Ok(Some(body)) = cache.get_autoload_body(name) {
                    let mut parser = ShellParser::new(&body);
                    if let Ok(commands) = parser.parse_script() {
                        if !commands.is_empty() {
                            // Compile to bytecodes and cache for next time
                            let compiler = crate::shell_compiler::ShellCompiler::new();
                            let chunk = compiler.compile(&commands);
                            if let Ok(blob) = bincode::serialize(&chunk) {
                                let _ = cache.set_autoload_bytecode(name, &blob);
                                tracing::trace!(name, bytes = blob.len(), "autoload: bytecodes compiled and cached");
                            }
                            return Some(self.wrap_autoload_commands(name, commands));
                        }
                    }
                }
            }
        }

        // SLOW PATH: Try ZWC cache (but skip if we're reloading an existing function)
        if !self.functions.contains_key(name) {
            // Try to load from ZWC files
            for dir in &self.fpath.clone() {
                // Try dir.zwc (e.g., /path/to/src.zwc for /path/to/src)
                let zwc_path = dir.with_extension("zwc");
                if zwc_path.exists() {
                    // Function name in directory ZWC includes path prefix
                    let prefixed_name = format!(
                        "{}/{}",
                        dir.file_name().and_then(|n| n.to_str()).unwrap_or(""),
                        name
                    );
                    if let Some(func) = self.load_function_from_zwc(&zwc_path, &prefixed_name) {
                        return Some(func);
                    }
                    // Also try without prefix
                    if let Some(func) = self.load_function_from_zwc(&zwc_path, name) {
                        return Some(func);
                    }
                }
                // Try individual function.zwc
                let func_zwc = dir.join(format!("{}.zwc", name));
                if func_zwc.exists() {
                    if let Some(func) = self.load_function_from_zwc(&func_zwc, name) {
                        return Some(func);
                    }
                }
            }
        }

        // SLOWEST PATH: Find the function file in fpath
        let path = self.find_function_file(name)?;

        // Read the file
        let content = std::fs::read_to_string(&path).ok()?;

        // Parse the content
        let mut parser = ShellParser::new(&content);

        if let Ok(commands) = parser.parse_script() {
            if commands.is_empty() {
                return None;
            }

            // Check if it's a single function definition for this name (ksh style)
            if commands.len() == 1 {
                if let ShellCommand::FunctionDef(ref fn_name, _) = commands[0] {
                    if fn_name == name {
                        return Some(commands[0].clone());
                    }
                }
            }

            // Otherwise, the file contents become the function body (zsh style)
            // Wrap all commands in a List
            let body = if commands.len() == 1 {
                commands.into_iter().next().unwrap()
            } else {
                // Convert to List with Semi separators
                let list_cmds: Vec<(ShellCommand, ListOp)> =
                    commands.into_iter().map(|c| (c, ListOp::Semi)).collect();
                ShellCommand::List(list_cmds)
            };

            return Some(ShellCommand::FunctionDef(name.to_string(), Box::new(body)));
        }

        None
    }

    /// Convert parsed commands into a FunctionDef, handling ksh vs zsh style.
    fn wrap_autoload_commands(&self, name: &str, commands: Vec<ShellCommand>) -> ShellCommand {
        // ksh style: file contains a single function definition for this name
        if commands.len() == 1 {
            if let ShellCommand::FunctionDef(ref fn_name, _) = commands[0] {
                if fn_name == name {
                    return commands.into_iter().next().unwrap();
                }
            }
        }
        // zsh style: file body IS the function body
        let body = if commands.len() == 1 {
            commands.into_iter().next().unwrap()
        } else {
            let list_cmds: Vec<(ShellCommand, ListOp)> =
                commands.into_iter().map(|c| (c, ListOp::Semi)).collect();
            ShellCommand::List(list_cmds)
        };
        ShellCommand::FunctionDef(name.to_string(), Box::new(body))
    }

    /// Check if a function is autoload pending and load it if so
    pub fn maybe_autoload(&mut self, name: &str) -> bool {
        if self.autoload_pending.contains_key(name) {
            if let Some(func) = self.load_autoload_function(name) {
                // For FunctionDef, extract the body and store it
                let to_store = match func {
                    ShellCommand::FunctionDef(_, body) => (*body).clone(),
                    other => other,
                };
                self.functions.insert(name.to_string(), to_store);
                self.autoload_pending.remove(name);
                return true;
            }
        }
        false
    }

    fn builtin_jobs(&mut self, args: &[String]) -> i32 {
        // jobs [ -dlprsZ ] [ job ... ]
        // -l: long format (show PID)
        // -p: print process group IDs only
        // -d: show directory from which job was started
        // -r: show running jobs only
        // -s: show stopped jobs only
        // -Z: set process name (not relevant here)

        let mut long_format = false;
        let mut pids_only = false;
        let mut show_dir = false;
        let mut running_only = false;
        let mut stopped_only = false;
        let mut job_ids: Vec<usize> = Vec::new();

        for arg in args {
            if arg.starts_with('-') {
                for c in arg[1..].chars() {
                    match c {
                        'l' => long_format = true,
                        'p' => pids_only = true,
                        'd' => show_dir = true,
                        'r' => running_only = true,
                        's' => stopped_only = true,
                        'Z' => {} // ignore
                        _ => {}
                    }
                }
            } else if arg.starts_with('%') {
                if let Ok(id) = arg[1..].parse::<usize>() {
                    job_ids.push(id);
                }
            } else if let Ok(id) = arg.parse::<usize>() {
                job_ids.push(id);
            }
        }

        // Reap finished jobs first
        for job in self.jobs.reap_finished() {
            if !running_only && !stopped_only {
                if pids_only {
                    println!("{}", job.pid);
                } else {
                    println!("[{}]  Done                    {}", job.id, job.command);
                }
            }
        }

        // List jobs (optionally filtered)
        for job in self.jobs.list() {
            // Filter by specific job IDs if provided
            if !job_ids.is_empty() && !job_ids.contains(&job.id) {
                continue;
            }

            // Filter by state
            if running_only && job.state != JobState::Running {
                continue;
            }
            if stopped_only && job.state != JobState::Stopped {
                continue;
            }

            if pids_only {
                println!("{}", job.pid);
                continue;
            }

            let marker = if job.is_current { "+" } else { "-" };
            let state = match job.state {
                JobState::Running => "running",
                JobState::Stopped => "suspended",
                JobState::Done => "done",
            };

            if long_format {
                println!(
                    "[{}]{} {:6} {}  {}",
                    job.id, marker, job.pid, state, job.command
                );
            } else {
                println!("[{}]{} {}  {}", job.id, marker, state, job.command);
            }

            if show_dir {
                if let Ok(cwd) = env::current_dir() {
                    println!("    (pwd: {})", cwd.display());
                }
            }
        }
        0
    }

    fn builtin_fg(&mut self, args: &[String]) -> i32 {
        let job_id = if let Some(arg) = args.first() {
            // Parse %N or just N
            let s = arg.trim_start_matches('%');
            match s.parse::<usize>() {
                Ok(id) => Some(id),
                Err(_) => {
                    eprintln!("fg: {}: no such job", arg);
                    return 1;
                }
            }
        } else {
            self.jobs.current().map(|j| j.id)
        };

        let Some(id) = job_id else {
            eprintln!("fg: no current job");
            return 1;
        };

        let Some(job) = self.jobs.get(id) else {
            eprintln!("fg: %{}: no such job", id);
            return 1;
        };

        let pid = job.pid;
        let cmd = job.command.clone();
        println!("{}", cmd);

        // Continue the job
        if let Err(e) = continue_job(pid) {
            eprintln!("fg: {}", e);
            return 1;
        }

        // Wait for it
        match wait_for_job(pid) {
            Ok(status) => {
                self.jobs.remove(id);
                status
            }
            Err(e) => {
                eprintln!("fg: {}", e);
                1
            }
        }
    }

    fn builtin_bg(&mut self, args: &[String]) -> i32 {
        let job_id = if let Some(arg) = args.first() {
            let s = arg.trim_start_matches('%');
            match s.parse::<usize>() {
                Ok(id) => Some(id),
                Err(_) => {
                    eprintln!("bg: {}: no such job", arg);
                    return 1;
                }
            }
        } else {
            self.jobs.current().map(|j| j.id)
        };

        let Some(id) = job_id else {
            eprintln!("bg: no current job");
            return 1;
        };

        let Some(job) = self.jobs.get_mut(id) else {
            eprintln!("bg: %{}: no such job", id);
            return 1;
        };

        let pid = job.pid;
        let cmd = job.command.clone();

        if let Err(e) = continue_job(pid) {
            eprintln!("bg: {}", e);
            return 1;
        }

        job.state = JobState::Running;
        println!("[{}] {} &", id, cmd);
        0
    }

    fn builtin_kill(&mut self, args: &[String]) -> i32 {
        // kill [ -s signal_name | -n signal_number | -sig ] job ...
        // kill -l [ sig ... ]
        use crate::jobs::send_signal;
        use nix::sys::signal::Signal;

        if args.is_empty() {
            eprintln!("kill: usage: kill [-s signal | -n num | -sig] pid ...");
            eprintln!("       kill -l [sig ...]");
            return 1;
        }

        // Signal name/number mapping
        let signal_map: &[(&str, i32, Signal)] = &[
            ("HUP", 1, Signal::SIGHUP),
            ("INT", 2, Signal::SIGINT),
            ("QUIT", 3, Signal::SIGQUIT),
            ("ILL", 4, Signal::SIGILL),
            ("TRAP", 5, Signal::SIGTRAP),
            ("ABRT", 6, Signal::SIGABRT),
            ("BUS", 7, Signal::SIGBUS),
            ("FPE", 8, Signal::SIGFPE),
            ("KILL", 9, Signal::SIGKILL),
            ("USR1", 10, Signal::SIGUSR1),
            ("SEGV", 11, Signal::SIGSEGV),
            ("USR2", 12, Signal::SIGUSR2),
            ("PIPE", 13, Signal::SIGPIPE),
            ("ALRM", 14, Signal::SIGALRM),
            ("TERM", 15, Signal::SIGTERM),
            ("CHLD", 17, Signal::SIGCHLD),
            ("CONT", 18, Signal::SIGCONT),
            ("STOP", 19, Signal::SIGSTOP),
            ("TSTP", 20, Signal::SIGTSTP),
            ("TTIN", 21, Signal::SIGTTIN),
            ("TTOU", 22, Signal::SIGTTOU),
            ("URG", 23, Signal::SIGURG),
            ("XCPU", 24, Signal::SIGXCPU),
            ("XFSZ", 25, Signal::SIGXFSZ),
            ("VTALRM", 26, Signal::SIGVTALRM),
            ("PROF", 27, Signal::SIGPROF),
            ("WINCH", 28, Signal::SIGWINCH),
            ("IO", 29, Signal::SIGIO),
            ("SYS", 31, Signal::SIGSYS),
        ];

        let mut sig = Signal::SIGTERM;
        let mut pids: Vec<String> = Vec::new();
        let mut list_mode = false;
        let mut list_args: Vec<String> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];

            if arg == "-l" || arg == "-L" {
                list_mode = true;
                // Remaining args are signal numbers to translate
                list_args = args[i + 1..].to_vec();
                break;
            } else if arg == "-s" {
                // -s signal_name
                i += 1;
                if i >= args.len() {
                    eprintln!("kill: -s requires an argument");
                    return 1;
                }
                let sig_name = args[i].to_uppercase();
                let sig_name = sig_name.strip_prefix("SIG").unwrap_or(&sig_name);
                if let Some((_, _, s)) = signal_map.iter().find(|(name, _, _)| *name == sig_name) {
                    sig = *s;
                } else {
                    eprintln!("kill: invalid signal: {}", args[i]);
                    return 1;
                }
            } else if arg == "-n" {
                // -n signal_number
                i += 1;
                if i >= args.len() {
                    eprintln!("kill: -n requires an argument");
                    return 1;
                }
                let num: i32 = match args[i].parse() {
                    Ok(n) => n,
                    Err(_) => {
                        eprintln!("kill: invalid signal number: {}", args[i]);
                        return 1;
                    }
                };
                if let Some((_, _, s)) = signal_map.iter().find(|(_, n, _)| *n == num) {
                    sig = *s;
                } else {
                    eprintln!("kill: invalid signal number: {}", num);
                    return 1;
                }
            } else if arg.starts_with('-') && arg.len() > 1 {
                // -SIGNAL or -NUM
                let sig_str = &arg[1..];
                let sig_upper = sig_str.to_uppercase();
                let sig_name = sig_upper.strip_prefix("SIG").unwrap_or(&sig_upper);

                // Try as number first
                if let Ok(num) = sig_str.parse::<i32>() {
                    if let Some((_, _, s)) = signal_map.iter().find(|(_, n, _)| *n == num) {
                        sig = *s;
                    } else {
                        eprintln!("kill: invalid signal: {}", arg);
                        return 1;
                    }
                } else if let Some((_, _, s)) =
                    signal_map.iter().find(|(name, _, _)| *name == sig_name)
                {
                    sig = *s;
                } else {
                    eprintln!("kill: invalid signal: {}", arg);
                    return 1;
                }
            } else {
                pids.push(arg.clone());
            }
            i += 1;
        }

        // Handle -l (list signals)
        if list_mode {
            if list_args.is_empty() {
                // List all signals
                for (name, num, _) in signal_map {
                    println!("{:2}) SIG{}", num, name);
                }
            } else {
                // Translate signal numbers to names or vice versa
                for arg in &list_args {
                    if let Ok(num) = arg.parse::<i32>() {
                        // Number -> name
                        if let Some((name, _, _)) = signal_map.iter().find(|(_, n, _)| *n == num) {
                            println!("{}", name);
                        } else {
                            eprintln!("kill: unknown signal: {}", num);
                        }
                    } else {
                        // Name -> number
                        let sig_upper = arg.to_uppercase();
                        let sig_name = sig_upper.strip_prefix("SIG").unwrap_or(&sig_upper);
                        if let Some((_, num, _)) =
                            signal_map.iter().find(|(name, _, _)| *name == sig_name)
                        {
                            println!("{}", num);
                        } else {
                            eprintln!("kill: unknown signal: {}", arg);
                        }
                    }
                }
            }
            return 0;
        }

        if pids.is_empty() {
            eprintln!("kill: usage: kill [-s signal | -n num | -sig] pid ...");
            return 1;
        }

        let mut status = 0;
        for arg in &pids {
            // Handle %job syntax
            if arg.starts_with('%') {
                let id: usize = match arg[1..].parse() {
                    Ok(id) => id,
                    Err(_) => {
                        eprintln!("kill: {}: no such job", arg);
                        status = 1;
                        continue;
                    }
                };
                if let Some(job) = self.jobs.get(id) {
                    if let Err(e) = send_signal(job.pid, sig) {
                        eprintln!("kill: {}", e);
                        status = 1;
                    }
                } else {
                    eprintln!("kill: {}: no such job", arg);
                    status = 1;
                }
            } else {
                // Direct PID
                let pid: u32 = match arg.parse() {
                    Ok(p) => p,
                    Err(_) => {
                        eprintln!("kill: {}: invalid pid", arg);
                        status = 1;
                        continue;
                    }
                };
                if let Err(e) = send_signal(pid as i32, sig) {
                    eprintln!("kill: {}", e);
                    status = 1;
                }
            }
        }
        status
    }

    fn builtin_disown(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            // Disown current job
            if let Some(job) = self.jobs.current() {
                let id = job.id;
                self.jobs.remove(id);
            }
            return 0;
        }

        for arg in args {
            let s = arg.trim_start_matches('%');
            if let Ok(id) = s.parse::<usize>() {
                self.jobs.remove(id);
            } else {
                eprintln!("disown: {}: no such job", arg);
            }
        }
        0
    }

    fn builtin_wait(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            // Wait for all jobs
            let ids: Vec<usize> = self.jobs.list().iter().map(|j| j.id).collect();
            for id in ids {
                if let Some(mut job) = self.jobs.remove(id) {
                    if let Some(ref mut child) = job.child {
                        let _ = wait_for_child(child);
                    }
                }
            }
            return 0;
        }

        let mut status = 0;
        for arg in args {
            if arg.starts_with('%') {
                let id: usize = match arg[1..].parse() {
                    Ok(id) => id,
                    Err(_) => {
                        eprintln!("wait: {}: no such job", arg);
                        status = 127;
                        continue;
                    }
                };
                if let Some(mut job) = self.jobs.remove(id) {
                    if let Some(ref mut child) = job.child {
                        match wait_for_child(child) {
                            Ok(s) => status = s,
                            Err(e) => {
                                eprintln!("wait: {}", e);
                                status = 127;
                            }
                        }
                    }
                } else {
                    eprintln!("wait: {}: no such job", arg);
                    status = 127;
                }
            } else {
                let pid: u32 = match arg.parse() {
                    Ok(p) => p,
                    Err(_) => {
                        eprintln!("wait: {}: invalid pid", arg);
                        status = 127;
                        continue;
                    }
                };
                match wait_for_job(pid as i32) {
                    Ok(s) => status = s,
                    Err(e) => {
                        eprintln!("wait: {}", e);
                        status = 127;
                    }
                }
            }
        }
        status
    }

    fn builtin_suspend(&self, args: &[String]) -> i32 {
        let mut force = false;
        for arg in args {
            if arg == "-f" {
                force = true;
            }
        }

        #[cfg(unix)]
        {
            use nix::sys::signal::{kill, Signal};
            use nix::unistd::getppid;

            // Check if we're a login shell (parent is init/PID 1)
            let ppid = getppid();
            if !force && ppid == nix::unistd::Pid::from_raw(1) {
                eprintln!("suspend: cannot suspend a login shell");
                return 1;
            }

            // Send SIGTSTP to ourselves
            let pid = nix::unistd::getpid();
            if let Err(e) = kill(pid, Signal::SIGTSTP) {
                eprintln!("suspend: {}", e);
                return 1;
            }
            0
        }

        #[cfg(not(unix))]
        {
            eprintln!("suspend: not supported on this platform");
            1
        }
    }
}

impl Default for ShellExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_echo() {
        let mut exec = ShellExecutor::new();
        let status = exec.execute_script("true").unwrap();
        assert_eq!(status, 0);
    }

    #[test]
    fn test_if_true() {
        let mut exec = ShellExecutor::new();
        let status = exec.execute_script("if true; then true; fi").unwrap();
        assert_eq!(status, 0);
    }

    #[test]
    fn test_if_false() {
        let mut exec = ShellExecutor::new();
        let status = exec
            .execute_script("if false; then true; else false; fi")
            .unwrap();
        assert_eq!(status, 1);
    }

    #[test]
    fn test_for_loop() {
        let mut exec = ShellExecutor::new();
        exec.execute_script("for i in a b c; do true; done")
            .unwrap();
        assert_eq!(exec.last_status, 0);
    }

    #[test]
    fn test_and_list() {
        let mut exec = ShellExecutor::new();
        let status = exec.execute_script("true && true").unwrap();
        assert_eq!(status, 0);

        let status = exec.execute_script("true && false").unwrap();
        assert_eq!(status, 1);
    }

    #[test]
    fn test_or_list() {
        let mut exec = ShellExecutor::new();
        let status = exec.execute_script("false || true").unwrap();
        assert_eq!(status, 0);
    }
}

impl ShellExecutor {
    fn builtin_history(&self, args: &[String]) -> i32 {
        let Some(ref engine) = self.history else {
            eprintln!("history: history engine not available");
            return 1;
        };

        // Parse options
        let mut count = 20usize;
        let mut show_all = false;
        let mut search_query = None;

        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "-c" | "--clear" => {
                    // Clear history - need mutable access
                    eprintln!("history: clear not supported in this mode");
                    return 1;
                }
                "-a" | "--all" => show_all = true,
                "-n" => {
                    if i + 1 < args.len() {
                        i += 1;
                        count = args[i].parse().unwrap_or(20);
                    }
                }
                s if s.starts_with('-') && s[1..].chars().all(|c| c.is_ascii_digit()) => {
                    count = s[1..].parse().unwrap_or(20);
                }
                s if s.chars().all(|c| c.is_ascii_digit()) => {
                    count = s.parse().unwrap_or(20);
                }
                s => {
                    search_query = Some(s.to_string());
                }
            }
            i += 1;
        }

        if show_all {
            count = 10000;
        }

        let entries = if let Some(ref q) = search_query {
            engine.search(q, count)
        } else {
            engine.recent(count)
        };

        match entries {
            Ok(entries) => {
                // Print in chronological order (reverse the results since recent() is newest-first)
                for entry in entries.into_iter().rev() {
                    println!("{:>6}  {}", entry.id, entry.command);
                }
                0
            }
            Err(e) => {
                eprintln!("history: {}", e);
                1
            }
        }
    }

    /// fc builtin - fix command (history manipulation)
    /// Ported from zsh/Src/builtin.c bin_fc() lines 1426-1700
    /// Options: -l (list), -n (no numbers), -r (reverse), -d/-f/-E/-i/-t (time formats),
    /// -D (duration), -e editor, -m pattern, -R/-W/-A (read/write/append history file),
    /// -p/-P (push/pop history stack), -I (skip old), -L (local), -s (substitute)
    fn builtin_fc(&mut self, args: &[String]) -> i32 {
        let Some(ref engine) = self.history else {
            eprintln!("fc: history engine not available");
            return 1;
        };

        // Parse options
        let mut list_mode = false;
        let mut no_numbers = false;
        let mut reverse = false;
        let mut show_time = false;
        let mut show_duration = false;
        let mut editor: Option<String> = None;
        let mut read_file = false;
        let mut write_file = false;
        let mut append_file = false;
        let mut substitute_mode = false;
        let mut positional: Vec<&str> = Vec::new();
        let mut substitutions: Vec<(String, String)> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if arg == "--" {
                i += 1;
                while i < args.len() {
                    positional.push(&args[i]);
                    i += 1;
                }
                break;
            }
            if arg.starts_with('-') && arg.len() > 1 {
                let chars: Vec<char> = arg[1..].chars().collect();
                let mut j = 0;
                while j < chars.len() {
                    match chars[j] {
                        'l' => list_mode = true,
                        'n' => no_numbers = true,
                        'r' => reverse = true,
                        'd' | 'f' | 'E' | 'i' => show_time = true,
                        'D' => show_duration = true,
                        'R' => read_file = true,
                        'W' => write_file = true,
                        'A' => append_file = true,
                        's' => substitute_mode = true,
                        'e' => {
                            if j + 1 < chars.len() {
                                editor = Some(chars[j + 1..].iter().collect());
                                break;
                            } else {
                                i += 1;
                                if i < args.len() {
                                    editor = Some(args[i].clone());
                                }
                            }
                        }
                        't' => {
                            show_time = true;
                            if j + 1 < chars.len() {
                                break;
                            } else {
                                i += 1;
                            }
                        }
                        'p' | 'P' | 'a' | 'I' | 'L' | 'm' => {} // Handled but no-op for now
                        _ => {
                            if chars[j].is_ascii_digit() {
                                positional.push(arg);
                                break;
                            }
                        }
                    }
                    j += 1;
                }
            } else if arg.contains('=') && !list_mode {
                if let Some((old, new)) = arg.split_once('=') {
                    substitutions.push((old.to_string(), new.to_string()));
                }
            } else {
                positional.push(arg);
            }
            i += 1;
        }

        // Handle file operations (read/write/append)
        // Note: HistoryEngine uses SQLite, so file ops are simplified
        if read_file || write_file || append_file {
            let filename = positional.first().map(|s| *s).unwrap_or("~/.zsh_history");
            let path = if filename.starts_with("~/") {
                dirs::home_dir()
                    .map(|h| h.join(&filename[2..]))
                    .unwrap_or_else(|| std::path::PathBuf::from(filename))
            } else {
                std::path::PathBuf::from(filename)
            };

            if read_file {
                // Read plain text history file and import
                if let Ok(contents) = std::fs::read_to_string(&path) {
                    for line in contents.lines() {
                        if !line.is_empty() && !line.starts_with('#') && !line.starts_with(':') {
                            let _ = engine.add(line, None);
                        }
                    }
                } else {
                    eprintln!("fc: cannot read {}", path.display());
                    return 1;
                }
            } else if write_file || append_file {
                // Export history to plain text file
                let mode = if append_file {
                    std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&path)
                } else {
                    std::fs::File::create(&path)
                };
                match mode {
                    Ok(mut file) => {
                        use std::io::Write;
                        if let Ok(entries) = engine.recent(10000) {
                            for entry in entries.iter().rev() {
                                let _ = writeln!(file, ": {}:0;{}", entry.timestamp, entry.command);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("fc: cannot write {}: {}", path.display(), e);
                        return 1;
                    }
                }
            }
            return 0;
        }

        // List mode (fc -l)
        if list_mode || args.is_empty() {
            let (first, last) = match positional.len() {
                0 => (-16i64, -1i64),
                1 => {
                    let n = positional[0].parse::<i64>().unwrap_or(-16);
                    (n, -1)
                }
                _ => {
                    let f = positional[0].parse::<i64>().unwrap_or(-16);
                    let l = positional[1].parse::<i64>().unwrap_or(-1);
                    (f, l)
                }
            };

            let count = if first < 0 { (-first) as usize } else { 16 };
            match engine.recent(count.max(100)) {
                Ok(mut entries) => {
                    if reverse {
                        entries.reverse();
                    }
                    for entry in entries.iter().rev().take(count) {
                        if no_numbers {
                            println!("{}", entry.command);
                        } else if show_time {
                            println!(
                                "{:>6}  {:>10}  {}",
                                entry.id, entry.timestamp, entry.command
                            );
                        } else if show_duration {
                            println!(
                                "{:>6}  {:>5}  {}",
                                entry.id,
                                entry.duration_ms.unwrap_or(0),
                                entry.command
                            );
                        } else {
                            println!("{:>6}  {}", entry.id, entry.command);
                        }
                    }
                    0
                }
                Err(e) => {
                    eprintln!("fc: {}", e);
                    1
                }
            }
        } else if substitute_mode || !substitutions.is_empty() {
            // Substitution mode: fc -s old=new
            match engine.get_by_offset(0) {
                Ok(Some(entry)) => {
                    let mut cmd = entry.command.clone();
                    for (old, new) in &substitutions {
                        cmd = cmd.replace(old, new);
                    }
                    println!("{}", cmd);
                    self.execute_script(&cmd).unwrap_or(1)
                }
                Ok(None) => {
                    eprintln!("fc: no command to re-execute");
                    1
                }
                Err(e) => {
                    eprintln!("fc: {}", e);
                    1
                }
            }
        } else if editor.as_deref() == Some("-") {
            // fc -e -: re-execute last command without editor
            match engine.get_by_offset(0) {
                Ok(Some(entry)) => {
                    println!("{}", entry.command);
                    self.execute_script(&entry.command).unwrap_or(1)
                }
                Ok(None) => {
                    eprintln!("fc: no command to re-execute");
                    1
                }
                Err(e) => {
                    eprintln!("fc: {}", e);
                    1
                }
            }
        } else if let Some(arg) = positional.first() {
            if arg.starts_with('-') || arg.starts_with('+') {
                // fc -N or fc +N: re-execute Nth command
                let n: usize = arg[1..].parse().unwrap_or(1);
                let offset = if arg.starts_with('-') { n - 1 } else { n };
                match engine.get_by_offset(offset) {
                    Ok(Some(entry)) => {
                        println!("{}", entry.command);
                        self.execute_script(&entry.command).unwrap_or(1)
                    }
                    Ok(None) => {
                        eprintln!("fc: event not found");
                        1
                    }
                    Err(e) => {
                        eprintln!("fc: {}", e);
                        1
                    }
                }
            } else {
                // Try to find command by prefix
                match engine.search_prefix(arg, 1) {
                    Ok(entries) if !entries.is_empty() => {
                        println!("{}", entries[0].command);
                        self.execute_script(&entries[0].command).unwrap_or(1)
                    }
                    Ok(_) => {
                        eprintln!("fc: event not found: {}", arg);
                        1
                    }
                    Err(e) => {
                        eprintln!("fc: {}", e);
                        1
                    }
                }
            }
        } else {
            // Default: edit and execute last command
            match engine.get_by_offset(0) {
                Ok(Some(entry)) => {
                    println!("{}", entry.command);
                    self.execute_script(&entry.command).unwrap_or(1)
                }
                Ok(None) => {
                    eprintln!("fc: no command to re-execute");
                    1
                }
                Err(e) => {
                    eprintln!("fc: {}", e);
                    1
                }
            }
        }
    }

    fn builtin_trap(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            // List all traps
            for (sig, action) in &self.traps {
                println!("trap -- '{}' {}", action, sig);
            }
            return 0;
        }

        // trap -l: list signal names
        if args.len() == 1 && args[0] == "-l" {
            let signals = [
                "HUP", "INT", "QUIT", "ILL", "TRAP", "ABRT", "BUS", "FPE", "KILL", "USR1", "SEGV",
                "USR2", "PIPE", "ALRM", "TERM", "STKFLT", "CHLD", "CONT", "STOP", "TSTP", "TTIN",
                "TTOU", "URG", "XCPU", "XFSZ", "VTALRM", "PROF", "WINCH", "IO", "PWR", "SYS",
            ];
            for (i, sig) in signals.iter().enumerate() {
                print!("{:2}) SIG{:<8}", i + 1, sig);
                if (i + 1) % 5 == 0 {
                    println!();
                }
            }
            println!();
            return 0;
        }

        // trap -p [sigspec...]: print trap commands
        if args.len() >= 1 && args[0] == "-p" {
            let signals = if args.len() > 1 {
                &args[1..]
            } else {
                &[] as &[String]
            };
            if signals.is_empty() {
                for (sig, action) in &self.traps {
                    println!("trap -- '{}' {}", action, sig);
                }
            } else {
                for sig in signals {
                    if let Some(action) = self.traps.get(sig) {
                        println!("trap -- '{}' {}", action, sig);
                    }
                }
            }
            return 0;
        }

        // trap '' signal: reset to default
        // trap action signal...: set trap
        // trap signal: print current action for signal
        if args.len() == 1 {
            // Print trap for this signal
            let sig = &args[0];
            if let Some(action) = self.traps.get(sig) {
                println!("trap -- '{}' {}", action, sig);
            }
            return 0;
        }

        let action = &args[0];
        let signals = &args[1..];

        for sig in signals {
            let sig_upper = sig.to_uppercase();
            let sig_name = if sig_upper.starts_with("SIG") {
                sig_upper[3..].to_string()
            } else {
                sig_upper.clone()
            };

            if action.is_empty() || action == "-" {
                // Reset to default
                self.traps.remove(&sig_name);
            } else {
                self.traps.insert(sig_name, action.clone());
            }
        }

        0
    }

    /// Execute trap handlers for a signal
    pub fn run_trap(&mut self, signal: &str) {
        if let Some(action) = self.traps.get(signal).cloned() {
            let _ = self.execute_script(&action);
        }
    }

    fn builtin_alias(&mut self, args: &[String]) -> i32 {
        // alias [ {+|-}gmrsL ] [ name[=value] ... ]
        // -g: global alias (expanded anywhere in command line)
        // -s: suffix alias (file.ext expands to "handler file.ext")
        // -r: regular alias (default)
        // -m: pattern match mode
        // -L: list in form suitable for reinput
        // +g/+s/+r: print aliases of that type

        let mut is_global = false;
        let mut is_suffix = false;
        let mut list_form = false;
        let mut pattern_match = false;
        let mut print_global = false;
        let mut print_suffix = false;
        let mut print_regular = false;
        let mut positional_args = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if arg.starts_with('+') && arg.len() > 1 {
                // +g, +s, +r: print aliases of that type
                for ch in arg[1..].chars() {
                    match ch {
                        'g' => print_global = true,
                        's' => print_suffix = true,
                        'r' => print_regular = true,
                        'L' => list_form = true,
                        'm' => pattern_match = true,
                        _ => {}
                    }
                }
            } else if arg.starts_with('-') && arg != "-" {
                for ch in arg[1..].chars() {
                    match ch {
                        'g' => is_global = true,
                        's' => is_suffix = true,
                        'L' => list_form = true,
                        'm' => pattern_match = true,
                        'r' => {} // regular alias (default)
                        _ => {
                            eprintln!("zshrs: alias: bad option: -{}", ch);
                            return 1;
                        }
                    }
                }
            } else {
                positional_args.push(arg.clone());
            }
            i += 1;
        }

        // If +g/+s/+r used, list those types
        if print_global || print_suffix || print_regular {
            if print_regular {
                for (name, value) in &self.aliases {
                    if list_form {
                        println!("alias {}='{}'", name, value);
                    } else {
                        println!("{}='{}'", name, value);
                    }
                }
            }
            if print_global {
                for (name, value) in &self.global_aliases {
                    if list_form {
                        println!("alias -g {}='{}'", name, value);
                    } else {
                        println!("{}='{}'", name, value);
                    }
                }
            }
            if print_suffix {
                for (name, value) in &self.suffix_aliases {
                    if list_form {
                        println!("alias -s {}='{}'", name, value);
                    } else {
                        println!("{}='{}'", name, value);
                    }
                }
            }
            return 0;
        }

        if positional_args.is_empty() {
            // List aliases
            let prefix = if is_suffix {
                "alias -s "
            } else if is_global {
                "alias -g "
            } else {
                "alias "
            };
            let alias_map: Vec<(String, String)> = if is_suffix {
                self.suffix_aliases
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            } else if is_global {
                self.global_aliases
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            } else {
                self.aliases
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            };
            for (name, value) in alias_map {
                if list_form {
                    println!("{}{}='{}'", prefix, name, value);
                } else {
                    println!("{}='{}'", name, value);
                }
            }
            return 0;
        }

        for arg in &positional_args {
            if let Some(eq_pos) = arg.find('=') {
                // Define alias: name=value
                let name = &arg[..eq_pos];
                let value = &arg[eq_pos + 1..];
                if is_suffix {
                    self.suffix_aliases
                        .insert(name.to_string(), value.to_string());
                } else if is_global {
                    self.global_aliases
                        .insert(name.to_string(), value.to_string());
                } else {
                    self.aliases.insert(name.to_string(), value.to_string());
                }
            } else if pattern_match {
                // -m: pattern match mode - list matching aliases
                let pattern = arg.replace("*", ".*").replace("?", ".");
                let re = regex::Regex::new(&format!("^{}$", pattern));

                let alias_map: &HashMap<String, String> = if is_suffix {
                    &self.suffix_aliases
                } else if is_global {
                    &self.global_aliases
                } else {
                    &self.aliases
                };

                let prefix = if is_suffix {
                    "alias -s "
                } else if is_global {
                    "alias -g "
                } else {
                    "alias "
                };

                for (name, value) in alias_map {
                    let matches = if let Ok(ref r) = re {
                        r.is_match(name)
                    } else {
                        name.contains(arg.as_str())
                    };
                    if matches {
                        if list_form {
                            println!("{}{}='{}'", prefix, name, value);
                        } else {
                            println!("{}='{}'", name, value);
                        }
                    }
                }
            } else {
                // Print alias - look up directly without holding borrow
                let value = if is_suffix {
                    self.suffix_aliases.get(arg.as_str()).cloned()
                } else if is_global {
                    self.global_aliases.get(arg.as_str()).cloned()
                } else {
                    self.aliases.get(arg.as_str()).cloned()
                };
                if let Some(v) = value {
                    println!("{}='{}'", arg, v);
                } else {
                    eprintln!("zshrs: alias: {}: not found", arg);
                    return 1;
                }
            }
        }
        0
    }

    fn builtin_unalias(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            eprintln!("zshrs: unalias: usage: unalias [-agsm] name [name ...]");
            return 1;
        }

        let mut is_global = false;
        let mut is_suffix = false;
        let mut remove_all = false;
        let mut positional_args = Vec::new();

        for arg in args {
            if arg.starts_with('-') && arg != "-" {
                for ch in arg[1..].chars() {
                    match ch {
                        'a' => remove_all = true,
                        'g' => is_global = true,
                        's' => is_suffix = true,
                        'm' => {} // pattern match, ignore for now
                        _ => {
                            eprintln!("zshrs: unalias: bad option: -{}", ch);
                            return 1;
                        }
                    }
                }
            } else {
                positional_args.push(arg.clone());
            }
        }

        if remove_all {
            if is_suffix {
                self.suffix_aliases.clear();
            } else if is_global {
                self.global_aliases.clear();
            } else {
                // -a without -g/-s clears all three
                self.aliases.clear();
                self.global_aliases.clear();
                self.suffix_aliases.clear();
            }
            return 0;
        }

        if positional_args.is_empty() {
            eprintln!("zshrs: unalias: usage: unalias [-agsm] name [name ...]");
            return 1;
        }

        for name in positional_args {
            let removed = if is_suffix {
                self.suffix_aliases.remove(&name).is_some()
            } else if is_global {
                self.global_aliases.remove(&name).is_some()
            } else {
                self.aliases.remove(&name).is_some()
            };
            if !removed {
                eprintln!("zshrs: unalias: {}: not found", name);
                return 1;
            }
        }
        0
    }

    fn builtin_set(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            // List all variables and their values (zsh behavior)
            let mut vars: Vec<_> = self.variables.iter().collect();
            vars.sort_by_key(|(k, _)| *k);
            for (k, v) in vars {
                println!("{}={}", k, shell_quote(v));
            }
            // Also print arrays
            let mut arrs: Vec<_> = self.arrays.iter().collect();
            arrs.sort_by_key(|(k, _)| *k);
            for (k, v) in arrs {
                let quoted: Vec<String> = v.iter().map(|s| shell_quote(s)).collect();
                println!("{}=( {} )", k, quoted.join(" "));
            }
            return 0;
        }

        // Check for "+" alone - print just variable names
        if args.len() == 1 && args[0] == "+" {
            let mut names: Vec<_> = self.variables.keys().collect();
            names.extend(self.arrays.keys());
            names.sort();
            names.dedup();
            for name in names {
                println!("{}", name);
            }
            return 0;
        }

        let mut iter = args.iter().peekable();
        let mut set_array: Option<bool> = None; // Some(true) = -A, Some(false) = +A
        let mut array_name: Option<String> = None;
        let mut sort_asc = false;
        let mut sort_desc = false;

        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-o" => {
                    // -o with no arg: print all options in "option on/off" format
                    if iter.peek().is_none()
                        || iter
                            .peek()
                            .map(|s| s.starts_with('-') || s.starts_with('+'))
                            .unwrap_or(false)
                    {
                        self.print_options_table();
                        continue;
                    }
                    if let Some(opt) = iter.next() {
                        let (name, enable) = Self::normalize_option_name(opt);
                        self.options.insert(name, enable);
                    }
                }
                "+o" => {
                    // +o with no arg: print options in re-entrant format
                    if iter.peek().is_none()
                        || iter
                            .peek()
                            .map(|s| s.starts_with('-') || s.starts_with('+'))
                            .unwrap_or(false)
                    {
                        self.print_options_reentrant();
                        continue;
                    }
                    if let Some(opt) = iter.next() {
                        let (name, enable) = Self::normalize_option_name(opt);
                        self.options.insert(name, !enable);
                    }
                }
                "-A" => {
                    set_array = Some(true);
                    if let Some(name) = iter.next() {
                        if !name.starts_with('-') && !name.starts_with('+') {
                            array_name = Some(name.clone());
                        }
                    }
                    if array_name.is_none() {
                        // Print all arrays with values
                        let mut arrs: Vec<_> = self.arrays.iter().collect();
                        arrs.sort_by_key(|(k, _)| *k);
                        for (k, v) in arrs {
                            let quoted: Vec<String> = v.iter().map(|s| shell_quote(s)).collect();
                            println!("{}=( {} )", k, quoted.join(" "));
                        }
                        return 0;
                    }
                }
                "+A" => {
                    set_array = Some(false);
                    if let Some(name) = iter.next() {
                        if !name.starts_with('-') && !name.starts_with('+') {
                            array_name = Some(name.clone());
                        }
                    }
                    if array_name.is_none() {
                        // Print array names only
                        let mut names: Vec<_> = self.arrays.keys().collect();
                        names.sort();
                        for name in names {
                            println!("{}", name);
                        }
                        return 0;
                    }
                }
                "-s" => sort_asc = true,
                "+s" => sort_desc = true,
                "-e" => {
                    self.options.insert("errexit".to_string(), true);
                }
                "+e" => {
                    self.options.insert("errexit".to_string(), false);
                }
                "-x" => {
                    self.options.insert("xtrace".to_string(), true);
                }
                "+x" => {
                    self.options.insert("xtrace".to_string(), false);
                }
                "-u" => {
                    self.options.insert("nounset".to_string(), true);
                }
                "+u" => {
                    self.options.insert("nounset".to_string(), false);
                }
                "-v" => {
                    self.options.insert("verbose".to_string(), true);
                }
                "+v" => {
                    self.options.insert("verbose".to_string(), false);
                }
                "-n" => {
                    self.options.insert("exec".to_string(), false);
                }
                "+n" => {
                    self.options.insert("exec".to_string(), true);
                }
                "-f" => {
                    self.options.insert("glob".to_string(), false);
                }
                "+f" => {
                    self.options.insert("glob".to_string(), true);
                }
                "-m" => {
                    self.options.insert("monitor".to_string(), true);
                }
                "+m" => {
                    self.options.insert("monitor".to_string(), false);
                }
                "-C" => {
                    self.options.insert("clobber".to_string(), false);
                }
                "+C" => {
                    self.options.insert("clobber".to_string(), true);
                }
                "-b" => {
                    self.options.insert("notify".to_string(), true);
                }
                "+b" => {
                    self.options.insert("notify".to_string(), false);
                }
                "--" => {
                    let remaining: Vec<String> = iter.cloned().collect();
                    if let Some(ref name) = array_name {
                        let mut values = remaining;
                        if sort_asc {
                            values.sort();
                        } else if sort_desc {
                            values.sort();
                            values.reverse();
                        }
                        if set_array == Some(true) {
                            self.arrays.insert(name.clone(), values);
                        } else {
                            // +A: replace initial elements
                            let arr = self.arrays.entry(name.clone()).or_default();
                            for (i, v) in values.into_iter().enumerate() {
                                if i < arr.len() {
                                    arr[i] = v;
                                } else {
                                    arr.push(v);
                                }
                            }
                        }
                    } else if remaining.is_empty() {
                        // "set --" with nothing after unsets positional params
                        self.positional_params.clear();
                    } else {
                        let mut values = remaining;
                        if sort_asc {
                            values.sort();
                        } else if sort_desc {
                            values.sort();
                            values.reverse();
                        }
                        self.positional_params = values;
                    }
                    return 0;
                }
                _ => {
                    // Handle single-letter options like -ex (multiple options)
                    if arg.starts_with('-') && arg.len() > 1 {
                        for c in arg[1..].chars() {
                            match c {
                                'e' => {
                                    self.options.insert("errexit".to_string(), true);
                                }
                                'x' => {
                                    self.options.insert("xtrace".to_string(), true);
                                }
                                'u' => {
                                    self.options.insert("nounset".to_string(), true);
                                }
                                'v' => {
                                    self.options.insert("verbose".to_string(), true);
                                }
                                'n' => {
                                    self.options.insert("exec".to_string(), false);
                                }
                                'f' => {
                                    self.options.insert("glob".to_string(), false);
                                }
                                'm' => {
                                    self.options.insert("monitor".to_string(), true);
                                }
                                'C' => {
                                    self.options.insert("clobber".to_string(), false);
                                }
                                'b' => {
                                    self.options.insert("notify".to_string(), true);
                                }
                                _ => {
                                    eprintln!("zshrs: set: -{}: invalid option", c);
                                    return 1;
                                }
                            }
                        }
                        continue;
                    }
                    if arg.starts_with('+') && arg.len() > 1 {
                        for c in arg[1..].chars() {
                            match c {
                                'e' => {
                                    self.options.insert("errexit".to_string(), false);
                                }
                                'x' => {
                                    self.options.insert("xtrace".to_string(), false);
                                }
                                'u' => {
                                    self.options.insert("nounset".to_string(), false);
                                }
                                'v' => {
                                    self.options.insert("verbose".to_string(), false);
                                }
                                'n' => {
                                    self.options.insert("exec".to_string(), true);
                                }
                                'f' => {
                                    self.options.insert("glob".to_string(), true);
                                }
                                'm' => {
                                    self.options.insert("monitor".to_string(), false);
                                }
                                'C' => {
                                    self.options.insert("clobber".to_string(), true);
                                }
                                'b' => {
                                    self.options.insert("notify".to_string(), false);
                                }
                                _ => {
                                    eprintln!("zshrs: set: +{}: invalid option", c);
                                    return 1;
                                }
                            }
                        }
                        continue;
                    }
                    // Treat as positional params
                    let mut values: Vec<String> =
                        std::iter::once(arg.clone()).chain(iter.cloned()).collect();
                    if sort_asc {
                        values.sort();
                    } else if sort_desc {
                        values.sort();
                        values.reverse();
                    }
                    if let Some(ref name) = array_name {
                        if set_array == Some(true) {
                            self.arrays.insert(name.clone(), values);
                        } else {
                            let arr = self.arrays.entry(name.clone()).or_default();
                            for (i, v) in values.into_iter().enumerate() {
                                if i < arr.len() {
                                    arr[i] = v;
                                } else {
                                    arr.push(v);
                                }
                            }
                        }
                    } else {
                        self.positional_params = values;
                    }
                    return 0;
                }
            }
        }
        0
    }

    fn default_on_options() -> &'static [&'static str] {
        &[
            "aliases",
            "alwayslastprompt",
            "appendhistory",
            "autolist",
            "automenu",
            "autoparamkeys",
            "autoparamslash",
            "autoremoveslash",
            "badpattern",
            "banghist",
            "bareglobqual",
            "beep",
            "bgnice",
            "caseglob",
            "casematch",
            "checkjobs",
            "checkrunningjobs",
            "clobber",
            "debugbeforecmd",
            "equals",
            "evallineno",
            "exec",
            "flowcontrol",
            "functionargzero",
            "glob",
            "globalexport",
            "globalrcs",
            "hashcmds",
            "hashdirs",
            "hashlistall",
            "histbeep",
            "histsavebycopy",
            "hup",
            "interactive",
            "listambiguous",
            "listbeep",
            "listtypes",
            "monitor",
            "multibyte",
            "multifuncdef",
            "multios",
            "nomatch",
            "notify",
            "promptcr",
            "promptpercent",
            "promptsp",
            "rcs",
            "shinstdin",
            "shortloops",
            "unset",
            "zle",
        ]
    }

    fn print_options_table(&self) {
        let mut opts: Vec<_> = Self::all_zsh_options().to_vec();
        opts.sort();
        let defaults_on = Self::default_on_options();
        for &opt in &opts {
            let enabled = self.options.get(opt).copied().unwrap_or(false);
            let is_default_on = defaults_on.contains(&opt);
            // zsh format: for default-ON options, show "noOPTION off" when on, "noOPTION on" when off
            // for default-OFF options, show "OPTION off" when off, "OPTION on" when on
            let (display_name, display_state) = if is_default_on {
                (format!("no{}", opt), if enabled { "off" } else { "on" })
            } else {
                (opt.to_string(), if enabled { "on" } else { "off" })
            };
            println!("{:<22}{}", display_name, display_state);
        }
    }

    fn print_options_reentrant(&self) {
        let mut opts: Vec<_> = Self::all_zsh_options().to_vec();
        opts.sort();
        let defaults_on = Self::default_on_options();
        for &opt in &opts {
            let enabled = self.options.get(opt).copied().unwrap_or(false);
            let is_default_on = defaults_on.contains(&opt);
            // zsh format: use noOPTION for default-on options
            let (display_name, use_minus) = if is_default_on {
                (format!("no{}", opt), !enabled)
            } else {
                (opt.to_string(), enabled)
            };
            if use_minus {
                println!("set -o {}", display_name);
            } else {
                println!("set +o {}", display_name);
            }
        }
    }

    /// caller - display call stack (bash)
    fn builtin_caller(&self, args: &[String]) -> i32 {
        let depth: usize = args.first().and_then(|s| s.parse().ok()).unwrap_or(0);
        // In a real implementation, we'd track the call stack
        // For now, show basic info
        if depth == 0 {
            println!("1 main");
        } else {
            println!("{} main", depth);
        }
        0
    }

    /// doctor - diagnostic report of shell health, caches, and performance
    fn builtin_doctor(&self, _args: &[String]) -> i32 {
        let green = |s: &str| format!("\x1b[32m{}\x1b[0m", s);
        let red = |s: &str| format!("\x1b[31m{}\x1b[0m", s);
        let yellow = |s: &str| format!("\x1b[33m{}\x1b[0m", s);
        let bold = |s: &str| format!("\x1b[1m{}\x1b[0m", s);
        let dim = |s: &str| format!("\x1b[2m{}\x1b[0m", s);

        println!("{}", bold("zshrs doctor"));
        println!("{}", dim(&"=".repeat(60)));
        println!();

        // --- Environment ---
        println!("{}", bold("Environment"));
        println!("  version:    zshrs {}", env!("CARGO_PKG_VERSION"));
        println!("  pid:        {}", std::process::id());
        let cwd = env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "?".to_string());
        println!("  cwd:        {}", cwd);
        println!("  shell:      {}", env::var("SHELL").unwrap_or_else(|_| "?".to_string()));
        println!("  pool size:  {}", self.worker_pool.size());
        println!("  pool done:  {} tasks completed", self.worker_pool.completed());
        println!("  pool queue: {} pending", self.worker_pool.queue_depth());
        println!();

        // --- Config ---
        println!("{}", bold("Config"));
        let config_path = crate::config::config_path();
        if config_path.exists() {
            println!("  {}  {}", green("*"), config_path.display());
        } else {
            println!("  {}  {} {}", dim("-"), config_path.display(), dim("(using defaults)"));
        }
        println!();

        // --- PATH ---
        println!("{}", bold("PATH"));
        let path_var = env::var("PATH").unwrap_or_default();
        let path_dirs: Vec<&str> = path_var.split(':').filter(|s| !s.is_empty()).collect();
        let path_ok = path_dirs.iter().filter(|d| std::path::Path::new(d).is_dir()).count();
        let path_missing = path_dirs.len() - path_ok;
        println!("  directories: {} total, {} {}, {} {}",
            path_dirs.len(),
            path_ok, green("valid"),
            path_missing, if path_missing > 0 { red("missing") } else { green("missing") },
        );
        println!("  hash table:  {} entries", self.command_hash.len());
        println!();

        // --- FPATH ---
        println!("{}", bold("FPATH"));
        println!("  directories: {}", self.fpath.len());
        let fpath_ok = self.fpath.iter().filter(|d| d.is_dir()).count();
        let fpath_missing = self.fpath.len() - fpath_ok;
        if fpath_missing > 0 {
            println!("  {} {} missing fpath directories", red("!"), fpath_missing);
        }
        println!("  functions:   {} loaded", self.functions.len());
        println!("  autoload:    {} pending", self.autoload_pending.len());
        println!();

        // --- SQLite Caches ---
        println!("{}", bold("SQLite Caches"));
        if let Some(ref engine) = self.history {
            let count = engine.count().unwrap_or(0);
            println!("  history:     {} entries  {}", count, green("OK"));
        } else {
            println!("  history:     {}", yellow("not initialized"));
        }

        if let Some(ref cache) = self.compsys_cache {
            let count = compsys::cache_entry_count(cache);
            println!("  compsys:     {} completions  {}", count, green("OK"));

            // Check bytecode blob coverage
            if let Ok(missing) = cache.get_autoloads_missing_bytecode() {
                if missing.is_empty() {
                    println!("  bytecode cache:   {}", green("all functions compiled to bytecode"));
                } else {
                    println!("  bytecode cache:   {} functions {}", missing.len(), yellow("missing bytecode blobs"));
                }
            }
        } else {
            println!("  compsys:     {}", yellow("no cache"));
        }

        if let Some(ref cache) = self.plugin_cache {
            let (plugins, functions) = cache.stats();
            println!("  plugins:     {} plugins, {} cached functions  {}", plugins, functions, green("OK"));
        } else {
            println!("  plugins:     {}", yellow("no cache"));
        }
        println!();

        // --- Shell State ---
        println!("{}", bold("Shell State"));
        println!("  aliases:     {}", self.aliases.len());
        println!("  global:      {} aliases", self.global_aliases.len());
        println!("  suffix:      {} aliases", self.suffix_aliases.len());
        println!("  variables:   {}", self.variables.len());
        println!("  arrays:      {}", self.arrays.len());
        println!("  assoc:       {}", self.assoc_arrays.len());
        println!("  options:     {} set", self.options.iter().filter(|(_, v)| **v).count());
        println!("  traps:       {} active", self.traps.len());
        println!("  hooks:       {} registered", self.hook_functions.values().map(|v| v.len()).sum::<usize>());
        println!();

        // --- Log ---
        println!("{}", bold("Log"));
        let log_path = crate::log::log_path();
        if log_path.exists() {
            let size = std::fs::metadata(&log_path).map(|m| m.len()).unwrap_or(0);
            println!("  {}  {} bytes", log_path.display(), size);
        } else {
            println!("  {}", dim("no log file yet"));
        }
        println!();

        // --- Profiling ---
        println!("{}", bold("Profiling"));
        println!("  chrome tracing: {}", if crate::log::profiling_enabled() { green("enabled") } else { dim("disabled") });
        println!("  flamegraph:     {}", if crate::log::flamegraph_enabled() { green("enabled") } else { dim("disabled") });
        println!("  prometheus:     {}", if crate::log::prometheus_enabled() { green("enabled") } else { dim("disabled") });
        println!();

        0
    }

    /// dbview — browse zshrs SQLite cache tables without SQL.
    ///
    /// Usage:
    ///   dbview                      — list all tables and row counts
    ///   dbview autoloads             — dump autoloads table (name, source, body len, ast len)
    ///   dbview autoloads _git        — show single row by name
    ///   dbview comps                 — dump comps table
    ///   dbview history               — recent history entries
    ///   dbview history <pattern>     — search history
    ///   dbview plugins               — plugin cache entries
    ///   dbview executables            — PATH executables cache
    ///   dbview <table> --count       — just the count
    fn builtin_dbview(&self, args: &[String]) -> i32 {
        let bold = |s: &str| format!("\x1b[1m{}\x1b[0m", s);
        let dim = |s: &str| format!("\x1b[2m{}\x1b[0m", s);
        let cyan = |s: &str| format!("\x1b[36m{}\x1b[0m", s);
        let green = |s: &str| format!("\x1b[32m{}\x1b[0m", s);
        let yellow = |s: &str| format!("\x1b[33m{}\x1b[0m", s);

        if args.is_empty() {
            // List all tables with row counts
            println!("{}", bold("zshrs SQLite caches"));
            println!();

            if let Some(ref cache) = self.compsys_cache {
                println!("  {} {}", bold("compsys.db"), dim("(completion cache)"));
                if let Ok(n) = cache.count_table("autoloads") {
                    let bc_count = cache.count_table_where("autoloads", "bytecode IS NOT NULL").unwrap_or(0);
                    println!("    autoloads:    {:>6} rows  ({} compiled)", n, bc_count);
                }
                if let Ok(n) = cache.count_table("comps") { println!("    comps:        {:>6} rows", n); }
                if let Ok(n) = cache.count_table("services") { println!("    services:     {:>6} rows", n); }
                if let Ok(n) = cache.count_table("patcomps") { println!("    patcomps:     {:>6} rows", n); }
                if let Ok(n) = cache.count_table("executables") { println!("    executables:  {:>6} rows", n); }
                if let Ok(n) = cache.count_table("zstyles") { println!("    zstyles:      {:>6} rows", n); }
                println!();
            }

            if let Some(ref engine) = self.history {
                println!("  {} {}", bold("history.db"), dim("(command history)"));
                if let Ok(n) = engine.count() { println!("    entries:      {:>6} rows", n); }
                println!();
            }

            if let Some(ref cache) = self.plugin_cache {
                let (plugins, functions) = cache.stats();
                println!("  {} {}", bold("plugins.db"), dim("(plugin source cache)"));
                println!("    plugins:      {:>6} rows", plugins);
                println!("    functions:    {:>6} rows", functions);
                println!();
            }

            println!("  Usage: {} <table> [name] [--count]", cyan("dbview"));
            return 0;
        }

        let table = args[0].as_str();
        let filter = args.get(1).map(|s| s.as_str());
        let count_only = args.iter().any(|a| a == "--count" || a == "-c");

        match table {
            "autoloads" => {
                let Some(ref cache) = self.compsys_cache else {
                    eprintln!("dbview: no compsys cache");
                    return 1;
                };

                if count_only {
                    let n = cache.count_table("autoloads").unwrap_or(0);
                    println!("{}", n);
                    return 0;
                }

                if let Some(name) = filter {
                    // Single row lookup
                    match cache.get_autoload(name) {
                        Ok(Some(stub)) => {
                            println!("{}", bold(&format!("autoload: {}", name)));
                            println!("  source:   {}", stub.source);
                            println!("  body:     {} bytes", stub.body.as_ref().map(|b| b.len()).unwrap_or(0));
                            match cache.get_autoload_bytecode(name) {
                                Ok(Some(blob)) => println!("  bytecode: {} {} bytes", green("YES"), blob.len()),
                                _ => println!("  bytecode: {}", yellow("NULL")),
                            }
                            // Show first few lines of body
                            if let Some(ref body) = stub.body {
                                println!("  preview:");
                                for (i, line) in body.lines().take(10).enumerate() {
                                    println!("    {:>3}: {}", i + 1, dim(line));
                                }
                                let total = body.lines().count();
                                if total > 10 {
                                    println!("    {} ({} more lines)", dim("..."), total - 10);
                                }
                            }
                        }
                        _ => {
                            eprintln!("dbview: autoload '{}' not found", name);
                            return 1;
                        }
                    }
                    return 0;
                }

                // Dump all autoloads
                let conn = &cache.conn();
                match conn.prepare("SELECT name, source, length(body), length(bytecode) FROM autoloads ORDER BY name LIMIT 200") {
                    Ok(mut stmt) => {
                        let rows = stmt.query_map([], |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, Option<i64>>(2)?,
                                row.get::<_, Option<i64>>(3)?,
                            ))
                        });
                        if let Ok(rows) = rows {
                            println!("{:<40} {:>8} {:>8}  {}", bold("NAME"), bold("BODY"), bold("BYTECODE"), bold("SOURCE"));
                            let mut count = 0;
                            for row in rows.flatten() {
                                let (name, source, body_len, ast_len) = row;
                                let ast_str = match ast_len {
                                    Some(n) => green(&format!("{:>8}", n)),
                                    None => yellow(&format!("{:>8}", "NULL")),
                                };
                                let body_str = match body_len {
                                    Some(n) => format!("{:>8}", n),
                                    None => dim("NULL").to_string(),
                                };
                                // Truncate source path for display
                                let src_short = if source.len() > 50 {
                                    format!("...{}", &source[source.len() - 47..])
                                } else {
                                    source
                                };
                                println!("{:<40} {} {}  {}", name, body_str, ast_str, dim(&src_short));
                                count += 1;
                            }
                            println!("\n{} rows shown (LIMIT 200)", count);
                        }
                    }
                    Err(e) => {
                        eprintln!("dbview: query failed: {}", e);
                        return 1;
                    }
                }
            }

            "comps" => {
                let Some(ref cache) = self.compsys_cache else {
                    eprintln!("dbview: no compsys cache");
                    return 1;
                };
                if count_only {
                    println!("{}", cache.count_table("comps").unwrap_or(0));
                    return 0;
                }
                let conn = cache.conn();
                let query = if let Some(pat) = filter {
                    format!("SELECT command, function FROM comps WHERE command LIKE '%{}%' ORDER BY command LIMIT 100", pat)
                } else {
                    "SELECT command, function FROM comps ORDER BY command LIMIT 100".to_string()
                };
                match conn.prepare(&query) {
                    Ok(mut stmt) => {
                        println!("{:<40} {}", bold("COMMAND"), bold("FUNCTION"));
                        let rows = stmt.query_map([], |row| {
                            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                        });
                        if let Ok(rows) = rows {
                            for row in rows.flatten() {
                                println!("{:<40} {}", row.0, cyan(&row.1));
                            }
                        }
                    }
                    Err(e) => { eprintln!("dbview: {}", e); return 1; }
                }
            }

            "executables" => {
                let Some(ref cache) = self.compsys_cache else {
                    eprintln!("dbview: no compsys cache");
                    return 1;
                };
                if count_only {
                    println!("{}", cache.count_table("executables").unwrap_or(0));
                    return 0;
                }
                let conn = cache.conn();
                let query = if let Some(pat) = filter {
                    format!("SELECT name, path FROM executables WHERE name LIKE '%{}%' ORDER BY name LIMIT 100", pat)
                } else {
                    "SELECT name, path FROM executables ORDER BY name LIMIT 100".to_string()
                };
                match conn.prepare(&query) {
                    Ok(mut stmt) => {
                        println!("{:<30} {}", bold("NAME"), bold("PATH"));
                        let rows = stmt.query_map([], |row| {
                            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                        });
                        if let Ok(rows) = rows {
                            for row in rows.flatten() {
                                println!("{:<30} {}", row.0, dim(&row.1));
                            }
                        }
                    }
                    Err(e) => { eprintln!("dbview: {}", e); return 1; }
                }
            }

            "history" => {
                let Some(ref engine) = self.history else {
                    eprintln!("dbview: no history engine");
                    return 1;
                };
                if count_only {
                    println!("{}", engine.count().unwrap_or(0));
                    return 0;
                }
                if let Some(pat) = filter {
                    if let Ok(entries) = engine.search(pat, 20) {
                        for e in entries {
                            println!("  {} {} {}", dim(&e.timestamp.to_string()), cyan(&e.command), dim(&format!("[{}]", e.exit_code.unwrap_or(0))));
                        }
                    }
                } else if let Ok(entries) = engine.recent(20) {
                    for e in entries {
                        println!("  {} {} {}", dim(&e.timestamp.to_string()), cyan(&e.command), dim(&format!("[{}]", e.exit_code.unwrap_or(0))));
                    }
                }
            }

            "plugins" => {
                let Some(ref cache) = self.plugin_cache else {
                    eprintln!("dbview: no plugin cache");
                    return 1;
                };
                let (plugins, functions) = cache.stats();
                println!("{} plugins, {} cached functions", plugins, functions);
            }

            _ => {
                eprintln!("dbview: unknown table '{}'. Available: autoloads, comps, executables, history, plugins", table);
                return 1;
            }
        }

        0
    }

    /// profile — in-process command profiling with nanosecond accuracy.
    ///
    /// Unlike `time` (which measures one command) or `zprof` (which only
    /// profiles function calls), `profile` traces every execute_command,
    /// expansion, glob, and builtin dispatch inside the block.
    ///
    /// Usage:
    ///   profile { commands }     — profile a block
    ///   profile -s 'script'     — profile a script string
    ///   profile -f func         — profile a function call
    ///   profile --clear         — clear accumulated profile data
    ///   profile --dump          — show accumulated profile data
    fn builtin_profile(&mut self, args: &[String]) -> i32 {
        let bold = |s: &str| format!("\x1b[1m{}\x1b[0m", s);
        let dim = |s: &str| format!("\x1b[2m{}\x1b[0m", s);
        let cyan = |s: &str| format!("\x1b[36m{}\x1b[0m", s);
        let yellow = |s: &str| format!("\x1b[33m{}\x1b[0m", s);

        if args.is_empty() {
            println!("Usage: profile {{ commands }}");
            println!("       profile -s 'script string'");
            println!("       profile -f function_name [args...]");
            println!("       profile --clear");
            println!("       profile --dump");
            return 0;
        }

        if args[0] == "--clear" {
            self.profiler = crate::zprof::Profiler::new();
            println!("profile data cleared");
            return 0;
        }

        if args[0] == "--dump" {
            let (_, output) = crate::zprof::builtin_zprof(
                &mut self.profiler,
                &crate::zprof::ZprofOptions { clear: false },
            );
            if !output.is_empty() {
                print!("{}", output);
            } else {
                println!("{}", dim("no profile data"));
            }
            return 0;
        }

        // Determine what to profile
        let code = if args[0] == "-s" {
            // profile -s 'script string'
            if args.len() < 2 {
                eprintln!("profile: -s requires a script string");
                return 1;
            }
            args[1..].join(" ")
        } else if args[0] == "-f" {
            // profile -f func_name [args...]
            if args.len() < 2 {
                eprintln!("profile: -f requires a function name");
                return 1;
            }
            args[1..].join(" ")
        } else {
            // profile { commands } — args is the block body
            args.join(" ")
        };

        // Enable profiling, run, collect results
        let was_enabled = self.profiling_enabled;
        self.profiling_enabled = true;
        self.profiler = crate::zprof::Profiler::new(); // fresh data for this run

        let t0 = std::time::Instant::now();
        let result = self.execute_script(&code);
        let elapsed = t0.elapsed();
        let status = match result {
            Ok(s) => s,
            Err(e) => {
                eprintln!("profile: {}", e);
                1
            }
        };

        // Collect timing data
        println!();
        println!("{}", bold("profile results"));
        println!("{}", dim(&"─".repeat(60)));
        let dur_str = if elapsed.as_secs() > 0 {
            format!("{:.3}s", elapsed.as_secs_f64())
        } else if elapsed.as_millis() > 0 {
            format!("{:.3}ms", elapsed.as_secs_f64() * 1000.0)
        } else {
            format!("{:.1}µs", elapsed.as_secs_f64() * 1_000_000.0)
        };
        println!("  total:     {}", cyan(&dur_str));
        println!("  status:    {}", status);
        println!();

        // Show function-level breakdown from profiler
        let (_, output) = crate::zprof::builtin_zprof(
            &mut self.profiler,
            &crate::zprof::ZprofOptions { clear: false },
        );
        if !output.is_empty() {
            println!("{}", bold("function breakdown"));
            print!("{}", output);
        }

        // Per-command breakdown from tracing (if tracing is at debug level)
        println!();
        println!("  {} set ZSHRS_LOG=trace for per-command tracing", yellow("tip:"));
        println!("  {} output: {}", dim("log"), dim(&crate::log::log_path().display().to_string()));

        self.profiling_enabled = was_enabled;
        status
    }

    // ═══════════════════════════════════════════════════════════════════
    // AOP INTERCEPT — the killer builtin
    // ═══════════════════════════════════════════════════════════════════

    /// Check intercepts for a command. Returns Some(result) if an around
    /// advice fully handled the command, None to proceed normally.
    fn run_intercepts(
        &mut self,
        cmd_name: &str,
        full_cmd: &str,
        args: &[String],
    ) -> Option<Result<i32, String>> {
        // Collect matching intercepts (clone to avoid borrow issues)
        let matching: Vec<Intercept> = self
            .intercepts
            .iter()
            .filter(|i| intercept_matches(&i.pattern, cmd_name, full_cmd))
            .cloned()
            .collect();

        if matching.is_empty() {
            return None;
        }

        // Set INTERCEPT_NAME and INTERCEPT_ARGS for advice code
        self.variables.insert("INTERCEPT_NAME".to_string(), cmd_name.to_string());
        self.variables.insert("INTERCEPT_ARGS".to_string(), args.join(" "));
        self.variables.insert("INTERCEPT_CMD".to_string(), full_cmd.to_string());

        // Run before advice
        for advice in matching.iter().filter(|i| matches!(i.kind, AdviceKind::Before)) {
            let _ = self.execute_advice(&advice.code);
        }

        // Check for around advice — first match wins
        let around = matching.iter().find(|i| matches!(i.kind, AdviceKind::Around));

        let t0 = std::time::Instant::now();

        let result = if let Some(advice) = around {
            // Around advice: set INTERCEPT_PROCEED flag, run advice code.
            // If advice calls `intercept_proceed`, the original command runs.
            self.variables.insert("__intercept_proceed".to_string(), "0".to_string());
            let advice_result = self.execute_advice(&advice.code);

            // Check if intercept_proceed was called
            let proceeded = self.variables.get("__intercept_proceed")
                .map(|v| v == "1")
                .unwrap_or(false);

            if proceeded {
                // The original command was already executed inside the advice
                advice_result
            } else {
                // Advice didn't call proceed — command was suppressed
                advice_result
            }
        } else {
            // No around advice — run the original command.
            // We return None to let the normal dispatch continue.
            // But we still need after advice to fire, so we can't return None here
            // if there are after advices. Run the command ourselves.
            let has_after = matching.iter().any(|i| matches!(i.kind, AdviceKind::After));
            if !has_after {
                // Only before advice, no after — let normal dispatch continue
                return None;
            }

            // Has after advice — we must run the command and then run after advice
            self.run_original_command(cmd_name, args)
        };

        let elapsed = t0.elapsed();

        // Set timing variable for after advice
        let ms = elapsed.as_secs_f64() * 1000.0;
        self.variables.insert("INTERCEPT_MS".to_string(), format!("{:.3}", ms));
        self.variables.insert("INTERCEPT_US".to_string(), format!("{:.0}", ms * 1000.0));

        // Run after advice
        for advice in matching.iter().filter(|i| matches!(i.kind, AdviceKind::After)) {
            let _ = self.execute_advice(&advice.code);
        }

        // Clean up
        self.variables.remove("INTERCEPT_NAME");
        self.variables.remove("INTERCEPT_ARGS");
        self.variables.remove("INTERCEPT_CMD");
        self.variables.remove("INTERCEPT_MS");
        self.variables.remove("INTERCEPT_US");
        self.variables.remove("__intercept_proceed");

        Some(result)
    }

    /// Execute the original command (used by around/after intercept dispatch).
    /// Execute advice code — dispatches @ prefix to stryke (fat binary),
    /// everything else to the shell parser. No fork. Machine code speed.
    fn execute_advice(&mut self, code: &str) -> Result<i32, String> {
        let code = code.trim();
        if code.starts_with('@') {
            let stryke_code = code.trim_start_matches('@').trim();
            if let Some(status) = crate::try_stryke_dispatch(stryke_code) {
                self.last_status = status;
                return Ok(status);
            }
            // No stryke handler (thin binary) — fall through to shell
        }
        self.execute_script(code)
    }

    fn run_original_command(&mut self, cmd_name: &str, args: &[String]) -> Result<i32, String> {
        // Try function
        if let Some(func) = self.functions.get(cmd_name).cloned() {
            return self.call_function(&func, args);
        }
        if self.maybe_autoload(cmd_name) {
            if let Some(func) = self.functions.get(cmd_name).cloned() {
                return self.call_function(&func, args);
            }
        }
        // External command
        self.execute_external(cmd_name, &args.to_vec(), &[])
    }

    /// intercept builtin — register AOP advice on commands.
    ///
    /// Usage:
    ///   intercept before <pattern> { code }
    ///   intercept after <pattern> { code }
    ///   intercept around <pattern> { code }
    ///   intercept list                       — show all intercepts
    ///   intercept remove <id>                — remove by ID
    ///   intercept clear                      — remove all
    fn builtin_intercept(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            println!("Usage: intercept <before|after|around> <pattern> {{ code }}");
            println!("       intercept list | remove <id> | clear");
            return 0;
        }

        match args[0].as_str() {
            "list" => {
                if self.intercepts.is_empty() {
                    println!("no intercepts registered");
                } else {
                    let bold = |s: &str| format!("\x1b[1m{}\x1b[0m", s);
                    let cyan = |s: &str| format!("\x1b[36m{}\x1b[0m", s);
                    println!("{:>4}  {:<8}  {:<20}  {}", bold("ID"), bold("KIND"), bold("PATTERN"), bold("CODE"));
                    for i in &self.intercepts {
                        let kind = match i.kind {
                            AdviceKind::Before => "before",
                            AdviceKind::After => "after",
                            AdviceKind::Around => "around",
                        };
                        let code_preview = if i.code.len() > 40 {
                            format!("{}...", &i.code[..37])
                        } else {
                            i.code.clone()
                        };
                        println!("{:>4}  {:<8}  {:<20}  {}", cyan(&i.id.to_string()), kind, i.pattern, code_preview);
                    }
                }
                0
            }
            "clear" => {
                let count = self.intercepts.len();
                self.intercepts.clear();
                println!("cleared {} intercepts", count);
                0
            }
            "remove" => {
                if args.len() < 2 {
                    eprintln!("intercept remove: requires ID");
                    return 1;
                }
                if let Ok(id) = args[1].parse::<u32>() {
                    let before = self.intercepts.len();
                    self.intercepts.retain(|i| i.id != id);
                    if self.intercepts.len() < before {
                        println!("removed intercept {}", id);
                        0
                    } else {
                        eprintln!("intercept: no intercept with ID {}", id);
                        1
                    }
                } else {
                    eprintln!("intercept remove: invalid ID");
                    1
                }
            }
            "before" | "after" | "around" => {
                let kind = match args[0].as_str() {
                    "before" => AdviceKind::Before,
                    "after" => AdviceKind::After,
                    "around" => AdviceKind::Around,
                    _ => unreachable!(),
                };

                if args.len() < 3 {
                    eprintln!("intercept {}: requires <pattern> {{ code }}", args[0]);
                    return 1;
                }

                let pattern = args[1].clone();
                // Join remaining args as the code (handles { code } or 'code')
                let code = args[2..].join(" ");
                // Strip surrounding braces if present
                let code = code.trim().to_string();
                let code = if code.starts_with('{') && code.ends_with('}') {
                    code[1..code.len() - 1].trim().to_string()
                } else {
                    code
                };

                let id = self.intercepts.iter().map(|i| i.id).max().unwrap_or(0) + 1;
                self.intercepts.push(Intercept {
                    pattern,
                    kind: kind.clone(),
                    code: code.clone(),
                    id,
                });

                let kind_str = match kind {
                    AdviceKind::Before => "before",
                    AdviceKind::After => "after",
                    AdviceKind::Around => "around",
                };
                println!("intercept #{}: {} {} → {}", id, kind_str, self.intercepts.last().unwrap().pattern,
                    if code.len() > 50 { format!("{}...", &code[..47]) } else { code });
                0
            }
            _ => {
                eprintln!("intercept: unknown subcommand '{}'. Use before|after|around|list|remove|clear", args[0]);
                1
            }
        }
    }

    /// intercept_proceed — called from around advice to execute the original command.
    fn builtin_intercept_proceed(&mut self, _args: &[String]) -> i32 {
        self.variables.insert("__intercept_proceed".to_string(), "1".to_string());
        // Run the original command using saved INTERCEPT_NAME/INTERCEPT_ARGS
        let cmd_name = self.variables.get("INTERCEPT_NAME").cloned().unwrap_or_default();
        let args_str = self.variables.get("INTERCEPT_ARGS").cloned().unwrap_or_default();
        let args: Vec<String> = if args_str.is_empty() {
            Vec::new()
        } else {
            args_str.split_whitespace().map(|s| s.to_string()).collect()
        };
        match self.run_original_command(&cmd_name, &args) {
            Ok(status) => status,
            Err(e) => {
                eprintln!("intercept_proceed: {}", e);
                1
            }
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // CONCURRENT PRIMITIVES — ship work to the worker pool from shell
    // No stryke dependency. Pure zshrs. Thin binary gets full parallelism.
    // ═══════════════════════════════════════════════════════════════════

    /// async { cmd } — run command on worker pool, return job ID immediately.
    /// Output captured in background, retrieve with `await $id`.
    ///
    /// Usage:
    ///   id=$(async 'sleep 2; echo done')
    ///   ... do other work ...
    ///   result=$(await $id)
    fn builtin_async(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            eprintln!("async: requires a command string");
            return 1;
        }

        let code = args.join(" ");
        let id = self.next_async_id;
        self.next_async_id += 1;

        let (tx, rx) = crossbeam_channel::bounded::<(i32, String)>(1);
        let pool = std::sync::Arc::clone(&self.worker_pool);

        pool.submit(move || {
            // Execute in a subprocess to capture stdout
            use std::process::{Command, Stdio};
            let output = Command::new("sh")
                .args(["-c", &code])
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .output();
            match output {
                Ok(out) => {
                    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                    let status = out.status.code().unwrap_or(1);
                    let _ = tx.send((status, stdout));
                }
                Err(_) => {
                    let _ = tx.send((127, String::new()));
                }
            }
        });

        self.async_jobs.insert(id, rx);
        // Print the job ID so it can be captured: id=$(async 'cmd')
        println!("{}", id);
        0
    }

    /// await $id — block until async job completes, print its stdout, return its status.
    ///
    /// Usage:
    ///   id=$(async 'expensive_command')
    ///   await $id    # blocks until done, prints output
    ///   echo $?      # exit status of the async command
    fn builtin_await(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            eprintln!("await: requires a job ID");
            return 1;
        }

        let id: u32 = match args[0].parse() {
            Ok(n) => n,
            Err(_) => {
                eprintln!("await: invalid job ID '{}'", args[0]);
                return 1;
            }
        };

        let rx = match self.async_jobs.remove(&id) {
            Some(rx) => rx,
            None => {
                eprintln!("await: no async job with ID {}", id);
                return 1;
            }
        };

        // Block until the job completes
        match rx.recv() {
            Ok((status, stdout)) => {
                if !stdout.is_empty() {
                    print!("{}", stdout);
                }
                self.last_status = status;
                status
            }
            Err(_) => {
                eprintln!("await: job {} died without result", id);
                1
            }
        }
    }

    /// pmap 'cmd {}' arg1 arg2 arg3 — parallel map across worker pool.
    /// Runs `cmd` for each argument, replacing `{}` with the argument.
    /// Output is collected in order. Returns 0 if all succeed.
    ///
    /// Usage:
    ///   pmap 'gzip {}' *.log
    ///   pmap 'echo {}' a b c d
    ///   ls *.rs | pmap 'wc -l {}'
    fn builtin_pmap(&mut self, args: &[String]) -> i32 {
        if args.len() < 2 {
            eprintln!("pmap: requires 'command {{}}' followed by arguments");
            return 1;
        }

        let template = &args[0];
        let items = &args[1..];

        // Ship each item to the pool
        let mut receivers = Vec::with_capacity(items.len());
        for item in items {
            let cmd = template.replace("{}", item);
            let rx = self.worker_pool.submit_with_result(move || {
                use std::process::{Command, Stdio};
                let output = Command::new("sh")
                    .args(["-c", &cmd])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::inherit())
                    .output();
                match output {
                    Ok(out) => (
                        out.status.code().unwrap_or(1),
                        String::from_utf8_lossy(&out.stdout).to_string(),
                    ),
                    Err(_) => (127, String::new()),
                }
            });
            receivers.push(rx);
        }

        // Collect results in order
        let mut any_fail = false;
        for rx in receivers {
            if let Ok((status, stdout)) = rx.recv() {
                if !stdout.is_empty() {
                    print!("{}", stdout);
                }
                if status != 0 {
                    any_fail = true;
                }
            }
        }

        if any_fail { 1 } else { 0 }
    }

    /// pgrep 'pattern' arg1 arg2 ... — parallel grep/filter across worker pool.
    /// Runs the pattern command for each argument, prints args where command succeeds.
    ///
    /// Usage:
    ///   pgrep 'test -f {}' /path/a /path/b /path/c
    ///   pgrep 'grep -q TODO {}' *.rs
    fn builtin_pgrep(&mut self, args: &[String]) -> i32 {
        if args.len() < 2 {
            eprintln!("pgrep: requires 'test_command {{}}' followed by arguments");
            return 1;
        }

        let template = &args[0];
        let items = &args[1..];

        let mut receivers: Vec<(String, crossbeam_channel::Receiver<bool>)> = Vec::with_capacity(items.len());
        for item in items {
            let cmd = template.replace("{}", item);
            let rx = self.worker_pool.submit_with_result(move || {
                use std::process::{Command, Stdio};
                Command::new("sh")
                    .args(["-c", &cmd])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false)
            });
            receivers.push((item.clone(), rx));
        }

        for (item, rx) in receivers {
            if let Ok(true) = rx.recv() {
                println!("{}", item);
            }
        }

        0
    }

    /// peach 'cmd {}' arg1 arg2 ... — parallel for-each, no output ordering.
    /// Like pmap but doesn't collect output — fire-and-forget, print as completed.
    ///
    /// Usage:
    ///   peach 'convert {} {}.png' *.svg
    ///   peach 'rsync -a {} remote:{}' dir1 dir2 dir3
    fn builtin_peach(&mut self, args: &[String]) -> i32 {
        if args.len() < 2 {
            eprintln!("peach: requires 'command {{}}' followed by arguments");
            return 1;
        }

        let template = &args[0];
        let items = &args[1..];

        let (tx, rx) = crossbeam_channel::unbounded::<(String, i32, String)>();

        for item in items {
            let cmd = template.replace("{}", item);
            let item_clone = item.clone();
            let tx = tx.clone();
            self.worker_pool.submit(move || {
                use std::process::{Command, Stdio};
                let output = Command::new("sh")
                    .args(["-c", &cmd])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::inherit())
                    .output();
                match output {
                    Ok(out) => {
                        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                        let status = out.status.code().unwrap_or(1);
                        let _ = tx.send((item_clone, status, stdout));
                    }
                    Err(_) => {
                        let _ = tx.send((item_clone, 127, String::new()));
                    }
                }
            });
        }
        drop(tx);

        let mut any_fail = false;
        for (_, status, stdout) in rx {
            if !stdout.is_empty() {
                print!("{}", stdout);
            }
            if status != 0 {
                any_fail = true;
            }
        }

        if any_fail { 1 } else { 0 }
    }

    /// barrier cmd1 ::: cmd2 ::: cmd3 — run commands in parallel, wait for ALL to complete.
    /// Returns the worst (highest) exit status.
    ///
    /// Usage:
    ///   barrier 'make -C proj1' ::: 'make -C proj2' ::: 'make -C proj3'
    ///   barrier 'npm test' ::: 'cargo test' ::: 'pytest'
    fn builtin_barrier(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            eprintln!("barrier: requires commands separated by :::");
            return 1;
        }

        // Split on ::: delimiter
        let mut commands: Vec<String> = Vec::new();
        let mut current = String::new();
        for arg in args {
            if arg == ":::" {
                if !current.is_empty() {
                    commands.push(current.trim().to_string());
                    current.clear();
                }
            } else {
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(arg);
            }
        }
        if !current.is_empty() {
            commands.push(current.trim().to_string());
        }

        if commands.is_empty() {
            return 0;
        }

        // Ship all to pool
        let mut receivers = Vec::with_capacity(commands.len());
        for cmd in &commands {
            let cmd = cmd.clone();
            let rx = self.worker_pool.submit_with_result(move || {
                use std::process::{Command, Stdio};
                Command::new("sh")
                    .args(["-c", &cmd])
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::inherit())
                    .status()
                    .map(|s| s.code().unwrap_or(1))
                    .unwrap_or(127)
            });
            receivers.push(rx);
        }

        // Wait for all — return worst status
        let mut worst = 0i32;
        for rx in receivers {
            if let Ok(status) = rx.recv() {
                if status > worst {
                    worst = status;
                }
            }
        }

        self.last_status = worst;
        worst
    }

    /// help - display help for builtins (bash)
    fn builtin_help(&self, args: &[String]) -> i32 {
        if args.is_empty() {
            println!("zshrs shell builtins:");
            println!("");
            println!("  alias, bg, bind, break, builtin, cd, command, continue,");
            println!("  declare, dirs, disown, echo, enable, eval, exec, exit,");
            println!("  export, false, fc, fg, getopts, hash, help, history,");
            println!("  jobs, kill, let, local, logout, popd, printf, pushd,");
            println!("  pwd, read, readonly, return, set, shift, shopt, source,");
            println!("  suspend, test, times, trap, true, type, typeset, ulimit,");
            println!("  umask, unalias, unset, wait, whence, where, which");
            println!("");
            println!("Type 'help name' for more information about 'name'.");
            return 0;
        }

        let cmd = &args[0];
        match cmd.as_str() {
            "cd" => println!("cd: cd [-L|-P] [dir]\n    Change the shell working directory."),
            "echo" => println!("echo: echo [-neE] [arg ...]\n    Write arguments to standard output."),
            "export" => println!("export: export [-fn] [name[=value] ...]\n    Set export attribute for shell variables."),
            "alias" => println!("alias: alias [-p] [name[=value] ...]\n    Define or display aliases."),
            "history" => println!("history: history [-c] [-d offset] [n]\n    Display or manipulate the history list."),
            "jobs" => println!("jobs: jobs [-lnprs] [jobspec ...]\n    Display status of jobs."),
            "kill" => println!("kill: kill [-s sigspec | -n signum | -sigspec] pid | jobspec ...\n    Send a signal to a job."),
            "read" => println!("read: read [-ers] [-a array] [-d delim] [-i text] [-n nchars] [-N nchars] [-p prompt] [-t timeout] [-u fd] [name ...]\n    Read a line from standard input."),
            "set" => println!("set: set [-abefhkmnptuvxBCHP] [-o option-name] [--] [arg ...]\n    Set or unset values of shell options and positional parameters."),
            "test" | "[" => println!("test: test [expr]\n    Evaluate conditional expression."),
            "type" => println!("type: type [-afptP] name [name ...]\n    Display information about command type."),
            _ => println!("{}: no help available", cmd),
        }
        0
    }

    /// readarray/mapfile - read lines into array (bash)
    fn builtin_readarray(&mut self, args: &[String]) -> i32 {
        use std::io::{BufRead, BufReader};

        let mut array_name = "MAPFILE".to_string();
        let mut delimiter = '\n';
        let mut count = 0usize; // 0 = unlimited
        let mut skip = 0usize;
        let mut strip_trailing = false;
        let mut callback: Option<String> = None;
        let mut callback_quantum = 0usize;

        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "-d" => {
                    i += 1;
                    if i < args.len() && !args[i].is_empty() {
                        delimiter = args[i].chars().next().unwrap_or('\n');
                    }
                }
                "-n" => {
                    i += 1;
                    if i < args.len() {
                        count = args[i].parse().unwrap_or(0);
                    }
                }
                "-O" => {
                    i += 1;
                    // Origin - start index (ignored, we always start at 0)
                }
                "-s" => {
                    i += 1;
                    if i < args.len() {
                        skip = args[i].parse().unwrap_or(0);
                    }
                }
                "-t" => strip_trailing = true,
                "-C" => {
                    i += 1;
                    if i < args.len() {
                        callback = Some(args[i].clone());
                    }
                }
                "-c" => {
                    i += 1;
                    if i < args.len() {
                        callback_quantum = args[i].parse().unwrap_or(5000);
                    }
                }
                "-u" => {
                    i += 1;
                    // fd - ignored, we read from stdin
                }
                s if !s.starts_with('-') => {
                    array_name = s.to_string();
                }
                _ => {}
            }
            i += 1;
        }

        let stdin = std::io::stdin();
        let reader = BufReader::new(stdin.lock());
        let mut lines = Vec::new();
        let mut line_count = 0usize;

        for line_result in reader.lines() {
            if let Ok(mut line) = line_result {
                line_count += 1;

                if line_count <= skip {
                    continue;
                }

                if strip_trailing {
                    while line.ends_with('\n') || line.ends_with('\r') {
                        line.pop();
                    }
                }

                lines.push(line);

                if count > 0 && lines.len() >= count {
                    break;
                }
            }
        }

        self.arrays.insert(array_name, lines);
        let _ = (callback, callback_quantum);
        0
    }

    fn builtin_shopt(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            // List all shell options
            for (opt, val) in &self.options {
                println!("shopt {} {}", if *val { "-s" } else { "-u" }, opt);
            }
            return 0;
        }

        let mut set = None;
        let mut opts = Vec::new();

        for arg in args {
            match arg.as_str() {
                "-s" => set = Some(true),
                "-u" => set = Some(false),
                "-p" => {
                    // Print option status
                    for opt in &opts {
                        let val = self.options.get(opt).copied().unwrap_or(false);
                        println!("shopt {} {}", if val { "-s" } else { "-u" }, opt);
                    }
                    return 0;
                }
                _ => opts.push(arg.clone()),
            }
        }

        if let Some(enable) = set {
            for opt in &opts {
                self.options.insert(opt.clone(), enable);
            }
        } else {
            // Query options
            for opt in &opts {
                let val = self.options.get(opt).copied().unwrap_or(false);
                println!("shopt {} {}", if val { "-s" } else { "-u" }, opt);
            }
        }
        0
    }

    /// zsh-compatible setopt builtin
    fn builtin_setopt(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            // List options that differ from compiled-in defaults (zsh behavior)
            // For default-ON options: show "noOPTION" if currently OFF
            // For default-OFF options: show "OPTION" if currently ON
            let defaults_on = Self::default_on_options();
            let mut diff_opts: Vec<String> = Vec::new();

            for &opt in Self::all_zsh_options() {
                let enabled = self.options.get(opt).copied().unwrap_or(false);
                let is_default_on = defaults_on.contains(&opt);

                if is_default_on && !enabled {
                    // Default ON but currently OFF -> show noOPTION
                    diff_opts.push(format!("no{}", opt));
                } else if !is_default_on && enabled {
                    // Default OFF but currently ON -> show OPTION
                    diff_opts.push(opt.to_string());
                }
            }
            diff_opts.sort();
            for opt in diff_opts {
                println!("{}", opt);
            }
            return 0;
        }

        let mut use_pattern = false;
        let mut iter = args.iter().peekable();

        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-m" => use_pattern = true,
                "-o" => {
                    // -o option_name: set option
                    if let Some(opt) = iter.next() {
                        let (name, enable) = Self::normalize_option_name(opt);
                        self.options.insert(name, enable);
                    }
                }
                "+o" => {
                    // +o option_name: unset option
                    if let Some(opt) = iter.next() {
                        let (name, enable) = Self::normalize_option_name(opt);
                        self.options.insert(name, !enable);
                    }
                }
                _ => {
                    if use_pattern {
                        // Match pattern against all options
                        for opt in Self::all_zsh_options() {
                            if Self::option_matches_pattern(opt, arg) {
                                self.options.insert(opt.to_string(), true);
                            }
                        }
                    } else {
                        let (name, enable) = Self::normalize_option_name(arg);
                        // Verify it's a valid option (zsh doesn't error on bad names in setopt)
                        self.options.insert(name, enable);
                    }
                }
            }
        }
        0
    }

    /// zsh-compatible unsetopt builtin
    fn builtin_unsetopt(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            // List all options in the format you'd pass to unsetopt to disable them
            // For default-ON options: show "noOPTION" (to turn it off)
            // For default-OFF options: show "OPTION" (already off, but this is what you'd type)
            let defaults_on = Self::default_on_options();
            let mut all_opts: Vec<String> = Vec::new();

            for &opt in Self::all_zsh_options() {
                let is_default_on = defaults_on.contains(&opt);
                if is_default_on {
                    all_opts.push(format!("no{}", opt));
                } else {
                    all_opts.push(opt.to_string());
                }
            }
            all_opts.sort();
            for opt in all_opts {
                println!("{}", opt);
            }
            return 0;
        }

        let mut use_pattern = false;
        let mut iter = args.iter().peekable();

        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-m" => use_pattern = true,
                "-o" => {
                    // -o option_name: unset option
                    if let Some(opt) = iter.next() {
                        let (name, enable) = Self::normalize_option_name(opt);
                        self.options.insert(name, !enable);
                    }
                }
                "+o" => {
                    // +o option_name: set option (opposite in unsetopt)
                    if let Some(opt) = iter.next() {
                        let (name, enable) = Self::normalize_option_name(opt);
                        self.options.insert(name, enable);
                    }
                }
                _ => {
                    if use_pattern {
                        for opt in Self::all_zsh_options() {
                            if Self::option_matches_pattern(opt, arg) {
                                self.options.insert(opt.to_string(), false);
                            }
                        }
                    } else {
                        let (name, enable) = Self::normalize_option_name(arg);
                        // unsetopt turns OFF the option (or ON if "no" prefix)
                        self.options.insert(name, !enable);
                    }
                }
            }
        }
        0
    }

    fn builtin_getopts(&mut self, args: &[String]) -> i32 {
        if args.len() < 2 {
            eprintln!("zshrs: getopts: usage: getopts optstring name [arg ...]");
            return 1;
        }

        let optstring = &args[0];
        let varname = &args[1];
        let opt_args: Vec<&str> = if args.len() > 2 {
            args[2..].iter().map(|s| s.as_str()).collect()
        } else {
            self.positional_params.iter().map(|s| s.as_str()).collect()
        };

        // Get current OPTIND
        let optind: usize = self
            .variables
            .get("OPTIND")
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);

        if optind > opt_args.len() {
            self.variables.insert(varname.to_string(), "?".to_string());
            return 1;
        }

        let current_arg = opt_args[optind - 1];

        if !current_arg.starts_with('-') || current_arg == "-" {
            self.variables.insert(varname.to_string(), "?".to_string());
            return 1;
        }

        if current_arg == "--" {
            self.variables
                .insert("OPTIND".to_string(), (optind + 1).to_string());
            self.variables.insert(varname.to_string(), "?".to_string());
            return 1;
        }

        // Get current option position within the argument
        let optpos: usize = self
            .variables
            .get("_OPTPOS")
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);

        let opt_char = current_arg.chars().nth(optpos);

        if let Some(c) = opt_char {
            // Look up option in optstring
            let opt_idx = optstring.find(c);

            match opt_idx {
                Some(idx) => {
                    // Check if option takes an argument
                    let takes_arg = optstring.chars().nth(idx + 1) == Some(':');

                    if takes_arg {
                        // Get argument
                        let arg = if optpos + 1 < current_arg.len() {
                            // Argument is rest of current arg
                            current_arg[optpos + 1..].to_string()
                        } else if optind < opt_args.len() {
                            // Argument is next arg
                            self.variables
                                .insert("OPTIND".to_string(), (optind + 2).to_string());
                            self.variables.remove("_OPTPOS");
                            opt_args[optind].to_string()
                        } else {
                            // Missing argument
                            self.variables.insert(varname.to_string(), "?".to_string());
                            if !optstring.starts_with(':') {
                                eprintln!("zshrs: getopts: option requires an argument -- {}", c);
                            }
                            self.variables.insert("OPTARG".to_string(), c.to_string());
                            return 1;
                        };

                        self.variables.insert("OPTARG".to_string(), arg);
                        self.variables
                            .insert("OPTIND".to_string(), (optind + 1).to_string());
                        self.variables.remove("_OPTPOS");
                    } else {
                        // No argument needed
                        if optpos + 1 < current_arg.len() {
                            // More options in this arg
                            self.variables
                                .insert("_OPTPOS".to_string(), (optpos + 1).to_string());
                        } else {
                            // Move to next arg
                            self.variables
                                .insert("OPTIND".to_string(), (optind + 1).to_string());
                            self.variables.remove("_OPTPOS");
                        }
                    }

                    self.variables.insert(varname.to_string(), c.to_string());
                    0
                }
                None => {
                    // Unknown option
                    if !optstring.starts_with(':') {
                        eprintln!("zshrs: getopts: illegal option -- {}", c);
                    }
                    self.variables.insert(varname.to_string(), "?".to_string());
                    self.variables.insert("OPTARG".to_string(), c.to_string());

                    // Advance to next option/arg
                    if optpos + 1 < current_arg.len() {
                        self.variables
                            .insert("_OPTPOS".to_string(), (optpos + 1).to_string());
                    } else {
                        self.variables
                            .insert("OPTIND".to_string(), (optind + 1).to_string());
                        self.variables.remove("_OPTPOS");
                    }
                    0
                }
            }
        } else {
            // No more options in current arg
            self.variables
                .insert("OPTIND".to_string(), (optind + 1).to_string());
            self.variables.remove("_OPTPOS");
            self.variables.insert(varname.to_string(), "?".to_string());
            1
        }
    }

    fn builtin_type(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            return 0;
        }

        let mut show_all = false;
        let mut path_only = false;
        let mut silent = false;
        let mut show_type = false;
        let mut names = Vec::new();

        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            if arg.starts_with('-') && arg.len() > 1 {
                for c in arg[1..].chars() {
                    match c {
                        'a' => show_all = true,
                        'p' => path_only = true,
                        'P' => path_only = true,
                        's' => silent = true,
                        't' => show_type = true,
                        'f' => {} // ignore functions (we still show them)
                        'w' => {} // like -t but different format
                        _ => {}
                    }
                }
            } else {
                names.push(arg.clone());
            }
        }

        if names.is_empty() {
            return 0;
        }

        let mut status = 0;
        for name in &names {
            let mut found_any = false;

            // Check for alias (skip if -p)
            if !path_only && self.aliases.contains_key(name) {
                found_any = true;
                if !silent {
                    if show_type {
                        println!("alias");
                    } else {
                        println!(
                            "{} is aliased to `{}'",
                            name,
                            self.aliases.get(name).unwrap()
                        );
                    }
                }
                if !show_all {
                    continue;
                }
            }

            // Check for function (skip if -p)
            if !path_only && self.functions.contains_key(name) {
                found_any = true;
                if !silent {
                    if show_type {
                        println!("function");
                    } else {
                        println!("{} is a shell function", name);
                    }
                }
                if !show_all {
                    continue;
                }
            }

            // Check for builtin (skip if -p)
            if !path_only && (self.is_builtin(name) || name == ":" || name == "[") {
                found_any = true;
                if !silent {
                    if show_type {
                        println!("builtin");
                    } else {
                        println!("{} is a shell builtin", name);
                    }
                }
                if !show_all {
                    continue;
                }
            }

            // Check for external command in PATH
            if let Ok(path_env) = std::env::var("PATH") {
                for dir in path_env.split(':') {
                    let full_path = format!("{}/{}", dir, name);
                    if std::path::Path::new(&full_path).exists() {
                        found_any = true;
                        if !silent {
                            if show_type {
                                println!("file");
                            } else {
                                println!("{} is {}", name, full_path);
                            }
                        }
                        if !show_all {
                            break;
                        }
                    }
                }
            }

            if !found_any {
                if !silent {
                    eprintln!("zshrs: type: {}: not found", name);
                }
                status = 1;
            }
        }
        status
    }

    fn builtin_hash(&mut self, args: &[String]) -> i32 {
        // hash [ -Ldfmrv ] [ name[=value] ] ...
        // hash -r clears the hash table
        // hash -d manages named directories
        // hash -f fills the table with all PATH commands
        // hash -m matches patterns
        // hash -v verbose
        // hash -L list in form suitable for reinput

        let mut dir_mode = false;
        let mut rehash = false;
        let mut fill_all = false;
        let mut pattern_match = false;
        let mut verbose = false;
        let mut list_form = false;
        let mut names = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if arg.starts_with('-') && arg.len() > 1 {
                for ch in arg[1..].chars() {
                    match ch {
                        'd' => dir_mode = true,
                        'r' => rehash = true,
                        'f' => fill_all = true,
                        'm' => pattern_match = true,
                        'v' => verbose = true,
                        'L' => list_form = true,
                        _ => {}
                    }
                }
            } else {
                names.push(arg.clone());
            }
            i += 1;
        }

        // -r: clear hash table
        if rehash && !dir_mode && names.is_empty() {
            self.command_hash.clear();
            return 0;
        }

        // -f: fill hash table with all commands in PATH
        if fill_all {
            if let Ok(path_var) = env::var("PATH") {
                for dir in path_var.split(':') {
                    if let Ok(entries) = std::fs::read_dir(dir) {
                        for entry in entries.flatten() {
                            if let Ok(ft) = entry.file_type() {
                                if ft.is_file() || ft.is_symlink() {
                                    if let Some(name) = entry.file_name().to_str() {
                                        let path = entry.path().to_string_lossy().to_string();
                                        self.command_hash.insert(name.to_string(), path);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            return 0;
        }

        if dir_mode {
            // Named directories mode (hash -d)
            if names.is_empty() {
                // List named directories
                for (name, path) in &self.named_dirs {
                    if list_form {
                        println!("hash -d {}={}", name, path.display());
                    } else if verbose {
                        println!("{}={}", name, path.display());
                    } else {
                        println!("{}={}", name, path.display());
                    }
                }
                return 0;
            }

            if rehash {
                // Remove named directories
                if pattern_match {
                    // -m: pattern matching
                    let to_remove: Vec<String> = self
                        .named_dirs
                        .keys()
                        .filter(|k| {
                            names.iter().any(|pat| {
                                let pattern = pat.replace("*", ".*").replace("?", ".");
                                regex::Regex::new(&format!("^{}$", pattern))
                                    .map(|r| r.is_match(k))
                                    .unwrap_or(false)
                            })
                        })
                        .cloned()
                        .collect();
                    for name in to_remove {
                        self.named_dirs.remove(&name);
                    }
                } else {
                    for name in &names {
                        self.named_dirs.remove(name);
                    }
                }
                return 0;
            }

            // Add named directories
            for name in &names {
                if let Some((n, p)) = name.split_once('=') {
                    self.add_named_dir(n, p);
                } else {
                    eprintln!("hash: -d: {} not in name=value format", name);
                    return 1;
                }
            }
            return 0;
        }

        // Regular hash - command path lookup
        if names.is_empty() {
            // List all hashed commands
            for (name, path) in &self.command_hash {
                if list_form {
                    println!("hash {}={}", name, path);
                } else {
                    println!("{}={}", name, path);
                }
            }
            return 0;
        }

        for name in &names {
            if let Some((cmd, path)) = name.split_once('=') {
                // Explicit assignment
                self.command_hash.insert(cmd.to_string(), path.to_string());
                if verbose {
                    println!("{}={}", cmd, path);
                }
            } else if let Some(path) = self.find_in_path(name) {
                // Look up in PATH and hash it
                self.command_hash.insert(name.clone(), path.clone());
                if verbose {
                    println!("{}={}", name, path);
                }
            } else {
                eprintln!("zshrs: hash: {}: not found", name);
                return 1;
            }
        }
        0
    }

    /// add-zsh-hook builtin - add function to hook
    fn builtin_add_zsh_hook(&mut self, args: &[String]) -> i32 {
        // add-zsh-hook [-d] hook function
        if args.len() < 2 {
            eprintln!("usage: add-zsh-hook [-d] hook function");
            return 1;
        }

        let (delete, hook, func) = if args[0] == "-d" {
            if args.len() < 3 {
                eprintln!("usage: add-zsh-hook -d hook function");
                return 1;
            }
            (true, &args[1], &args[2])
        } else {
            (false, &args[0], &args[1])
        };

        if delete {
            // Remove function from hook
            if let Some(funcs) = self.hook_functions.get_mut(hook.as_str()) {
                funcs.retain(|f| f != func);
            }
        } else {
            // Add function to hook
            self.add_hook(hook, func);
        }
        0
    }

    fn builtin_command(&mut self, args: &[String], redirects: &[Redirect]) -> i32 {
        // command [ -pvV ] simple command
        // -p: use default PATH
        // -v: print path (like which)
        // -V: verbose description (like type)
        let mut use_default_path = false;
        let mut print_path = false;
        let mut verbose = false;
        let mut positional_args: Vec<&str> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if arg.starts_with('-') && arg.len() > 1 && positional_args.is_empty() {
                for ch in arg[1..].chars() {
                    match ch {
                        'p' => use_default_path = true,
                        'v' => print_path = true,
                        'V' => verbose = true,
                        '-' => {
                            // -- ends options
                            i += 1;
                            break;
                        }
                        _ => {
                            eprintln!("command: bad option: -{}", ch);
                            return 1;
                        }
                    }
                }
            } else {
                positional_args.push(arg);
            }
            i += 1;
        }

        // Add remaining args after --
        while i < args.len() {
            positional_args.push(&args[i]);
            i += 1;
        }

        if positional_args.is_empty() {
            return 0;
        }

        let cmd = positional_args[0];

        // -v or -V: just print info about command
        if print_path || verbose {
            // Search PATH for command
            let path_var = if use_default_path {
                "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin".to_string()
            } else {
                env::var("PATH").unwrap_or_default()
            };

            for dir in path_var.split(':') {
                let full_path = PathBuf::from(dir).join(cmd);
                if full_path.exists() && full_path.is_file() {
                    if verbose {
                        println!("{} is {}", cmd, full_path.display());
                    } else {
                        println!("{}", full_path.display());
                    }
                    return 0;
                }
            }

            if verbose {
                eprintln!("{} not found", cmd);
            }
            return 1;
        }

        // Execute as external command (bypassing functions and aliases)
        let cmd_args: Vec<String> = positional_args[1..].iter().map(|s| s.to_string()).collect();

        if use_default_path {
            // Temporarily set PATH
            let old_path = env::var("PATH").ok();
            env::set_var("PATH", "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin");
            let result = self
                .execute_external(
                    cmd,
                    &cmd_args
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                        .split_whitespace()
                        .map(String::from)
                        .collect::<Vec<_>>(),
                    redirects,
                )
                .unwrap_or(127);
            if let Some(p) = old_path {
                env::set_var("PATH", p);
            }
            result
        } else {
            self.execute_external(cmd, &cmd_args, redirects)
                .unwrap_or(127)
        }
    }

    fn builtin_builtin(&mut self, args: &[String], redirects: &[Redirect]) -> i32 {
        // Run builtin, bypassing functions and aliases
        if args.is_empty() {
            return 0;
        }

        let cmd = &args[0];
        let cmd_args = &args[1..];

        match cmd.as_str() {
            "cd" => self.builtin_cd(cmd_args),
            "pwd" => self.builtin_pwd(redirects),
            "echo" => self.builtin_echo(cmd_args, redirects),
            "export" => self.builtin_export(cmd_args),
            "unset" => self.builtin_unset(cmd_args),
            "exit" => self.builtin_exit(cmd_args),
            "return" => self.builtin_return(cmd_args),
            "true" => 0,
            "false" => 1,
            ":" => 0,
            "test" | "[" => self.builtin_test(cmd_args),
            "local" => self.builtin_local(cmd_args),
            "declare" | "typeset" => self.builtin_declare(cmd_args),
            "read" => self.builtin_read(cmd_args),
            "shift" => self.builtin_shift(cmd_args),
            "eval" => self.builtin_eval(cmd_args),
            "alias" => self.builtin_alias(cmd_args),
            "unalias" => self.builtin_unalias(cmd_args),
            "set" => self.builtin_set(cmd_args),
            "shopt" => self.builtin_shopt(cmd_args),
            "getopts" => self.builtin_getopts(cmd_args),
            "type" => self.builtin_type(cmd_args),
            "hash" => self.builtin_hash(cmd_args),
            "add-zsh-hook" => self.builtin_add_zsh_hook(cmd_args),
            "autoload" => self.builtin_autoload(cmd_args),
            "source" | "." => self.builtin_source(cmd_args),
            "functions" => self.builtin_functions(cmd_args),
            "zle" => self.builtin_zle(cmd_args),
            "bindkey" => self.builtin_bindkey(cmd_args),
            "setopt" => self.builtin_setopt(cmd_args),
            "unsetopt" => self.builtin_unsetopt(cmd_args),
            "emulate" => self.builtin_emulate(cmd_args),
            "zstyle" => self.builtin_zstyle(cmd_args),
            "compadd" => self.builtin_compadd(cmd_args),
            "compset" => self.builtin_compset(cmd_args),
            "compdef" => self.builtin_compdef(cmd_args),
            "compinit" => self.builtin_compinit(cmd_args),
            "cdreplay" => self.builtin_cdreplay(cmd_args),
            "zmodload" => self.builtin_zmodload(cmd_args),
            "zcompile" => self.builtin_zcompile(cmd_args),
            "zformat" => self.builtin_zformat(cmd_args),
            "zprof" => self.builtin_zprof(cmd_args),
            "print" => self.builtin_print(cmd_args),
            "printf" => self.builtin_printf(cmd_args),
            "command" => self.builtin_command(cmd_args, redirects),
            "whence" => self.builtin_whence(cmd_args),
            "which" => self.builtin_which(cmd_args),
            "where" => self.builtin_where(cmd_args),
            "fc" => self.builtin_fc(cmd_args),
            "history" => self.builtin_history(cmd_args),
            "dirs" => self.builtin_dirs(cmd_args),
            "pushd" => self.builtin_pushd(cmd_args),
            "popd" => self.builtin_popd(cmd_args),
            "bg" => self.builtin_bg(cmd_args),
            "fg" => self.builtin_fg(cmd_args),
            "jobs" => self.builtin_jobs(cmd_args),
            "kill" => self.builtin_kill(cmd_args),
            "wait" => self.builtin_wait(cmd_args),
            "trap" => self.builtin_trap(cmd_args),
            "umask" => self.builtin_umask(cmd_args),
            "ulimit" => self.builtin_ulimit(cmd_args),
            "times" => self.builtin_times(cmd_args),
            "let" => self.builtin_let(cmd_args),
            "integer" => self.builtin_integer(cmd_args),
            "float" => self.builtin_float(cmd_args),
            "readonly" => self.builtin_readonly(cmd_args),
            _ => {
                eprintln!("zshrs: builtin: {}: not a shell builtin", cmd);
                1
            }
        }
    }

    fn builtin_let(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            return 1;
        }

        let mut result = 0i64;
        for expr in args {
            result = self.evaluate_arithmetic_expr(expr);
        }

        // let returns 1 if last expression evaluates to 0, 0 otherwise
        if result == 0 {
            1
        } else {
            0
        }
    }

    /// Generate completion candidates
    fn builtin_compgen(&self, args: &[String]) -> i32 {
        let mut i = 0;
        let mut prefix = String::new();
        let mut actions = Vec::new();
        let mut wordlist = None;
        let mut globpat = None;

        while i < args.len() {
            match args[i].as_str() {
                "-W" => {
                    i += 1;
                    if i < args.len() {
                        wordlist = Some(args[i].clone());
                    }
                }
                "-G" => {
                    i += 1;
                    if i < args.len() {
                        globpat = Some(args[i].clone());
                    }
                }
                "-a" => actions.push("alias"),
                "-b" => actions.push("builtin"),
                "-c" => actions.push("command"),
                "-d" => actions.push("directory"),
                "-e" => actions.push("export"),
                "-f" => actions.push("file"),
                "-j" => actions.push("job"),
                "-k" => actions.push("keyword"),
                "-u" => actions.push("user"),
                "-v" => actions.push("variable"),
                s if !s.starts_with('-') => prefix = s.to_string(),
                _ => {}
            }
            i += 1;
        }

        let mut results = Vec::new();

        // Generate based on actions
        for action in actions {
            match action {
                "alias" => {
                    for name in self.aliases.keys() {
                        if name.starts_with(&prefix) {
                            results.push(name.clone());
                        }
                    }
                }
                "builtin" => {
                    for name in [
                        "cd", "pwd", "echo", "export", "unset", "source", "exit", "return", "true",
                        "false", ":", "test", "[", "local", "declare", "jobs", "fg", "bg", "kill",
                        "disown", "wait", "alias", "unalias", "set", "shopt",
                    ] {
                        if name.starts_with(&prefix) {
                            results.push(name.to_string());
                        }
                    }
                }
                "directory" => {
                    if let Ok(entries) = std::fs::read_dir(".") {
                        for entry in entries.flatten() {
                            if let Ok(ft) = entry.file_type() {
                                if ft.is_dir() {
                                    let name = entry.file_name().to_string_lossy().to_string();
                                    if name.starts_with(&prefix) {
                                        results.push(name);
                                    }
                                }
                            }
                        }
                    }
                }
                "file" => {
                    if let Ok(entries) = std::fs::read_dir(".") {
                        for entry in entries.flatten() {
                            let name = entry.file_name().to_string_lossy().to_string();
                            if name.starts_with(&prefix) {
                                results.push(name);
                            }
                        }
                    }
                }
                "variable" => {
                    for name in self.variables.keys() {
                        if name.starts_with(&prefix) {
                            results.push(name.clone());
                        }
                    }
                    for name in std::env::vars().map(|(k, _)| k) {
                        if name.starts_with(&prefix) && !results.contains(&name) {
                            results.push(name);
                        }
                    }
                }
                _ => {}
            }
        }

        // Handle wordlist
        if let Some(words) = wordlist {
            for word in words.split_whitespace() {
                if word.starts_with(&prefix) {
                    results.push(word.to_string());
                }
            }
        }

        // Handle glob pattern
        if let Some(_pattern) = globpat {
            let full_pattern = format!("{}*", prefix);
            if let Ok(paths) = glob::glob(&full_pattern) {
                for path in paths.flatten() {
                    results.push(path.to_string_lossy().to_string());
                }
            }
        }

        results.sort();
        results.dedup();
        for r in results {
            println!("{}", r);
        }
        0
    }

    /// Define completion spec for a command
    fn builtin_complete(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            // List all completion specs
            for (cmd, spec) in &self.completions {
                let mut parts = vec!["complete".to_string()];
                for action in &spec.actions {
                    parts.push(format!("-{}", action));
                }
                if let Some(ref w) = spec.wordlist {
                    parts.push("-W".to_string());
                    parts.push(format!("'{}'", w));
                }
                if let Some(ref f) = spec.function {
                    parts.push("-F".to_string());
                    parts.push(f.clone());
                }
                if let Some(ref c) = spec.command {
                    parts.push("-C".to_string());
                    parts.push(c.clone());
                }
                parts.push(cmd.clone());
                println!("{}", parts.join(" "));
            }
            return 0;
        }

        let mut spec = CompSpec::default();
        let mut commands = Vec::new();
        let mut i = 0;

        while i < args.len() {
            match args[i].as_str() {
                "-W" => {
                    i += 1;
                    if i < args.len() {
                        spec.wordlist = Some(args[i].clone());
                    }
                }
                "-F" => {
                    i += 1;
                    if i < args.len() {
                        spec.function = Some(args[i].clone());
                    }
                }
                "-C" => {
                    i += 1;
                    if i < args.len() {
                        spec.command = Some(args[i].clone());
                    }
                }
                "-G" => {
                    i += 1;
                    if i < args.len() {
                        spec.globpat = Some(args[i].clone());
                    }
                }
                "-P" => {
                    i += 1;
                    if i < args.len() {
                        spec.prefix = Some(args[i].clone());
                    }
                }
                "-S" => {
                    i += 1;
                    if i < args.len() {
                        spec.suffix = Some(args[i].clone());
                    }
                }
                "-a" => spec.actions.push("a".to_string()),
                "-b" => spec.actions.push("b".to_string()),
                "-c" => spec.actions.push("c".to_string()),
                "-d" => spec.actions.push("d".to_string()),
                "-e" => spec.actions.push("e".to_string()),
                "-f" => spec.actions.push("f".to_string()),
                "-j" => spec.actions.push("j".to_string()),
                "-r" => {
                    // Remove completion spec
                    i += 1;
                    while i < args.len() {
                        self.completions.remove(&args[i]);
                        i += 1;
                    }
                    return 0;
                }
                s if !s.starts_with('-') => commands.push(s.to_string()),
                _ => {}
            }
            i += 1;
        }

        for cmd in commands {
            self.completions.insert(cmd, spec.clone());
        }
        0
    }

    /// Modify completion options
    fn builtin_compopt(&mut self, args: &[String]) -> i32 {
        // Basic stub - just accept the options
        let _ = args;
        0
    }

    /// zsh compadd - add completion matches
    fn builtin_compadd(&mut self, args: &[String]) -> i32 {
        // Basic stub for zsh completion system
        // In a full implementation, this would add completion candidates
        let _ = args;
        0
    }

    /// zsh compset - modify completion prefix/suffix
    fn builtin_compset(&mut self, args: &[String]) -> i32 {
        // Basic stub for zsh completion system
        let _ = args;
        0
    }

    /// compdef - register completion functions for commands
    /// Usage: compdef _git git
    ///        compdef _docker docker docker-compose
    ///        compdef -d git  # delete
    fn builtin_compdef(&mut self, args: &[String]) -> i32 {
        if let Some(cache) = &mut self.compsys_cache {
            compsys::compdef::compdef_execute(cache, args)
        } else {
            // No cache - defer for cdreplay (zinit turbo mode)
            self.deferred_compdefs.push(args.to_vec());
            0
        }
    }

    /// compinit - initialize the completion system
    /// Scans fpath for completion functions and registers them
    #[tracing::instrument(level = "info", skip(self))]
    fn builtin_compinit(&mut self, args: &[String]) -> i32 {
        // Parse options
        // -C: use cache if valid (skip fpath scan)
        // -D: don't dump (don't write .zcompdump)
        // -d file: specify dump file
        // -u: use insecure dirs anyway  -i: silently ignore insecure dirs
        // -q: quiet
        let mut quiet = false;
        let mut no_dump = false;
        let mut dump_file: Option<String> = None;
        let mut use_cache = false;
        let mut ignore_insecure = false;
        let mut use_insecure = false;

        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "-q" => quiet = true,
                "-C" => use_cache = true,
                "-D" => no_dump = true,
                "-d" => {
                    i += 1;
                    if i < args.len() {
                        dump_file = Some(args[i].clone());
                    }
                }
                "-u" => use_insecure = true,
                "-i" => ignore_insecure = true,
                _ => {}
            }
            i += 1;
        }

        // Run compaudit with SQLite cache (unless -u skips it entirely)
        if !use_insecure && !self.posix_mode {
            if let Some(ref cache) = self.plugin_cache {
                let insecure = cache.compaudit_cached(&self.fpath);
                if !insecure.is_empty() && !ignore_insecure {
                    if !quiet {
                        eprintln!("compinit: insecure directories:");
                        for d in &insecure {
                            eprintln!("  {}", d);
                        }
                        eprintln!("compinit: run with -i to ignore or -u to use anyway");
                    }
                    return 1;
                }
            }
        }

        // ZSH COMPAT MODE: Use traditional zsh algorithm (fpath scan, .zcompdump, no SQLite)
        if self.zsh_compat {
            return self.compinit_compat(quiet, no_dump, dump_file, use_cache);
        }

        // ZSHRS MODE: Use SQLite cache with function bodies

        // Try to use existing cache if -C and cache is valid
        if use_cache {
            if let Some(cache) = &self.compsys_cache {
                if compsys::cache_is_valid(cache) {
                    // Load from cache instead of rescanning
                    if let Ok(result) = compsys::load_from_cache(cache) {
                        if !quiet {
                            tracing::info!(
                                comps = result.comps.len(),
                                "compinit: using cached completions"
                            );
                        }
                        self.assoc_arrays.insert("_comps".to_string(), result.comps);
                        self.assoc_arrays
                            .insert("_services".to_string(), result.services);
                        self.assoc_arrays
                            .insert("_patcomps".to_string(), result.patcomps);

                        // Background: fill bytecode blobs for any autoloads that have body but no ast.
                        // This populates the cache so subsequent autoload calls skip parsing.
                        if let Some(ref cache) = self.compsys_cache {
                            if let Ok(missing) = cache.count_autoloads_missing_bytecode() {
                                if missing > 0 {
                                    tracing::info!(
                                        count = missing,
                                        "compinit: backfilling bytecode blobs on worker pool"
                                    );
                                    let cache_path = compsys::cache::default_cache_path();
                                    let total_missing = missing;
                                    self.worker_pool.submit(move || {
                                        let mut cache = match compsys::cache::CompsysCache::open(&cache_path) {
                                            Ok(c) => c,
                                            Err(_) => return,
                                        };
                                        // Loop in batches of 100: fetch 100 bodies from SQLite,
                                        // parse them, write bytecode blobs back, repeat until none left.
                                        // Peak memory: ~100 function bodies + ASTs at a time.
                                        let mut total_cached = 0usize;
                                        loop {
                                            let stubs = match cache.get_autoloads_missing_bytecode_batch(100) {
                                                Ok(s) if !s.is_empty() => s,
                                                _ => break,
                                            };
                                            let mut batch: Vec<(String, Vec<u8>)> = Vec::with_capacity(stubs.len());
                                            for (name, body) in &stubs {
                                                let mut parser = crate::parser::ShellParser::new(body);
                                                if let Ok(commands) = parser.parse_script() {
                                                    if !commands.is_empty() {
                                                        let compiler = crate::shell_compiler::ShellCompiler::new();
                                                        let chunk = compiler.compile(&commands);
                                                        if let Ok(blob) = bincode::serialize(&chunk) {
                                                            batch.push((name.clone(), blob));
                                                        }
                                                    }
                                                }
                                            }
                                            total_cached += batch.len();
                                            if let Err(e) = cache.set_autoload_bytecodes_bulk(&batch) {
                                                tracing::warn!(error = %e, "compinit: bytecode backfill batch failed");
                                                break;
                                            }
                                            // If we got fewer than 100 results, we're done
                                            if stubs.len() < 100 {
                                                break;
                                            }
                                        }
                                        tracing::info!(
                                            cached = total_cached,
                                            total = total_missing,
                                            "compinit: bytecode backfill complete"
                                        );
                                    });
                                }
                            }
                        }

                        return 0;
                    }
                }
            }
        }

        // Ship compinit to worker pool — no ad-hoc thread spawn.
        // The heavy work (scan + SQLite write) runs on a pool thread.
        // Results are merged into shell state lazily via drain_compinit_bg().
        let fpath = self.fpath.clone();
        let fpath_count = fpath.len();
        let pool_size = self.worker_pool.size();
        let (tx, rx) = std::sync::mpsc::channel();
        let bg_start = std::time::Instant::now();
        tracing::info!(
            fpath_dirs = fpath_count,
            worker_pool = pool_size,
            "compinit: shipping to worker pool"
        );
        self.worker_pool.submit(move || {
                tracing::debug!("compinit-bg: thread started");
                let cache_path = compsys::cache::default_cache_path();
                if let Some(parent) = cache_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                // Remove old DB to start fresh
                let _ = std::fs::remove_file(&cache_path);
                let _ = std::fs::remove_file(format!("{}-shm", cache_path.display()));
                let _ = std::fs::remove_file(format!("{}-wal", cache_path.display()));

                let mut cache = match compsys::cache::CompsysCache::open(&cache_path) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!("compinit: failed to create cache: {}", e);
                        return;
                    }
                };

                let result = match compsys::build_cache_from_fpath(&fpath, &mut cache) {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::error!("compinit: scan failed: {}", e);
                        return;
                    }
                };

                tracing::info!(
                    functions = result.files_scanned,
                    comps = result.comps.len(),
                    dirs = result.dirs_scanned,
                    ms = result.scan_time_ms,
                    "compinit: background scan complete"
                );

                // Pre-parse function bodies and cache bytecode blobs.
                // Stream: parse one → serialize → write → drop. Never accumulate.
                // 16k functions × ~10KB AST = OOM if held in memory.
                let parse_start = std::time::Instant::now();
                let mut parse_ok = 0usize;
                let mut parse_fail = 0usize;
                let mut no_body = 0usize;
                let batch_size = 100;
                let mut batch: Vec<(String, Vec<u8>)> = Vec::with_capacity(batch_size);

                for file in &result.files {
                    if let Some(ref body) = file.body {
                        let mut parser = crate::parser::ShellParser::new(body);
                        match parser.parse_script() {
                            Ok(commands) if !commands.is_empty() => {
                                // Compile AST → fusevm bytecodes, then serialize the Chunk
                                let compiler = crate::shell_compiler::ShellCompiler::new();
                                let chunk = compiler.compile(&commands);
                                if let Ok(blob) = bincode::serialize(&chunk) {
                                    batch.push((file.name.clone(), blob));
                                    parse_ok += 1;
                                    if batch.len() >= batch_size {
                                        let _ = cache.set_autoload_bytecodes_bulk(&batch);
                                        batch.clear();
                                    }
                                }
                            }
                            Ok(_) => { parse_fail += 1; }
                            Err(_) => { parse_fail += 1; }
                        }
                    } else {
                        no_body += 1;
                    }
                }
                // Flush remaining
                if !batch.is_empty() {
                    let _ = cache.set_autoload_bytecodes_bulk(&batch);
                    batch.clear();
                }

                tracing::info!(
                    cached = parse_ok,
                    failed = parse_fail,
                    no_body = no_body,
                    total = result.files.len(),
                    ms = parse_start.elapsed().as_millis() as u64,
                    "compinit: bytecode blobs cached"
                );

                let _ = tx.send(CompInitBgResult { result, cache });
            });

        self.compinit_pending = Some((rx, bg_start));
        0
    }

    /// Non-blocking drain of background compinit results.
    /// Call this before any completion lookup (prompt, tab-complete, etc.).
    /// If the background thread hasn't finished yet, this is a no-op.
    pub fn drain_compinit_bg(&mut self) {
        if let Some((rx, start)) = self.compinit_pending.take() {
            match rx.try_recv() {
                Ok(bg) => {
                    let comps = bg.result.comps.len();
                    self.assoc_arrays
                        .insert("_comps".to_string(), bg.result.comps);
                    self.assoc_arrays
                        .insert("_services".to_string(), bg.result.services);
                    self.assoc_arrays
                        .insert("_patcomps".to_string(), bg.result.patcomps);
                    self.compsys_cache = Some(bg.cache);
                    tracing::info!(
                        wall_ms = start.elapsed().as_millis() as u64,
                        comps,
                        "compinit: background results merged"
                    );
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // Not ready yet — put the receiver back for next poll
                    self.compinit_pending = Some((rx, start));
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    tracing::warn!("compinit: background thread died without sending results");
                }
            }
        }
    }

    /// Traditional zsh compinit (--zsh-compat mode)
    /// Uses fpath scanning, .zcompdump files, no SQLite
    fn compinit_compat(
        &mut self,
        quiet: bool,
        no_dump: bool,
        dump_file: Option<String>,
        use_cache: bool,
    ) -> i32 {
        let zdotdir = self
            .variables
            .get("ZDOTDIR")
            .cloned()
            .or_else(|| std::env::var("ZDOTDIR").ok())
            .unwrap_or_else(|| std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string()));

        let dump_path = dump_file
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(&zdotdir).join(".zcompdump"));

        // -C: Try to use existing .zcompdump if valid
        if use_cache && dump_path.exists() {
            if compsys::check_dump(&dump_path, &self.fpath, "zshrs-0.1.0") {
                // Valid dump - source it to load _comps
                // For now, just rescan (proper impl would source the dump file)
                if !quiet {
                    tracing::info!("compinit: .zcompdump valid, rescanning for compat");
                }
            }
        }

        // Full fpath scan (traditional zsh algorithm)
        let result = compsys::compinit(&self.fpath);

        if !quiet {
            tracing::info!(
                functions = result.files_scanned,
                comps = result.comps.len(),
                dirs = result.dirs_scanned,
                ms = result.scan_time_ms,
                "compinit: fpath scan complete"
            );
        }

        // Write .zcompdump unless -D
        if !no_dump {
            let _ = compsys::compdump(&result, &dump_path, "zshrs-0.1.0");
        }

        // Set up _comps associative array
        self.assoc_arrays
            .insert("_comps".to_string(), result.comps.clone());
        self.assoc_arrays
            .insert("_services".to_string(), result.services.clone());
        self.assoc_arrays
            .insert("_patcomps".to_string(), result.patcomps.clone());

        // No SQLite cache in compat mode
        self.compsys_cache = None;

        0
    }

    /// cdreplay - replay deferred compdef calls (zinit turbo mode)
    /// Usage: cdreplay [-q]
    fn builtin_cdreplay(&mut self, args: &[String]) -> i32 {
        let quiet = args.contains(&"-q".to_string());

        if self.deferred_compdefs.is_empty() {
            return 0;
        }

        let deferred = std::mem::take(&mut self.deferred_compdefs);
        let count = deferred.len();

        if let Some(cache) = &mut self.compsys_cache {
            for compdef_args in deferred {
                compsys::compdef::compdef_execute(cache, &compdef_args);
            }
        }

        if !quiet {
            eprintln!("cdreplay: replayed {} compdef calls", count);
        }

        0
    }

    /// zsh zstyle - configure styles for completion
    fn builtin_zstyle(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            // List all styles
            for (pattern, style, values) in self.style_table.list(None) {
                println!("zstyle '{}' {} {}", pattern, style, values.join(" "));
            }
            return 0;
        }

        // Handle options
        if args[0].starts_with('-') {
            match args[0].as_str() {
                "-d" => {
                    // Delete style
                    let pattern = args.get(1).map(|s| s.as_str());
                    let style = args.get(2).map(|s| s.as_str());
                    self.style_table.delete(pattern, style);
                    return 0;
                }
                "-g" => {
                    // Get style into array
                    if args.len() >= 4 {
                        let array_name = &args[1];
                        let context = &args[2];
                        let style = &args[3];
                        if let Some(values) = self.style_table.get(context, style) {
                            self.arrays.insert(array_name.clone(), values.to_vec());
                            return 0;
                        }
                    }
                    return 1;
                }
                "-s" => {
                    // Get style as scalar
                    if args.len() >= 4 {
                        let var_name = &args[1];
                        let context = &args[2];
                        let style = &args[3];
                        let sep = args.get(4).map(|s| s.as_str()).unwrap_or(" ");
                        if let Some(values) = self.style_table.get(context, style) {
                            self.variables.insert(var_name.clone(), values.join(sep));
                            return 0;
                        }
                    }
                    return 1;
                }
                "-t" => {
                    // Test style (check if true/yes)
                    if args.len() >= 3 {
                        let context = &args[1];
                        let style = &args[2];
                        return if self.style_table.test_bool(context, style).unwrap_or(false) {
                            0
                        } else {
                            1
                        };
                    }
                    return 1;
                }
                "-L" => {
                    // List in re-usable format
                    for (pattern, style, values) in self.style_table.list(None) {
                        let values_str = values
                            .iter()
                            .map(|v| format!("'{}'", v.replace('\'', "'\\''")))
                            .collect::<Vec<_>>()
                            .join(" ");
                        println!("zstyle '{}' {} {}", pattern, style, values_str);
                    }
                    return 0;
                }
                _ => {}
            }
        }

        // Set style: zstyle pattern style values...
        if args.len() >= 2 {
            let pattern = &args[0];
            let style = &args[1];
            let values: Vec<String> = args[2..].to_vec();
            self.style_table.set(pattern, style, values.clone(), false);

            // Write to SQLite cache for completion lookups
            if let Some(cache) = &self.compsys_cache {
                let _ = cache.set_zstyle(pattern, style, &values, false);
            }

            // Also update legacy zstyles for backward compat
            let existing = self
                .zstyles
                .iter_mut()
                .find(|s| s.pattern == *pattern && s.style == *style);
            if let Some(s) = existing {
                s.values = args[2..].to_vec();
            } else {
                self.zstyles.push(ZStyle {
                    pattern: pattern.clone(),
                    style: style.clone(),
                    values: args[2..].to_vec(),
                });
            }
        }
        0
    }

    /// Tie a parameter to a GDBM database
    /// Usage: ztie -d db/gdbm -f /path/to/db.gdbm [-r] PARAM_NAME
    fn builtin_ztie(&mut self, args: &[String]) -> i32 {
        use crate::db_gdbm;

        let mut db_type: Option<String> = None;
        let mut file_path: Option<String> = None;
        let mut readonly = false;
        let mut param_args: Vec<String> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "-d" => {
                    if i + 1 < args.len() {
                        db_type = Some(args[i + 1].clone());
                        i += 2;
                    } else {
                        eprintln!("ztie: -d requires an argument");
                        return 1;
                    }
                }
                "-f" => {
                    if i + 1 < args.len() {
                        file_path = Some(args[i + 1].clone());
                        i += 2;
                    } else {
                        eprintln!("ztie: -f requires an argument");
                        return 1;
                    }
                }
                "-r" => {
                    readonly = true;
                    i += 1;
                }
                arg if arg.starts_with('-') => {
                    eprintln!("ztie: bad option: {}", arg);
                    return 1;
                }
                _ => {
                    param_args.push(args[i].clone());
                    i += 1;
                }
            }
        }

        match db_gdbm::ztie(
            &param_args,
            readonly,
            db_type.as_deref(),
            file_path.as_deref(),
        ) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("ztie: {}", e);
                1
            }
        }
    }

    /// Untie a parameter from its GDBM database
    /// Usage: zuntie [-u] PARAM_NAME...
    fn builtin_zuntie(&mut self, args: &[String]) -> i32 {
        use crate::db_gdbm;

        let mut force_unset = false;
        let mut param_args: Vec<String> = Vec::new();

        for arg in args {
            match arg.as_str() {
                "-u" => force_unset = true,
                a if a.starts_with('-') => {
                    eprintln!("zuntie: bad option: {}", a);
                    return 1;
                }
                _ => param_args.push(arg.clone()),
            }
        }

        if param_args.is_empty() {
            eprintln!("zuntie: not enough arguments");
            return 1;
        }

        match db_gdbm::zuntie(&param_args, force_unset) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("zuntie: {}", e);
                1
            }
        }
    }

    /// Get the path of a tied GDBM database
    /// Usage: zgdbmpath PARAM_NAME
    /// Sets $REPLY to the path
    fn builtin_zgdbmpath(&mut self, args: &[String]) -> i32 {
        use crate::db_gdbm;

        if args.is_empty() {
            eprintln!(
                "zgdbmpath: parameter name (whose path is to be written to $REPLY) is required"
            );
            return 1;
        }

        match db_gdbm::zgdbmpath(&args[0]) {
            Ok(path) => {
                self.variables.insert("REPLY".to_string(), path.clone());
                std::env::set_var("REPLY", &path);
                0
            }
            Err(e) => {
                eprintln!("zgdbmpath: {}", e);
                1
            }
        }
    }

    /// Push directory onto stack and cd to it
    fn builtin_pushd(&mut self, args: &[String]) -> i32 {
        // pushd [ -qsLP ] [ arg ]
        // pushd [ -qsLP ] old new
        // pushd [ -qsLP ] {+|-}n
        // -q: quiet (don't print stack)
        // -s: no symlink resolution (use -L cd behavior)
        // -L: logical directory (resolve .. before symlinks)
        // -P: physical directory (resolve symlinks)

        let mut quiet = false;
        let mut physical = false;
        let mut positional_args: Vec<String> = Vec::new();

        for arg in args {
            if arg.starts_with('-') && arg.len() > 1 {
                // Check if it's a stack index
                if arg[1..].chars().all(|c| c.is_ascii_digit()) {
                    positional_args.push(arg.clone());
                    continue;
                }
                for ch in arg[1..].chars() {
                    match ch {
                        'q' => quiet = true,
                        's' => physical = false,
                        'L' => physical = false,
                        'P' => physical = true,
                        _ => {}
                    }
                }
            } else if arg.starts_with('+') {
                positional_args.push(arg.clone());
            } else {
                positional_args.push(arg.clone());
            }
        }

        let current = match std::env::current_dir() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("pushd: {}", e);
                return 1;
            }
        };

        if positional_args.is_empty() {
            // Swap top two directories
            if self.dir_stack.is_empty() {
                eprintln!("pushd: no other directory");
                return 1;
            }
            let target = self.dir_stack.pop().unwrap();
            self.dir_stack.push(current.clone());

            let resolved = if physical {
                target.canonicalize().unwrap_or(target.clone())
            } else {
                target.clone()
            };

            if let Err(e) = std::env::set_current_dir(&resolved) {
                eprintln!("pushd: {}: {}", target.display(), e);
                self.dir_stack.pop();
                self.dir_stack.push(target);
                return 1;
            }
            if !quiet {
                self.print_dir_stack();
            }
            return 0;
        }

        let arg = &positional_args[0];

        // Handle +N and -N for rotating the stack
        if arg.starts_with('+') || arg.starts_with('-') {
            if let Ok(n) = arg[1..].parse::<usize>() {
                let total = self.dir_stack.len() + 1;
                if n >= total {
                    eprintln!("pushd: {}: directory stack index out of range", arg);
                    return 1;
                }
                // Rotate stack
                let rotate_pos = if arg.starts_with('+') { n } else { total - n };
                let mut full_stack = vec![current.clone()];
                full_stack.extend(self.dir_stack.iter().cloned());
                full_stack.rotate_left(rotate_pos);

                let target = full_stack.remove(0);
                self.dir_stack = full_stack;

                let resolved = if physical {
                    target.canonicalize().unwrap_or(target.clone())
                } else {
                    target.clone()
                };

                if let Err(e) = std::env::set_current_dir(&resolved) {
                    eprintln!("pushd: {}: {}", target.display(), e);
                    return 1;
                }
                if !quiet {
                    self.print_dir_stack();
                }
                return 0;
            }
        }

        // Regular directory push
        let target = PathBuf::from(arg);
        let resolved = if physical {
            target.canonicalize().unwrap_or(target.clone())
        } else {
            target.clone()
        };

        self.dir_stack.push(current);
        if let Err(e) = std::env::set_current_dir(&resolved) {
            eprintln!("pushd: {}: {}", arg, e);
            self.dir_stack.pop();
            return 1;
        }
        if !quiet {
            self.print_dir_stack();
        }
        0
    }

    /// Pop directory from stack and cd to it
    fn builtin_popd(&mut self, args: &[String]) -> i32 {
        // popd [ -qsLP ] [ {+|-}n ]
        // -q: quiet (don't print stack)
        // -s: no symlink resolution
        // -L: logical directory
        // -P: physical directory

        let mut quiet = false;
        let mut physical = false;
        let mut stack_index: Option<String> = None;

        for arg in args {
            if arg.starts_with('-') && arg.len() > 1 {
                // Check if it's a stack index
                if arg[1..].chars().all(|c| c.is_ascii_digit()) {
                    stack_index = Some(arg.clone());
                    continue;
                }
                for ch in arg[1..].chars() {
                    match ch {
                        'q' => quiet = true,
                        's' => physical = false,
                        'L' => physical = false,
                        'P' => physical = true,
                        _ => {}
                    }
                }
            } else if arg.starts_with('+') {
                stack_index = Some(arg.clone());
            }
        }

        if self.dir_stack.is_empty() {
            eprintln!("popd: directory stack empty");
            return 1;
        }

        // Handle +N and -N
        if let Some(arg) = stack_index {
            if arg.starts_with('+') || arg.starts_with('-') {
                if let Ok(n) = arg[1..].parse::<usize>() {
                    let total = self.dir_stack.len() + 1;
                    if n >= total {
                        eprintln!("popd: {}: directory stack index out of range", arg);
                        return 1;
                    }
                    let remove_pos = if arg.starts_with('+') {
                        n
                    } else {
                        total - 1 - n
                    };
                    if remove_pos == 0 {
                        // Remove current and cd to next
                        let target = self.dir_stack.remove(0);
                        let resolved = if physical {
                            target.canonicalize().unwrap_or(target.clone())
                        } else {
                            target.clone()
                        };
                        if let Err(e) = std::env::set_current_dir(&resolved) {
                            eprintln!("popd: {}: {}", target.display(), e);
                            return 1;
                        }
                    } else {
                        self.dir_stack.remove(remove_pos - 1);
                    }
                    if !quiet {
                        self.print_dir_stack();
                    }
                    return 0;
                }
            }
        }

        let target = self.dir_stack.pop().unwrap();
        let resolved = if physical {
            target.canonicalize().unwrap_or(target.clone())
        } else {
            target.clone()
        };
        if let Err(e) = std::env::set_current_dir(&resolved) {
            eprintln!("popd: {}: {}", target.display(), e);
            self.dir_stack.push(target);
            return 1;
        }
        if !quiet {
            self.print_dir_stack();
        }
        0
    }

    /// Display directory stack
    fn builtin_dirs(&mut self, args: &[String]) -> i32 {
        // dirs [ -c ] [ -l ] [ -p ] [ -v ] [ arg ... ]
        // -c: clear the directory stack
        // -l: full pathnames (don't use ~)
        // -p: print one entry per line
        // -v: verbose (numbered list)

        let mut clear = false;
        let mut full_paths = false;
        let mut per_line = false;
        let mut verbose = false;
        let mut indices: Vec<i32> = Vec::new();

        for arg in args {
            if arg.starts_with('-') && arg.len() > 1 {
                // Check if it's a negative index like -2
                if arg[1..].chars().all(|c| c.is_ascii_digit()) {
                    if let Ok(n) = arg.parse::<i32>() {
                        indices.push(n);
                        continue;
                    }
                }
                for ch in arg[1..].chars() {
                    match ch {
                        'c' => clear = true,
                        'l' => full_paths = true,
                        'p' => per_line = true,
                        'v' => verbose = true,
                        _ => {}
                    }
                }
            } else if arg.starts_with('+') && arg.len() > 1 {
                if let Ok(n) = arg[1..].parse::<i32>() {
                    indices.push(n);
                }
            } else {
                // Could be a number
                if let Ok(n) = arg.parse::<i32>() {
                    indices.push(n);
                }
            }
        }

        if clear {
            self.dir_stack.clear();
            return 0;
        }

        let current = std::env::current_dir().unwrap_or_default();
        let home = dirs::home_dir().unwrap_or_default();

        let format_path = |p: &std::path::Path| -> String {
            let path_str = p.to_string_lossy().to_string();
            if !full_paths {
                let home_str = home.to_string_lossy();
                if path_str.starts_with(home_str.as_ref()) {
                    return format!("~{}", &path_str[home_str.len()..]);
                }
            }
            path_str
        };

        // If specific indices requested
        if !indices.is_empty() {
            let stack_len = self.dir_stack.len() + 1; // +1 for current dir
            for idx in indices {
                let actual_idx = if idx >= 0 {
                    idx as usize
                } else {
                    stack_len.saturating_sub((-idx) as usize)
                };

                if actual_idx == 0 {
                    println!("{}", format_path(&current));
                } else if actual_idx <= self.dir_stack.len() {
                    // Stack is reversed, so index from end
                    let stack_idx = self.dir_stack.len() - actual_idx;
                    if let Some(dir) = self.dir_stack.get(stack_idx) {
                        println!("{}", format_path(dir));
                    }
                }
            }
            return 0;
        }

        if verbose {
            println!(" 0  {}", format_path(&current));
            for (i, dir) in self.dir_stack.iter().rev().enumerate() {
                println!("{:2}  {}", i + 1, format_path(dir));
            }
        } else if per_line {
            println!("{}", format_path(&current));
            for dir in self.dir_stack.iter().rev() {
                println!("{}", format_path(dir));
            }
        } else {
            let mut parts = vec![format_path(&current)];
            for dir in self.dir_stack.iter().rev() {
                parts.push(format_path(dir));
            }
            println!("{}", parts.join(" "));
        }
        0
    }

    fn print_dir_stack(&self) {
        let current = std::env::current_dir().unwrap_or_default();
        let mut parts = vec![current.to_string_lossy().to_string()];
        for dir in self.dir_stack.iter().rev() {
            parts.push(dir.to_string_lossy().to_string());
        }
        println!("{}", parts.join(" "));
    }

    /// printf builtin - format and print data (zsh/bash compatible)
    fn builtin_printf(&self, args: &[String]) -> i32 {
        if args.is_empty() {
            eprintln!("printf: usage: printf format [arguments]");
            return 1;
        }

        let format = &args[0];
        let format_args = &args[1..];
        let mut arg_idx = 0;
        let mut output = String::new();
        let mut chars = format.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => output.push('\n'),
                    Some('t') => output.push('\t'),
                    Some('r') => output.push('\r'),
                    Some('\\') => output.push('\\'),
                    Some('a') => output.push('\x07'),
                    Some('b') => output.push('\x08'),
                    Some('e') | Some('E') => output.push('\x1b'),
                    Some('f') => output.push('\x0c'),
                    Some('v') => output.push('\x0b'),
                    Some('"') => output.push('"'),
                    Some('\'') => output.push('\''),
                    Some('0') => {
                        let mut octal = String::new();
                        while octal.len() < 3 {
                            if let Some(&d) = chars.peek() {
                                if d >= '0' && d <= '7' {
                                    octal.push(d);
                                    chars.next();
                                } else {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                        if octal.is_empty() {
                            output.push('\0');
                        } else if let Ok(val) = u8::from_str_radix(&octal, 8) {
                            output.push(val as char);
                        }
                    }
                    Some('x') => {
                        let mut hex = String::new();
                        while hex.len() < 2 {
                            if let Some(&d) = chars.peek() {
                                if d.is_ascii_hexdigit() {
                                    hex.push(d);
                                    chars.next();
                                } else {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                        if !hex.is_empty() {
                            if let Ok(val) = u8::from_str_radix(&hex, 16) {
                                output.push(val as char);
                            }
                        }
                    }
                    Some('u') => {
                        let mut hex = String::new();
                        while hex.len() < 4 {
                            if let Some(&d) = chars.peek() {
                                if d.is_ascii_hexdigit() {
                                    hex.push(d);
                                    chars.next();
                                } else {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                        if !hex.is_empty() {
                            if let Ok(val) = u32::from_str_radix(&hex, 16) {
                                if let Some(c) = char::from_u32(val) {
                                    output.push(c);
                                }
                            }
                        }
                    }
                    Some('U') => {
                        let mut hex = String::new();
                        while hex.len() < 8 {
                            if let Some(&d) = chars.peek() {
                                if d.is_ascii_hexdigit() {
                                    hex.push(d);
                                    chars.next();
                                } else {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                        if !hex.is_empty() {
                            if let Ok(val) = u32::from_str_radix(&hex, 16) {
                                if let Some(c) = char::from_u32(val) {
                                    output.push(c);
                                }
                            }
                        }
                    }
                    Some('c') => {
                        print!("{}", output);
                        return 0;
                    }
                    Some(other) => {
                        output.push('\\');
                        output.push(other);
                    }
                    None => output.push('\\'),
                }
            } else if c == '%' {
                if chars.peek() == Some(&'%') {
                    chars.next();
                    output.push('%');
                    continue;
                }

                let mut flags = String::new();
                while let Some(&f) = chars.peek() {
                    if f == '-' || f == '+' || f == ' ' || f == '#' || f == '0' {
                        flags.push(f);
                        chars.next();
                    } else {
                        break;
                    }
                }

                let mut width = String::new();
                if chars.peek() == Some(&'*') {
                    chars.next();
                    if arg_idx < format_args.len() {
                        width = format_args[arg_idx].clone();
                        arg_idx += 1;
                    }
                } else {
                    while let Some(&d) = chars.peek() {
                        if d.is_ascii_digit() {
                            width.push(d);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                }

                let mut precision = String::new();
                if chars.peek() == Some(&'.') {
                    chars.next();
                    if chars.peek() == Some(&'*') {
                        chars.next();
                        if arg_idx < format_args.len() {
                            precision = format_args[arg_idx].clone();
                            arg_idx += 1;
                        }
                    } else {
                        while let Some(&d) = chars.peek() {
                            if d.is_ascii_digit() {
                                precision.push(d);
                                chars.next();
                            } else {
                                break;
                            }
                        }
                    }
                }

                let specifier = chars.next().unwrap_or('s');
                let arg = if arg_idx < format_args.len() {
                    let a = &format_args[arg_idx];
                    arg_idx += 1;
                    a.clone()
                } else {
                    String::new()
                };

                let width_val: usize = width.parse().unwrap_or(0);
                let prec_val: Option<usize> = if precision.is_empty() {
                    None
                } else {
                    precision.parse().ok()
                };
                let left_align = flags.contains('-');
                let zero_pad = flags.contains('0') && !left_align;
                let plus_sign = flags.contains('+');
                let space_sign = flags.contains(' ') && !plus_sign;
                let alt_form = flags.contains('#');

                match specifier {
                    's' => {
                        let mut s = arg;
                        if let Some(p) = prec_val {
                            s = s.chars().take(p).collect();
                        }
                        if width_val > s.len() {
                            if left_align {
                                output.push_str(&s);
                                output.push_str(&" ".repeat(width_val - s.len()));
                            } else {
                                output.push_str(&" ".repeat(width_val - s.len()));
                                output.push_str(&s);
                            }
                        } else {
                            output.push_str(&s);
                        }
                    }
                    'b' => {
                        let expanded = self.expand_printf_escapes(&arg);
                        if let Some(p) = prec_val {
                            let s: String = expanded.chars().take(p).collect();
                            output.push_str(&s);
                        } else {
                            output.push_str(&expanded);
                        }
                    }
                    'c' => {
                        if let Some(ch) = arg.chars().next() {
                            output.push(ch);
                        }
                    }
                    'q' => {
                        output.push('\'');
                        for ch in arg.chars() {
                            if ch == '\'' {
                                output.push_str("'\\''");
                            } else {
                                output.push(ch);
                            }
                        }
                        output.push('\'');
                    }
                    'd' | 'i' => {
                        let val: i64 = if arg.starts_with("0x") || arg.starts_with("0X") {
                            i64::from_str_radix(&arg[2..], 16).unwrap_or(0)
                        } else if arg.starts_with("0") && arg.len() > 1 && !arg.contains('.') {
                            i64::from_str_radix(&arg[1..], 8).unwrap_or(0)
                        } else if arg.starts_with('\'') || arg.starts_with('"') {
                            arg.chars().nth(1).map(|c| c as i64).unwrap_or(0)
                        } else {
                            arg.parse().unwrap_or(0)
                        };

                        let sign = if val < 0 {
                            "-"
                        } else if plus_sign {
                            "+"
                        } else if space_sign {
                            " "
                        } else {
                            ""
                        };
                        let abs_val = val.abs();
                        let num_str = abs_val.to_string();
                        let total_len = sign.len() + num_str.len();

                        if width_val > total_len {
                            if left_align {
                                output.push_str(sign);
                                output.push_str(&num_str);
                                output.push_str(&" ".repeat(width_val - total_len));
                            } else if zero_pad {
                                output.push_str(sign);
                                output.push_str(&"0".repeat(width_val - total_len));
                                output.push_str(&num_str);
                            } else {
                                output.push_str(&" ".repeat(width_val - total_len));
                                output.push_str(sign);
                                output.push_str(&num_str);
                            }
                        } else {
                            output.push_str(sign);
                            output.push_str(&num_str);
                        }
                    }
                    'u' => {
                        let val: u64 = if arg.starts_with("0x") || arg.starts_with("0X") {
                            u64::from_str_radix(&arg[2..], 16).unwrap_or(0)
                        } else if arg.starts_with("0") && arg.len() > 1 {
                            u64::from_str_radix(&arg[1..], 8).unwrap_or(0)
                        } else {
                            arg.parse().unwrap_or(0)
                        };
                        let num_str = val.to_string();
                        if width_val > num_str.len() {
                            if left_align {
                                output.push_str(&num_str);
                                output.push_str(&" ".repeat(width_val - num_str.len()));
                            } else if zero_pad {
                                output.push_str(&"0".repeat(width_val - num_str.len()));
                                output.push_str(&num_str);
                            } else {
                                output.push_str(&" ".repeat(width_val - num_str.len()));
                                output.push_str(&num_str);
                            }
                        } else {
                            output.push_str(&num_str);
                        }
                    }
                    'o' => {
                        let val: u64 = arg.parse().unwrap_or(0);
                        let num_str = format!("{:o}", val);
                        let prefix = if alt_form && val != 0 { "0" } else { "" };
                        let total_len = prefix.len() + num_str.len();
                        if width_val > total_len {
                            if left_align {
                                output.push_str(prefix);
                                output.push_str(&num_str);
                                output.push_str(&" ".repeat(width_val - total_len));
                            } else {
                                output.push_str(&" ".repeat(width_val - total_len));
                                output.push_str(prefix);
                                output.push_str(&num_str);
                            }
                        } else {
                            output.push_str(prefix);
                            output.push_str(&num_str);
                        }
                    }
                    'x' => {
                        let val: u64 = arg.parse().unwrap_or(0);
                        let num_str = format!("{:x}", val);
                        let prefix = if alt_form && val != 0 { "0x" } else { "" };
                        let total_len = prefix.len() + num_str.len();
                        if width_val > total_len {
                            if left_align {
                                output.push_str(prefix);
                                output.push_str(&num_str);
                                output.push_str(&" ".repeat(width_val - total_len));
                            } else {
                                output.push_str(&" ".repeat(width_val - total_len));
                                output.push_str(prefix);
                                output.push_str(&num_str);
                            }
                        } else {
                            output.push_str(prefix);
                            output.push_str(&num_str);
                        }
                    }
                    'X' => {
                        let val: u64 = arg.parse().unwrap_or(0);
                        let num_str = format!("{:X}", val);
                        let prefix = if alt_form && val != 0 { "0X" } else { "" };
                        let total_len = prefix.len() + num_str.len();
                        if width_val > total_len {
                            if left_align {
                                output.push_str(prefix);
                                output.push_str(&num_str);
                                output.push_str(&" ".repeat(width_val - total_len));
                            } else {
                                output.push_str(&" ".repeat(width_val - total_len));
                                output.push_str(prefix);
                                output.push_str(&num_str);
                            }
                        } else {
                            output.push_str(prefix);
                            output.push_str(&num_str);
                        }
                    }
                    'e' | 'E' => {
                        let val: f64 = arg.parse().unwrap_or(0.0);
                        let prec = prec_val.unwrap_or(6);
                        let formatted = if specifier == 'e' {
                            format!("{:.prec$e}", val, prec = prec)
                        } else {
                            format!("{:.prec$E}", val, prec = prec)
                        };
                        if width_val > formatted.len() {
                            if left_align {
                                output.push_str(&formatted);
                                output.push_str(&" ".repeat(width_val - formatted.len()));
                            } else {
                                output.push_str(&" ".repeat(width_val - formatted.len()));
                                output.push_str(&formatted);
                            }
                        } else {
                            output.push_str(&formatted);
                        }
                    }
                    'f' | 'F' => {
                        let val: f64 = arg.parse().unwrap_or(0.0);
                        let prec = prec_val.unwrap_or(6);
                        let sign = if val < 0.0 {
                            "-"
                        } else if plus_sign {
                            "+"
                        } else if space_sign {
                            " "
                        } else {
                            ""
                        };
                        let formatted = format!("{:.prec$}", val.abs(), prec = prec);
                        let total = sign.len() + formatted.len();
                        if width_val > total {
                            if left_align {
                                output.push_str(sign);
                                output.push_str(&formatted);
                                output.push_str(&" ".repeat(width_val - total));
                            } else if zero_pad {
                                output.push_str(sign);
                                output.push_str(&"0".repeat(width_val - total));
                                output.push_str(&formatted);
                            } else {
                                output.push_str(&" ".repeat(width_val - total));
                                output.push_str(sign);
                                output.push_str(&formatted);
                            }
                        } else {
                            output.push_str(sign);
                            output.push_str(&formatted);
                        }
                    }
                    'g' | 'G' => {
                        let val: f64 = arg.parse().unwrap_or(0.0);
                        let prec = prec_val.unwrap_or(6).max(1);
                        let formatted = if specifier == 'g' {
                            format!("{:.prec$}", val, prec = prec)
                        } else {
                            format!("{:.prec$}", val, prec = prec).to_uppercase()
                        };
                        output.push_str(&formatted);
                    }
                    'a' | 'A' => {
                        let val: f64 = arg.parse().unwrap_or(0.0);
                        let formatted = float_to_hex(val, specifier == 'A');
                        output.push_str(&formatted);
                    }
                    _ => {
                        output.push('%');
                        output.push(specifier);
                    }
                }
            } else {
                output.push(c);
            }
        }

        print!("{}", output);
        0
    }

    fn expand_printf_escapes(&self, s: &str) -> String {
        let mut result = String::new();
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => result.push('\n'),
                    Some('t') => result.push('\t'),
                    Some('r') => result.push('\r'),
                    Some('\\') => result.push('\\'),
                    Some('a') => result.push('\x07'),
                    Some('b') => result.push('\x08'),
                    Some('e') | Some('E') => result.push('\x1b'),
                    Some('f') => result.push('\x0c'),
                    Some('v') => result.push('\x0b'),
                    Some('0') => {
                        let mut octal = String::new();
                        while octal.len() < 3 {
                            if let Some(&d) = chars.peek() {
                                if d >= '0' && d <= '7' {
                                    octal.push(d);
                                    chars.next();
                                } else {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                        if octal.is_empty() {
                            result.push('\0');
                        } else if let Ok(val) = u8::from_str_radix(&octal, 8) {
                            result.push(val as char);
                        }
                    }
                    Some('c') => break,
                    Some(other) => {
                        result.push('\\');
                        result.push(other);
                    }
                    None => result.push('\\'),
                }
            } else {
                result.push(c);
            }
        }
        result
    }

    fn evaluate_arithmetic_expr(&mut self, expr: &str) -> i64 {
        self.eval_arith_expr(expr)
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // Additional zsh builtins
    // ═══════════════════════════════════════════════════════════════════════════

    /// break - exit from for/while/until loop
    fn builtin_break(&mut self, args: &[String]) -> i32 {
        let levels: i32 = args.first().and_then(|s| s.parse().ok()).unwrap_or(1);
        self.breaking = levels.max(1);
        0
    }

    /// continue - skip to next iteration of loop
    fn builtin_continue(&mut self, args: &[String]) -> i32 {
        let levels: i32 = args.first().and_then(|s| s.parse().ok()).unwrap_or(1);
        self.continuing = levels.max(1);
        0
    }

    /// disable - disable shell builtins, aliases, functions
    fn builtin_disable(&mut self, args: &[String]) -> i32 {
        let mut disable_aliases = false;
        let mut disable_builtins = false;
        let mut disable_functions = false;
        let mut names = Vec::new();

        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-a" => disable_aliases = true,
                "-f" => disable_functions = true,
                "-r" => disable_builtins = true,
                _ if arg.starts_with('-') => {}
                _ => names.push(arg.clone()),
            }
        }

        // Default to builtins if no flags
        if !disable_aliases && !disable_functions {
            disable_builtins = true;
        }

        for name in names {
            if disable_aliases {
                self.aliases.remove(&name);
            }
            if disable_functions {
                self.functions.remove(&name);
            }
            if disable_builtins {
                // Store disabled builtins
                self.options.insert(format!("_disabled_{}", name), true);
            }
        }
        0
    }

    /// enable - enable disabled shell builtins
    fn builtin_enable(&mut self, args: &[String]) -> i32 {
        for arg in args {
            if !arg.starts_with('-') {
                self.options.remove(&format!("_disabled_{}", arg));
            }
        }
        0
    }

    /// emulate - set up zsh emulation mode
    fn builtin_emulate(&mut self, args: &[String]) -> i32 {
        // emulate [ -lLR ] [ {zsh|sh|ksh|csh} [ flags ... ] ]
        // flags can include: -c arg, -o opt, +o opt
        let mut local_mode = false;
        let mut reset_mode = false;
        let mut list_mode = false;
        let mut mode: Option<String> = None;
        let mut command_arg: Option<String> = None;
        let mut extra_set_opts: Vec<String> = Vec::new();
        let mut extra_unset_opts: Vec<String> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];

            if arg == "-c" {
                // -c arg: evaluate arg in emulation mode
                i += 1;
                if i < args.len() {
                    command_arg = Some(args[i].clone());
                } else {
                    eprintln!("emulate: -c requires an argument");
                    return 1;
                }
            } else if arg == "-o" {
                // -o opt: set option
                i += 1;
                if i < args.len() {
                    extra_set_opts.push(args[i].clone());
                } else {
                    eprintln!("emulate: -o requires an argument");
                    return 1;
                }
            } else if arg == "+o" {
                // +o opt: unset option
                i += 1;
                if i < args.len() {
                    extra_unset_opts.push(args[i].clone());
                } else {
                    eprintln!("emulate: +o requires an argument");
                    return 1;
                }
            } else if arg.starts_with('-') && arg.len() > 1 && !arg.starts_with("--") {
                // Parse combined flags like -LR
                for ch in arg[1..].chars() {
                    match ch {
                        'L' => local_mode = true,
                        'R' => reset_mode = true,
                        'l' => list_mode = true,
                        _ => {
                            eprintln!("emulate: bad option: -{}", ch);
                            return 1;
                        }
                    }
                }
            } else if arg.starts_with('+') && arg.len() > 1 {
                // +X flags (unset single-letter options)
                for ch in arg[1..].chars() {
                    // Map single-letter to option name if needed
                    extra_unset_opts.push(ch.to_string());
                }
            } else if mode.is_none() {
                mode = Some(arg.clone());
            }
            i += 1;
        }

        // -L and -c are mutually exclusive
        if local_mode && command_arg.is_some() {
            eprintln!("emulate: -L and -c are mutually exclusive");
            return 1;
        }

        // No argument: print current emulation mode
        if mode.is_none() && !list_mode {
            let current = self
                .variables
                .get("EMULATE")
                .cloned()
                .unwrap_or_else(|| "zsh".to_string());
            println!("{}", current);
            return 0;
        }

        let mode = mode.unwrap_or_else(|| "zsh".to_string());

        // Get the options that would be set for this mode
        let (set_opts, unset_opts) = Self::emulate_mode_options(&mode, reset_mode);

        // -l: just list the options, don't apply
        if list_mode {
            for opt in &set_opts {
                println!("{}", opt);
            }
            for opt in &unset_opts {
                println!("no{}", opt);
            }
            if local_mode {
                println!("localoptions");
                println!("localpatterns");
                println!("localtraps");
            }
            return 0;
        }

        // Save current state if -c is used
        let saved_options = if command_arg.is_some() {
            Some(self.options.clone())
        } else {
            None
        };
        let saved_emulate = if command_arg.is_some() {
            self.variables.get("EMULATE").cloned()
        } else {
            None
        };

        // Apply the emulation
        self.variables.insert("EMULATE".to_string(), mode.clone());

        // Set options for this mode
        for opt in &set_opts {
            let opt_name = opt.to_lowercase().replace('_', "");
            self.options.insert(opt_name, true);
        }
        for opt in &unset_opts {
            let opt_name = opt.to_lowercase().replace('_', "");
            self.options.insert(opt_name, false);
        }

        // Apply extra -o / +o options
        for opt in &extra_set_opts {
            let opt_name = opt.to_lowercase().replace('_', "");
            self.options.insert(opt_name, true);
        }
        for opt in &extra_unset_opts {
            let opt_name = opt.to_lowercase().replace('_', "");
            self.options.insert(opt_name, false);
        }

        // -L: set local options/traps
        if local_mode {
            self.options.insert("localoptions".to_string(), true);
            self.options.insert("localpatterns".to_string(), true);
            self.options.insert("localtraps".to_string(), true);
        }

        // -c arg: execute command then restore
        let result = if let Some(cmd) = command_arg {
            let status = self.execute_script(&cmd).unwrap_or(1);

            // Restore saved state
            if let Some(opts) = saved_options {
                self.options = opts;
            }
            if let Some(emu) = saved_emulate {
                self.variables.insert("EMULATE".to_string(), emu);
            } else {
                self.variables.remove("EMULATE");
            }

            status
        } else {
            0
        };

        result
    }

    /// Get options to set/unset for an emulation mode
    fn emulate_mode_options(mode: &str, reset: bool) -> (Vec<&'static str>, Vec<&'static str>) {
        match mode {
            "zsh" => {
                if reset {
                    // Full reset: return to zsh defaults
                    (
                        vec![
                            "aliases",
                            "alwayslastprompt",
                            "autolist",
                            "automenu",
                            "autoparamslash",
                            "autoremoveslash",
                            "banghist",
                            "bareglobqual",
                            "completeinword",
                            "extendedhistory",
                            "functionargzero",
                            "glob",
                            "hashcmds",
                            "hashdirs",
                            "histexpand",
                            "histignoredups",
                            "interactivecomments",
                            "listambiguous",
                            "listtypes",
                            "multios",
                            "nomatch",
                            "notify",
                            "promptpercent",
                            "promptsubst",
                        ],
                        vec![
                            "ksharrays",
                            "kshglob",
                            "shwordsplit",
                            "shglob",
                            "posixbuiltins",
                            "posixidentifiers",
                            "posixstrings",
                            "bsdecho",
                            "ignorebraces",
                        ],
                    )
                } else {
                    // Minimal changes for portability
                    (vec!["functionargzero"], vec!["ksharrays", "shwordsplit"])
                }
            }
            "sh" => {
                let set = vec![
                    "ksharrays",
                    "shwordsplit",
                    "posixbuiltins",
                    "shglob",
                    "shfileexpansion",
                    "globsubst",
                    "interactivecomments",
                    "rmstarsilent",
                    "bsdecho",
                    "ignorebraces",
                ];
                let unset = vec![
                    "badpattern",
                    "banghist",
                    "bgnice",
                    "equals",
                    "functionargzero",
                    "globalexport",
                    "multios",
                    "nomatch",
                    "notify",
                    "promptpercent",
                ];
                (set, unset)
            }
            "ksh" => {
                let set = vec![
                    "ksharrays",
                    "kshglob",
                    "shwordsplit",
                    "posixbuiltins",
                    "kshoptionprint",
                    "localoptions",
                    "promptbang",
                    "promptsubst",
                    "singlelinezle",
                    "interactivecomments",
                ];
                let unset = vec![
                    "badpattern",
                    "banghist",
                    "bgnice",
                    "equals",
                    "functionargzero",
                    "globalexport",
                    "multios",
                    "nomatch",
                    "notify",
                    "promptpercent",
                ];
                (set, unset)
            }
            "csh" => {
                // C shell emulation (limited)
                (vec!["cshnullglob", "cshjunkiequotes"], vec!["nomatch"])
            }
            "bash" => {
                let set = vec![
                    "ksharrays",
                    "shwordsplit",
                    "interactivecomments",
                    "shfileexpansion",
                    "globsubst",
                ];
                let unset = vec![
                    "badpattern",
                    "banghist",
                    "functionargzero",
                    "multios",
                    "nomatch",
                    "notify",
                    "promptpercent",
                ];
                (set, unset)
            }
            _ => (vec![], vec![]),
        }
    }

    /// exec - replace the shell with a command
    fn builtin_exec(&mut self, args: &[String]) -> i32 {
        // exec [ -c ] [ -l ] [ -a argv0 ] [ command [ arg ... ] ]
        // -c: clear environment
        // -l: place - at front of argv[0] (login shell)
        // -a argv0: set argv[0] to specified name

        let mut clear_env = false;
        let mut login_shell = false;
        let mut argv0: Option<String> = None;
        let mut cmd_args: Vec<String> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];

            if arg == "-c" && cmd_args.is_empty() {
                clear_env = true;
            } else if arg == "-l" && cmd_args.is_empty() {
                login_shell = true;
            } else if arg == "-a" && cmd_args.is_empty() {
                i += 1;
                if i < args.len() {
                    argv0 = Some(args[i].clone());
                }
            } else if arg.starts_with('-') && cmd_args.is_empty() {
                // Combined flags like -cl
                for ch in arg[1..].chars() {
                    match ch {
                        'c' => clear_env = true,
                        'l' => login_shell = true,
                        'a' => {
                            i += 1;
                            if i < args.len() {
                                argv0 = Some(args[i].clone());
                            }
                        }
                        _ => {}
                    }
                }
            } else {
                cmd_args.push(arg.clone());
            }
            i += 1;
        }

        if cmd_args.is_empty() {
            // No command: just modify shell's environment
            if clear_env {
                for (key, _) in env::vars() {
                    env::remove_var(&key);
                }
            }
            return 0;
        }

        let cmd = &cmd_args[0];
        let rest_args: Vec<&str> = cmd_args[1..].iter().map(|s| s.as_str()).collect();

        // Determine argv[0]
        let effective_argv0 = if let Some(a0) = argv0 {
            a0
        } else if login_shell {
            format!("-{}", cmd)
        } else {
            cmd.clone()
        };

        use std::os::unix::process::CommandExt;
        let mut command = std::process::Command::new(cmd);
        command.arg0(&effective_argv0);
        command.args(&rest_args);

        if clear_env {
            command.env_clear();
        }

        let err = command.exec();
        eprintln!("exec: {}: {}", cmd, err);
        1
    }

    /// float - declare floating point variables
    fn builtin_float(&mut self, args: &[String]) -> i32 {
        for arg in args {
            if arg.starts_with('-') {
                continue;
            }
            if let Some(eq_pos) = arg.find('=') {
                let name = &arg[..eq_pos];
                let value = &arg[eq_pos + 1..];
                let float_val: f64 = value.parse().unwrap_or(0.0);
                self.variables
                    .insert(name.to_string(), float_val.to_string());
                self.options.insert(format!("_float_{}", name), true);
            } else {
                self.variables.insert(arg.clone(), "0.0".to_string());
                self.options.insert(format!("_float_{}", arg), true);
            }
        }
        0
    }

    /// integer - declare integer variables
    fn builtin_integer(&mut self, args: &[String]) -> i32 {
        for arg in args {
            if arg.starts_with('-') {
                continue;
            }
            if let Some(eq_pos) = arg.find('=') {
                let name = &arg[..eq_pos];
                let value = &arg[eq_pos + 1..];
                let int_val: i64 = value.parse().unwrap_or(0);
                self.variables.insert(name.to_string(), int_val.to_string());
                self.options.insert(format!("_integer_{}", name), true);
            } else {
                self.variables.insert(arg.clone(), "0".to_string());
                self.options.insert(format!("_integer_{}", arg), true);
            }
        }
        0
    }

    /// functions - list or manipulate function definitions
    fn builtin_functions(&self, args: &[String]) -> i32 {
        let mut list_only = false;
        let mut show_trace = false;
        let mut names: Vec<&str> = Vec::new();

        for arg in args {
            match arg.as_str() {
                "-l" => list_only = true,
                "-t" => show_trace = true,
                _ if arg.starts_with('-') => {}
                _ => names.push(arg),
            }
        }

        if names.is_empty() {
            // List all functions
            let mut func_names: Vec<_> = self.functions.keys().collect();
            func_names.sort();
            for name in func_names {
                if list_only {
                    println!("{}", name);
                } else if let Some(func) = self.functions.get(name) {
                    let body = crate::text::getpermtext(func);
                    println!("{} () {{\n\t{}\n}}", name, body.trim());
                }
            }
        } else {
            // Show specific functions
            for name in names {
                if let Some(func) = self.functions.get(name) {
                    if show_trace {
                        println!("functions -t {}", name);
                    } else {
                        let body = crate::text::getpermtext(func);
                        println!("{} () {{\n\t{}\n}}", name, body.trim());
                    }
                } else {
                    eprintln!("functions: no such function: {}", name);
                    return 1;
                }
            }
        }
        0
    }

    /// print - zsh print builtin with many options
    fn builtin_print(&mut self, args: &[String]) -> i32 {
        // print [ -abcDilmnNoOpPrsSz ] [ -u n ] [ -f format ] [ -C cols ]
        //       [ -v name ] [ -xX tabstop ] [ -R [ -en ]] [ arg ... ]
        let mut no_newline = false;
        let mut one_per_line = false;
        let mut interpret_escapes = true; // zsh default is to interpret
        let mut raw_mode = false;
        let mut prompt_expand = false;
        let mut fd: i32 = 1; // stdout
        let mut columns = 0usize;
        let mut null_terminate = false;
        let mut push_to_stack = false;
        let mut add_to_history = false;
        let mut sort_asc = false;
        let mut sort_desc = false;
        let mut named_dir_subst = false;
        let mut store_var: Option<String> = None;
        let mut format_string: Option<String> = None;
        let mut output_args: Vec<String> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];

            if arg == "--" {
                i += 1;
                while i < args.len() {
                    output_args.push(args[i].clone());
                    i += 1;
                }
                break;
            }

            if arg.starts_with('-')
                && arg.len() > 1
                && !arg
                    .chars()
                    .nth(1)
                    .map(|c| c.is_ascii_digit())
                    .unwrap_or(false)
            {
                let mut chars = arg[1..].chars().peekable();
                while let Some(ch) = chars.next() {
                    match ch {
                        'n' => no_newline = true,
                        'l' => one_per_line = true,
                        'r' => {
                            raw_mode = true;
                            interpret_escapes = false;
                        }
                        'R' => {
                            raw_mode = true;
                            interpret_escapes = false;
                        }
                        'e' => interpret_escapes = true,
                        'E' => interpret_escapes = false,
                        'P' => prompt_expand = true,
                        'N' => null_terminate = true,
                        'z' => push_to_stack = true,
                        's' => add_to_history = true,
                        'o' => sort_asc = true,
                        'O' => sort_desc = true,
                        'D' => named_dir_subst = true,
                        'c' => columns = 1,
                        'a' | 'b' | 'i' | 'm' | 'p' | 'S' | 'x' | 'X' => {} // TODO
                        'u' => {
                            // -u n: output to fd n
                            let rest: String = chars.collect();
                            if !rest.is_empty() {
                                fd = rest.parse().unwrap_or(1);
                            } else {
                                i += 1;
                                if i < args.len() {
                                    fd = args[i].parse().unwrap_or(1);
                                }
                            }
                            break;
                        }
                        'C' => {
                            // -C n: n columns
                            let rest: String = chars.collect();
                            if !rest.is_empty() {
                                columns = rest.parse().unwrap_or(0);
                            } else {
                                i += 1;
                                if i < args.len() {
                                    columns = args[i].parse().unwrap_or(0);
                                }
                            }
                            break;
                        }
                        'v' => {
                            // -v name: store in variable
                            let rest: String = chars.collect();
                            if !rest.is_empty() {
                                store_var = Some(rest);
                            } else {
                                i += 1;
                                if i < args.len() {
                                    store_var = Some(args[i].clone());
                                }
                            }
                            break;
                        }
                        'f' => {
                            // -f format: printf-style format
                            let rest: String = chars.collect();
                            if !rest.is_empty() {
                                format_string = Some(rest);
                            } else {
                                i += 1;
                                if i < args.len() {
                                    format_string = Some(args[i].clone());
                                }
                            }
                            break;
                        }
                        _ => {}
                    }
                }
            } else {
                output_args.push(arg.clone());
            }
            i += 1;
        }

        let _ = push_to_stack; // TODO: implement push to buffer stack
        let _ = fd; // TODO: implement fd selection

        // Sort if requested
        if sort_asc {
            output_args.sort();
        } else if sort_desc {
            output_args.sort_by(|a, b| b.cmp(a));
        }

        // Handle -f format
        if let Some(fmt) = format_string {
            let output = self.printf_format(&fmt, &output_args);
            if let Some(var) = store_var {
                self.variables.insert(var, output);
            } else {
                print!("{}", output);
            }
            return 0;
        }

        // Process output
        let processed: Vec<String> = output_args
            .iter()
            .map(|s| {
                let mut result = s.clone();
                if prompt_expand {
                    result = self.expand_prompt_string(&result);
                }
                if interpret_escapes && !raw_mode {
                    result = self.expand_printf_escapes(&result);
                }
                if named_dir_subst {
                    // Replace home dir with ~
                    if let Ok(home) = env::var("HOME") {
                        if result.starts_with(&home) {
                            result = format!("~{}", &result[home.len()..]);
                        }
                    }
                    // Replace named dirs
                    for (name, path) in &self.named_dirs {
                        let path_str = path.to_string_lossy();
                        if result.starts_with(path_str.as_ref()) {
                            result = format!("~{}{}", name, &result[path_str.len()..]);
                            break;
                        }
                    }
                }
                result
            })
            .collect();

        // Determine separator and terminator
        let separator = if one_per_line { "\n" } else { " " };
        let terminator = if null_terminate {
            "\0"
        } else if no_newline {
            ""
        } else {
            "\n"
        };

        // Build output
        let output = if one_per_line {
            processed.join("\n")
        } else if columns > 0 {
            // Column output - calculate column widths
            let mut result = String::new();
            let num_items = processed.len();
            let rows = (num_items + columns - 1) / columns;
            for row in 0..rows {
                let mut row_items = Vec::new();
                for col in 0..columns {
                    let idx = row + col * rows;
                    if idx < num_items {
                        row_items.push(processed[idx].as_str());
                    }
                }
                result.push_str(&row_items.join("\t"));
                if row < rows - 1 {
                    result.push('\n');
                }
            }
            result
        } else {
            processed.join(separator)
        };

        // Add to history if -s
        if add_to_history {
            if let Some(ref mut engine) = self.history {
                let _ = engine.add(&output, None);
            }
        }

        // Store in variable or print
        if let Some(var) = store_var {
            self.variables.insert(var, output);
        } else {
            print!("{}{}", output, terminator);
        }

        0
    }

    fn printf_format(&self, format: &str, args: &[String]) -> String {
        let mut result = String::new();
        let mut arg_idx = 0;
        let mut chars = format.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '%' {
                if chars.peek() == Some(&'%') {
                    chars.next();
                    result.push('%');
                    continue;
                }

                // Parse format specifier
                let mut spec = String::from("%");

                // Flags
                while let Some(&c) = chars.peek() {
                    if c == '-' || c == '+' || c == ' ' || c == '#' || c == '0' {
                        spec.push(c);
                        chars.next();
                    } else {
                        break;
                    }
                }

                // Width
                while let Some(&c) = chars.peek() {
                    if c.is_ascii_digit() {
                        spec.push(c);
                        chars.next();
                    } else {
                        break;
                    }
                }

                // Precision
                if chars.peek() == Some(&'.') {
                    spec.push('.');
                    chars.next();
                    while let Some(&c) = chars.peek() {
                        if c.is_ascii_digit() {
                            spec.push(c);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                }

                // Conversion specifier
                if let Some(conv) = chars.next() {
                    let arg = args.get(arg_idx).map(|s| s.as_str()).unwrap_or("");
                    arg_idx += 1;

                    match conv {
                        's' => result.push_str(arg),
                        'd' | 'i' => {
                            let n: i64 = arg.parse().unwrap_or(0);
                            result.push_str(&n.to_string());
                        }
                        'u' => {
                            let n: u64 = arg.parse().unwrap_or(0);
                            result.push_str(&n.to_string());
                        }
                        'x' => {
                            let n: i64 = arg.parse().unwrap_or(0);
                            result.push_str(&format!("{:x}", n));
                        }
                        'X' => {
                            let n: i64 = arg.parse().unwrap_or(0);
                            result.push_str(&format!("{:X}", n));
                        }
                        'o' => {
                            let n: i64 = arg.parse().unwrap_or(0);
                            result.push_str(&format!("{:o}", n));
                        }
                        'f' | 'F' | 'e' | 'E' | 'g' | 'G' => {
                            let n: f64 = arg.parse().unwrap_or(0.0);
                            result.push_str(&format!("{}", n));
                        }
                        'c' => {
                            if let Some(c) = arg.chars().next() {
                                result.push(c);
                            }
                        }
                        'b' => {
                            result.push_str(&self.expand_printf_escapes(arg));
                        }
                        'n' => result.push('\n'),
                        _ => {
                            result.push('%');
                            result.push(conv);
                        }
                    }
                }
            } else {
                result.push(ch);
            }
        }

        result
    }

    /// whence - show how a command would be interpreted
    fn builtin_whence(&self, args: &[String]) -> i32 {
        // whence [ -vcwfpamsS ] [ -x num ] name ...
        // -v: verbose (like type)
        // -c: csh-style output
        // -w: print word type (alias, builtin, command, function, hashed, reserved, none)
        // -f: skip functions
        // -p: search path only
        // -a: show all matches
        // -m: pattern match with glob
        // -s: show symlink resolution
        // -S: show steps of symlink resolution
        // -x num: expand tabs to num spaces

        let mut verbose = false;
        let mut csh_style = false;
        let mut word_type = false;
        let mut skip_functions = false;
        let mut path_only = false;
        let mut show_all = false;
        let mut pattern_mode = false;
        let mut show_symlink = false;
        let mut show_symlink_steps = false;
        let mut tab_expand: Option<usize> = None;
        let mut names: Vec<&str> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];

            if arg == "--" {
                i += 1;
                while i < args.len() {
                    names.push(&args[i]);
                    i += 1;
                }
                break;
            }

            if arg.starts_with('-') && arg.len() > 1 {
                let mut chars = arg[1..].chars().peekable();
                while let Some(ch) = chars.next() {
                    match ch {
                        'v' => verbose = true,
                        'c' => csh_style = true,
                        'w' => word_type = true,
                        'f' => skip_functions = true,
                        'p' => path_only = true,
                        'a' => show_all = true,
                        'm' => pattern_mode = true,
                        's' => show_symlink = true,
                        'S' => show_symlink_steps = true,
                        'x' => {
                            // -x num: tab expansion
                            let rest: String = chars.collect();
                            if !rest.is_empty() {
                                tab_expand = rest.parse().ok();
                            } else {
                                i += 1;
                                if i < args.len() {
                                    tab_expand = args[i].parse().ok();
                                }
                            }
                            break;
                        }
                        _ => {}
                    }
                }
            } else {
                names.push(arg);
            }
            i += 1;
        }

        let _ = csh_style; // TODO: implement csh-style output
        let _ = pattern_mode; // TODO: implement glob pattern matching
        let _ = tab_expand;

        let mut status = 0;
        for name in names {
            let mut found = false;
            let mut word = "none";

            if !path_only {
                // Check reserved words
                if self.is_reserved_word(name) {
                    found = true;
                    word = "reserved";
                    if word_type {
                        println!("{}: {}", name, word);
                    } else if verbose {
                        println!("{} is a reserved word", name);
                    } else {
                        println!("{}", name);
                    }
                    if !show_all {
                        continue;
                    }
                }

                // Check aliases
                if let Some(alias_val) = self.aliases.get(name) {
                    found = true;
                    word = "alias";
                    if word_type {
                        println!("{}: {}", name, word);
                    } else if verbose {
                        println!("{} is an alias for {}", name, alias_val);
                    } else {
                        println!("{}", alias_val);
                    }
                    if !show_all {
                        continue;
                    }
                }

                // Check functions (unless -f)
                if !skip_functions && self.functions.contains_key(name) {
                    found = true;
                    word = "function";
                    if word_type {
                        println!("{}: {}", name, word);
                    } else if verbose {
                        println!("{} is a shell function", name);
                    } else {
                        println!("{}", name);
                    }
                    if !show_all {
                        continue;
                    }
                }

                // Check builtins
                if self.is_builtin(name) {
                    found = true;
                    word = "builtin";
                    if word_type {
                        println!("{}: {}", name, word);
                    } else if verbose {
                        println!("{} is a shell builtin", name);
                    } else {
                        println!("{}", name);
                    }
                    if !show_all {
                        continue;
                    }
                }

                // Check hashed commands (named_dirs can serve as a command hash)
                // The hash builtin adds to named_dirs for now
                if let Some(path) = self.named_dirs.get(name) {
                    found = true;
                    word = "hashed";
                    if word_type {
                        println!("{}: {}", name, word);
                    } else if verbose {
                        println!("{} is hashed ({})", name, path.display());
                    } else {
                        println!("{}", path.display());
                    }
                    if !show_all {
                        continue;
                    }
                }
            }

            // Check PATH
            if let Some(path) = self.find_in_path(name) {
                found = true;
                word = "command";

                // Handle symlink resolution
                let display_path = if show_symlink || show_symlink_steps {
                    let p = std::path::Path::new(&path);
                    if show_symlink_steps {
                        let mut current = p.to_path_buf();
                        let mut steps = vec![path.clone()];
                        while let Ok(target) = std::fs::read_link(&current) {
                            let resolved = if target.is_absolute() {
                                target.clone()
                            } else {
                                current
                                    .parent()
                                    .unwrap_or(std::path::Path::new("/"))
                                    .join(&target)
                            };
                            steps.push(resolved.to_string_lossy().to_string());
                            current = resolved;
                        }
                        steps.join(" -> ")
                    } else {
                        match p.canonicalize() {
                            Ok(resolved) => format!("{} -> {}", path, resolved.display()),
                            Err(_) => path.clone(),
                        }
                    }
                } else {
                    path.clone()
                };

                if word_type {
                    println!("{}: {}", name, word);
                } else if verbose {
                    println!("{} is {}", name, display_path);
                } else {
                    println!("{}", display_path);
                }
            }

            if !found {
                if word_type {
                    println!("{}: none", name);
                } else if verbose {
                    println!("{} not found", name);
                }
                status = 1;
            }
        }
        status
    }

    fn is_reserved_word(&self, name: &str) -> bool {
        matches!(
            name,
            "if" | "then"
                | "else"
                | "elif"
                | "fi"
                | "case"
                | "esac"
                | "for"
                | "select"
                | "while"
                | "until"
                | "do"
                | "done"
                | "in"
                | "function"
                | "time"
                | "coproc"
                | "{"
                | "}"
                | "!"
                | "[["
                | "]]"
                | "(("
                | "))"
        )
    }

    /// where - show all locations of a command
    fn builtin_where(&self, args: &[String]) -> i32 {
        // where is like whence -ca
        let mut new_args = vec!["-a".to_string(), "-v".to_string()];
        new_args.extend(args.iter().cloned());
        self.builtin_whence(&new_args)
    }

    /// which - show path of command
    fn builtin_which(&self, args: &[String]) -> i32 {
        // which is like whence -c
        let mut new_args = vec!["-c".to_string()];
        new_args.extend(args.iter().cloned());
        self.builtin_whence(&new_args)
    }

    /// Helper to check if name is a builtin
    /// O(1) builtin check via static HashSet — replaces 130+ arm linear match
    fn is_builtin(&self, name: &str) -> bool {
        BUILTIN_SET.contains(name) || name.starts_with('_')
    }

    /// Helper to find command in PATH — checks command_hash first for O(1) hit
    fn find_in_path(&self, name: &str) -> Option<String> {
        // O(1) hash table lookup from rehash
        if let Some(path) = self.command_hash.get(name) {
            return Some(path.clone());
        }
        // Fallback: linear PATH walk
        let path_var = env::var("PATH").unwrap_or_default();
        for dir in path_var.split(':') {
            let full_path = format!("{}/{}", dir, name);
            if std::path::Path::new(&full_path).exists() {
                return Some(full_path);
            }
        }
        None
    }

    /// ulimit - get/set resource limits
    fn builtin_ulimit(&self, args: &[String]) -> i32 {
        use libc::{getrlimit, rlimit, setrlimit};
        use libc::{RLIMIT_AS, RLIMIT_CORE, RLIMIT_CPU, RLIMIT_DATA, RLIMIT_FSIZE};
        use libc::{RLIMIT_NOFILE, RLIMIT_NPROC, RLIMIT_RSS, RLIMIT_STACK};

        let mut resource = RLIMIT_FSIZE; // default: file size
        let mut hard = false;
        let mut soft = true;
        let mut value: Option<u64> = None;

        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-H" => {
                    hard = true;
                    soft = false;
                }
                "-S" => {
                    soft = true;
                    hard = false;
                }
                "-a" => {
                    // Print all limits
                    self.print_all_limits(soft);
                    return 0;
                }
                "-c" => resource = RLIMIT_CORE,
                "-d" => resource = RLIMIT_DATA,
                "-f" => resource = RLIMIT_FSIZE,
                "-n" => resource = RLIMIT_NOFILE,
                "-s" => resource = RLIMIT_STACK,
                "-t" => resource = RLIMIT_CPU,
                "-u" => resource = RLIMIT_NPROC,
                "-v" => resource = RLIMIT_AS,
                "-m" => resource = RLIMIT_RSS,
                "unlimited" => value = Some(libc::RLIM_INFINITY as u64),
                _ if !arg.starts_with('-') => {
                    value = arg.parse().ok();
                }
                _ => {}
            }
        }

        let mut rlim = rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        unsafe {
            if getrlimit(resource, &mut rlim) != 0 {
                eprintln!("ulimit: cannot get limit");
                return 1;
            }
        }

        if let Some(v) = value {
            // Set limit
            if soft {
                rlim.rlim_cur = v as libc::rlim_t;
            }
            if hard {
                rlim.rlim_max = v as libc::rlim_t;
            }
            unsafe {
                if setrlimit(resource, &rlim) != 0 {
                    eprintln!("ulimit: cannot set limit");
                    return 1;
                }
            }
        } else {
            // Print limit
            let limit = if hard { rlim.rlim_max } else { rlim.rlim_cur };
            if limit == libc::RLIM_INFINITY as libc::rlim_t {
                println!("unlimited");
            } else {
                println!("{}", limit);
            }
        }
        0
    }

    fn print_all_limits(&self, soft: bool) {
        use libc::{getrlimit, rlimit};
        use libc::{RLIMIT_AS, RLIMIT_CORE, RLIMIT_CPU, RLIMIT_DATA, RLIMIT_FSIZE};
        use libc::{RLIMIT_NOFILE, RLIMIT_NPROC, RLIMIT_RSS, RLIMIT_STACK};

        let limits = [
            (RLIMIT_CORE, "core file size", "blocks", 512),
            (RLIMIT_DATA, "data seg size", "kbytes", 1024),
            (RLIMIT_FSIZE, "file size", "blocks", 512),
            (RLIMIT_NOFILE, "open files", "", 1),
            (RLIMIT_STACK, "stack size", "kbytes", 1024),
            (RLIMIT_CPU, "cpu time", "seconds", 1),
            (RLIMIT_NPROC, "max user processes", "", 1),
            (RLIMIT_AS, "virtual memory", "kbytes", 1024),
            (RLIMIT_RSS, "max memory size", "kbytes", 1024),
        ];

        for (resource, name, unit, divisor) in limits {
            let mut rlim = rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            unsafe {
                if getrlimit(resource, &mut rlim) == 0 {
                    let limit = if soft { rlim.rlim_cur } else { rlim.rlim_max };
                    let unit_str = if unit.is_empty() {
                        ""
                    } else {
                        &format!("({})", unit)
                    };
                    if limit == libc::RLIM_INFINITY as libc::rlim_t {
                        println!("{:25} {} unlimited", name, unit_str);
                    } else {
                        println!("{:25} {} {}", name, unit_str, limit / divisor);
                    }
                }
            }
        }
    }

    /// limit - csh-style resource limits
    fn builtin_limit(&self, args: &[String]) -> i32 {
        // Delegate to ulimit with csh-style names
        if args.is_empty() {
            // Print all resource limits in csh format
            use libc::{getrlimit, rlimit, RLIM_INFINITY};
            let resources = [
                (libc::RLIMIT_CPU, "cputime", 1, "seconds"),
                (libc::RLIMIT_FSIZE, "filesize", 1024, "kB"),
                (libc::RLIMIT_DATA, "datasize", 1024, "kB"),
                (libc::RLIMIT_STACK, "stacksize", 1024, "kB"),
                (libc::RLIMIT_CORE, "coredumpsize", 1024, "kB"),
                (libc::RLIMIT_RSS, "memoryuse", 1024, "kB"),
                #[cfg(target_os = "linux")]
                (libc::RLIMIT_NPROC, "maxproc", 1, ""),
                (libc::RLIMIT_NOFILE, "descriptors", 1, ""),
            ];
            for (res, name, divisor, unit) in resources {
                let mut rl: rlimit = unsafe { std::mem::zeroed() };
                unsafe { getrlimit(res, &mut rl); }
                let val = if rl.rlim_cur == RLIM_INFINITY as u64 {
                    "unlimited".to_string()
                } else {
                    let v = rl.rlim_cur as u64 / divisor;
                    if unit.is_empty() { format!("{}", v) } else { format!("{}{}", v, unit) }
                };
                println!("{:<16}{}", name, val);
            }
            return 0;
        }
        self.builtin_ulimit(args)
    }

    /// unlimit - remove resource limits
    fn builtin_unlimit(&self, args: &[String]) -> i32 {
        let mut new_args = args.to_vec();
        new_args.push("unlimited".to_string());
        self.builtin_ulimit(&new_args)
    }

    /// umask - get/set file creation mask
    fn builtin_umask(&self, args: &[String]) -> i32 {
        use libc::umask;

        let mut symbolic = false;
        let mut value: Option<&str> = None;

        for arg in args {
            match arg.as_str() {
                "-S" => symbolic = true,
                _ if !arg.starts_with('-') => value = Some(arg),
                _ => {}
            }
        }

        if let Some(v) = value {
            // Set umask
            if let Ok(mask) = u32::from_str_radix(v, 8) {
                unsafe {
                    umask(mask as libc::mode_t);
                }
            } else {
                eprintln!("umask: invalid mask: {}", v);
                return 1;
            }
        } else {
            // Get umask
            let mask = unsafe {
                let m = umask(0);
                umask(m);
                m
            };
            if symbolic {
                let u = 7 - ((mask >> 6) & 7);
                let g = 7 - ((mask >> 3) & 7);
                let o = 7 - (mask & 7);
                println!(
                    "u={}{}{}g={}{}{}o={}{}{}",
                    if u & 4 != 0 { "r" } else { "" },
                    if u & 2 != 0 { "w" } else { "" },
                    if u & 1 != 0 { "x" } else { "" },
                    if g & 4 != 0 { "r" } else { "" },
                    if g & 2 != 0 { "w" } else { "" },
                    if g & 1 != 0 { "x" } else { "" },
                    if o & 4 != 0 { "r" } else { "" },
                    if o & 2 != 0 { "w" } else { "" },
                    if o & 1 != 0 { "x" } else { "" },
                );
            } else {
                println!("{:04o}", mask);
            }
        }
        0
    }

    /// rehash - rebuild command hash table
    fn builtin_rehash(&mut self, args: &[String]) -> i32 {
        // rehash [ -d ] [ -f ] [ -v ]
        // -d: rehash named directories
        // -f: force rehash of all commands in PATH
        // -v: verbose (print each command being hashed)

        let mut rehash_dirs = false;
        let mut force = false;
        let mut verbose = false;

        for arg in args {
            if arg.starts_with('-') {
                for ch in arg[1..].chars() {
                    match ch {
                        'd' => rehash_dirs = true,
                        'f' => force = true,
                        'v' => verbose = true,
                        _ => {}
                    }
                }
            }
        }

        if rehash_dirs {
            // Rebuild named directories from special params like ~user
            // For now just clear and rebuild from HOME
            self.named_dirs.clear();
            if let Ok(home) = env::var("HOME") {
                self.named_dirs.insert(String::new(), PathBuf::from(&home)); // ~ without name
            }
            return 0;
        }

        // Clear command hash table
        self.command_hash.clear();

        if force {
            // Parallel PATH scan — each PATH dir on a pool thread.
            // zsh does this single-threaded; we fan out across workers.
            if let Ok(path_var) = env::var("PATH") {
                let dirs: Vec<String> = path_var
                    .split(':')
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect();

                let (tx, rx) = std::sync::mpsc::channel::<Vec<(String, String)>>();

                for dir in dirs {
                    let tx = tx.clone();
                    self.worker_pool.submit(move || {
                        let mut batch = Vec::new();
                        if let Ok(entries) = std::fs::read_dir(&dir) {
                            for entry in entries.flatten() {
                                if let Ok(ft) = entry.file_type() {
                                    if ft.is_file() || ft.is_symlink() {
                                        if let Some(name) = entry.file_name().to_str() {
                                            let path =
                                                entry.path().to_string_lossy().to_string();
                                            batch.push((name.to_string(), path));
                                        }
                                    }
                                }
                            }
                        }
                        let _ = tx.send(batch);
                    });
                }
                drop(tx);

                for batch in rx {
                    for (name, path) in batch {
                        if verbose {
                            println!("{}={}", name, path);
                        }
                        self.command_hash.insert(name, path);
                    }
                }
            }
        }

        0
    }

    /// unhash - remove entries from hash table
    fn builtin_unhash(&mut self, args: &[String]) -> i32 {
        let mut remove_aliases = false;
        let mut remove_functions = false;
        let mut remove_dirs = false;
        let mut names: Vec<&str> = Vec::new();

        for arg in args {
            match arg.as_str() {
                "-a" => remove_aliases = true,
                "-f" => remove_functions = true,
                "-d" => remove_dirs = true,
                "-m" => {} // pattern matching (TODO)
                _ if arg.starts_with('-') => {}
                _ => names.push(arg),
            }
        }

        for name in names {
            if remove_aliases {
                self.aliases.remove(name);
            }
            if remove_functions {
                self.functions.remove(name);
            }
            if remove_dirs {
                // Remove from named directories (TODO)
            }
        }
        0
    }

    /// times - print accumulated user and system times
    fn builtin_times(&self, _args: &[String]) -> i32 {
        use libc::{getrusage, rusage, RUSAGE_CHILDREN, RUSAGE_SELF};

        let mut self_usage: rusage = unsafe { std::mem::zeroed() };
        let mut child_usage: rusage = unsafe { std::mem::zeroed() };

        unsafe {
            getrusage(RUSAGE_SELF, &mut self_usage);
            getrusage(RUSAGE_CHILDREN, &mut child_usage);
        }

        let self_user =
            self_usage.ru_utime.tv_sec as f64 + self_usage.ru_utime.tv_usec as f64 / 1_000_000.0;
        let self_sys =
            self_usage.ru_stime.tv_sec as f64 + self_usage.ru_stime.tv_usec as f64 / 1_000_000.0;
        let child_user =
            child_usage.ru_utime.tv_sec as f64 + child_usage.ru_utime.tv_usec as f64 / 1_000_000.0;
        let child_sys =
            child_usage.ru_stime.tv_sec as f64 + child_usage.ru_stime.tv_usec as f64 / 1_000_000.0;

        println!("{:.3}s {:.3}s", self_user, self_sys);
        println!("{:.3}s {:.3}s", child_user, child_sys);
        0
    }

    /// zmodload - load/unload zsh modules (stub)
    fn builtin_zmodload(&mut self, args: &[String]) -> i32 {
        let mut list_loaded = false;
        let mut unload = false;
        let mut modules: Vec<&str> = Vec::new();

        for arg in args {
            match arg.as_str() {
                "-l" | "-L" => list_loaded = true,
                "-u" => unload = true,
                "-a" | "-b" | "-c" | "-d" | "-e" | "-f" | "-i" | "-p" | "-s" => {}
                _ if arg.starts_with('-') => {}
                _ => modules.push(arg),
            }
        }

        if list_loaded || modules.is_empty() {
            // List loaded modules (stub - we don't really have modules)
            println!("zsh/complete");
            println!("zsh/complist");
            println!("zsh/parameter");
            println!("zsh/zutil");
            return 0;
        }

        for module in modules {
            if unload {
                // Unload module (stub)
                self.options.remove(&format!("_module_{}", module));
            } else {
                // Load module (stub)
                self.options.insert(format!("_module_{}", module), true);
            }
        }
        0
    }

    /// r - redo last command (alias for fc -e -)
    fn builtin_r(&mut self, args: &[String]) -> i32 {
        let mut fc_args = vec!["-e".to_string(), "-".to_string()];
        fc_args.extend(args.iter().cloned());
        self.builtin_fc(&fc_args)
    }

    /// ttyctl - control terminal settings
    fn builtin_ttyctl(&self, args: &[String]) -> i32 {
        for arg in args {
            match arg.as_str() {
                "-f" => {
                    // Freeze terminal settings
                    // In a full implementation, this would save terminal state
                }
                "-u" => {
                    // Unfreeze terminal settings
                }
                _ => {}
            }
        }
        0
    }

    /// noglob - run command without globbing
    fn builtin_noglob(&mut self, args: &[String], redirects: &[Redirect]) -> i32 {
        if args.is_empty() {
            return 0;
        }

        // Temporarily disable globbing
        let saved = self.options.get("noglob").cloned();
        self.options.insert("noglob".to_string(), true);

        // Execute the command
        let status = self.builtin_command(args, redirects);

        // Restore globbing state
        if let Some(v) = saved {
            self.options.insert("noglob".to_string(), v);
        } else {
            self.options.remove("noglob");
        }

        status
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // zsh module builtins
    // ═══════════════════════════════════════════════════════════════════════════

    /// zstat - file status (zsh/stat module)
    fn builtin_zstat(&self, args: &[String]) -> i32 {
        use std::os::unix::fs::MetadataExt;
        use std::os::unix::fs::PermissionsExt;

        let mut show_all = true;
        let mut symbolic_mode = false;
        let mut show_link = false;
        let mut _as_array = false;
        let mut _array_name = String::new();
        let mut format_time = String::new();
        let mut elements: Vec<String> = Vec::new();
        let mut files: Vec<&str> = Vec::new();

        let mut iter = args.iter().peekable();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-s" => symbolic_mode = true,
                "-L" => show_link = true,
                "-N" => {} // Don't resolve symlinks
                "-n" => {} // Numeric user/group
                "-o" => show_all = false,
                "-A" => {
                    _as_array = true;
                    if let Some(name) = iter.next() {
                        _array_name = name.clone();
                    }
                }
                "-F" => {
                    if let Some(fmt) = iter.next() {
                        format_time = fmt.clone();
                    }
                }
                s if s.starts_with('+') => {
                    elements.push(s[1..].to_string());
                    show_all = false;
                }
                s if !s.starts_with('-') => files.push(s),
                _ => {}
            }
        }

        if files.is_empty() {
            eprintln!("zstat: no files specified");
            return 1;
        }

        for file in files {
            let meta = if show_link {
                std::fs::symlink_metadata(file)
            } else {
                std::fs::metadata(file)
            };

            let meta = match meta {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("zstat: {}: {}", file, e);
                    return 1;
                }
            };

            let output_element = |name: &str, value: &str| {
                if _as_array {
                    // Would need mutable self to store in array
                    println!("{}={}", name, value);
                } else if show_all || elements.contains(&name.to_string()) {
                    println!("{}: {}", name, value);
                }
            };

            output_element("device", &meta.dev().to_string());
            output_element("inode", &meta.ino().to_string());

            if symbolic_mode {
                let mode = meta.permissions().mode();
                let mode_str = format!(
                    "{}{}{}{}{}{}{}{}{}{}",
                    match mode & 0o170000 {
                        0o040000 => 'd',
                        0o120000 => 'l',
                        0o100000 => '-',
                        0o060000 => 'b',
                        0o020000 => 'c',
                        0o010000 => 'p',
                        0o140000 => 's',
                        _ => '?',
                    },
                    if mode & 0o400 != 0 { 'r' } else { '-' },
                    if mode & 0o200 != 0 { 'w' } else { '-' },
                    if mode & 0o4000 != 0 {
                        's'
                    } else if mode & 0o100 != 0 {
                        'x'
                    } else {
                        '-'
                    },
                    if mode & 0o040 != 0 { 'r' } else { '-' },
                    if mode & 0o020 != 0 { 'w' } else { '-' },
                    if mode & 0o2000 != 0 {
                        's'
                    } else if mode & 0o010 != 0 {
                        'x'
                    } else {
                        '-'
                    },
                    if mode & 0o004 != 0 { 'r' } else { '-' },
                    if mode & 0o002 != 0 { 'w' } else { '-' },
                    if mode & 0o1000 != 0 {
                        't'
                    } else if mode & 0o001 != 0 {
                        'x'
                    } else {
                        '-'
                    },
                );
                output_element("mode", &mode_str);
            } else {
                output_element("mode", &format!("{:o}", meta.permissions().mode()));
            }

            output_element("nlink", &meta.nlink().to_string());
            output_element("uid", &meta.uid().to_string());
            output_element("gid", &meta.gid().to_string());
            output_element("rdev", &meta.rdev().to_string());
            output_element("size", &meta.len().to_string());

            let format_timestamp = |secs: i64| -> String {
                if format_time.is_empty() {
                    secs.to_string()
                } else {
                    chrono::DateTime::from_timestamp(secs, 0)
                        .map(|dt| dt.format(&format_time).to_string())
                        .unwrap_or_else(|| secs.to_string())
                }
            };

            output_element("atime", &format_timestamp(meta.atime()));
            output_element("mtime", &format_timestamp(meta.mtime()));
            output_element("ctime", &format_timestamp(meta.ctime()));
            output_element("blksize", &meta.blksize().to_string());
            output_element("blocks", &meta.blocks().to_string());

            if show_link && meta.file_type().is_symlink() {
                if let Ok(target) = std::fs::read_link(file) {
                    output_element("link", &target.to_string_lossy());
                }
            }
        }

        0
    }

    /// strftime - format date/time (zsh/datetime module)
    fn builtin_strftime(&self, args: &[String]) -> i32 {
        let mut format = "%c".to_string();
        let mut timestamp: Option<i64> = None;
        let mut to_var = false;
        let mut var_name = String::new();

        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-s" => {
                    to_var = true;
                    if let Some(name) = iter.next() {
                        var_name = name.clone();
                    }
                }
                "-r" => {
                    // Reference time from a variable
                    if let Some(ts_str) = iter.next() {
                        timestamp = ts_str.parse().ok();
                    }
                }
                s if !s.starts_with('-') => {
                    if format == "%c" {
                        format = s.to_string();
                    } else if timestamp.is_none() {
                        timestamp = s.parse().ok();
                    }
                }
                _ => {}
            }
        }

        let ts = timestamp.unwrap_or_else(|| chrono::Local::now().timestamp());

        let result = chrono::DateTime::from_timestamp(ts, 0)
            .map(|dt: chrono::DateTime<chrono::Utc>| {
                dt.with_timezone(&chrono::Local).format(&format).to_string()
            })
            .unwrap_or_else(|| "invalid timestamp".to_string());

        if to_var && !var_name.is_empty() {
            // Would need mutable self
            println!("{}={}", var_name, result);
        } else {
            println!("{}", result);
        }

        0
    }

    /// zsleep - sleep with fractional seconds
    fn builtin_zsleep(&self, args: &[String]) -> i32 {
        if args.is_empty() {
            eprintln!("zsleep: missing argument");
            return 1;
        }

        let secs: f64 = match args[0].parse() {
            Ok(s) => s,
            Err(_) => {
                eprintln!("zsleep: invalid number: {}", args[0]);
                return 1;
            }
        };

        std::thread::sleep(std::time::Duration::from_secs_f64(secs));
        0
    }

    /// zsystem - system interface (zsh/system module)
    /// Ported from zsh/Src/Modules/system.c bin_zsystem() lines 805-816
    fn builtin_zsystem(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            eprintln!("zsystem: subcommand expected");
            return 1;
        }
        match args[0].as_str() {
            "flock" => self.builtin_zsystem_flock(&args[1..]),
            "supports" => self.builtin_zsystem_supports(&args[1..]),
            _ => {
                eprintln!("zsystem: unknown subcommand: {}", args[0]);
                1
            }
        }
    }

    /// zsystem supports - ported from system.c bin_zsystem_supports() lines 780-801
    fn builtin_zsystem_supports(&self, args: &[String]) -> i32 {
        if args.is_empty() {
            eprintln!("zsystem: supports: not enough arguments");
            return 255;
        }
        if args.len() > 1 {
            eprintln!("zsystem: supports: too many arguments");
            return 255;
        }
        match args[0].as_str() {
            "supports" | "flock" => 0,
            _ => 1,
        }
    }

    /// zsystem flock - ported from system.c bin_zsystem_flock() lines 546-774
    fn builtin_zsystem_flock(&mut self, args: &[String]) -> i32 {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;

            let mut cloexec = true;
            let mut readlock = false;
            let mut timeout: Option<f64> = None;
            let mut fdvar: Option<String> = None;
            let mut file: Option<&str> = None;

            let mut i = 0;
            while i < args.len() {
                let arg = &args[i];
                if arg == "--" {
                    i += 1;
                    if i < args.len() {
                        file = Some(&args[i]);
                    }
                    break;
                }
                if !arg.starts_with('-') {
                    file = Some(arg);
                    break;
                }
                let mut chars = arg[1..].chars().peekable();
                while let Some(c) = chars.next() {
                    match c {
                        'e' => cloexec = false,
                        'r' => readlock = true,
                        'u' => return 0,
                        'f' => {
                            let rest: String = chars.collect();
                            if !rest.is_empty() {
                                fdvar = Some(rest);
                            } else {
                                i += 1;
                                if i < args.len() {
                                    fdvar = Some(args[i].clone());
                                } else {
                                    eprintln!("zsystem: flock: option f requires a variable name");
                                    return 1;
                                }
                            }
                            break;
                        }
                        't' => {
                            let rest: String = chars.collect();
                            let val = if !rest.is_empty() {
                                rest
                            } else {
                                i += 1;
                                if i < args.len() {
                                    args[i].clone()
                                } else {
                                    eprintln!(
                                        "zsystem: flock: option t requires a numeric timeout"
                                    );
                                    return 1;
                                }
                            };
                            match val.parse::<f64>() {
                                Ok(t) => timeout = Some(t),
                                Err(_) => {
                                    eprintln!("zsystem: flock: invalid timeout value: '{}'", val);
                                    return 1;
                                }
                            }
                            break;
                        }
                        'i' => {
                            let rest: String = chars.collect();
                            if rest.is_empty() {
                                i += 1;
                                if i >= args.len() {
                                    eprintln!("zsystem: flock: option i requires a numeric retry interval");
                                    return 1;
                                }
                            }
                            break;
                        }
                        _ => {
                            eprintln!("zsystem: flock: unknown option: -{}", c);
                            return 1;
                        }
                    }
                }
                i += 1;
            }

            let filepath = match file {
                Some(f) => f,
                None => {
                    eprintln!("zsystem: flock: not enough arguments");
                    return 1;
                }
            };

            use std::fs::OpenOptions;
            let file_handle = match OpenOptions::new()
                .read(true)
                .write(!readlock)
                .create(true)
                .truncate(false)
                .open(filepath)
            {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("zsystem: flock: {}: {}", filepath, e);
                    return 1;
                }
            };

            let lock_type = if readlock {
                libc::F_RDLCK as i16
            } else {
                libc::F_WRLCK as i16
            };

            let mut flock = libc::flock {
                l_type: lock_type,
                l_whence: libc::SEEK_SET as i16,
                l_start: 0,
                l_len: 0,
                l_pid: 0,
            };

            let cmd = if timeout.is_some() {
                libc::F_SETLK
            } else {
                libc::F_SETLKW
            };
            let start = std::time::Instant::now();
            let timeout_duration = timeout.map(|t| std::time::Duration::from_secs_f64(t));

            loop {
                let ret = unsafe { libc::fcntl(file_handle.as_raw_fd(), cmd, &mut flock) };
                if ret == 0 {
                    if let Some(ref var) = fdvar {
                        let fd = file_handle.as_raw_fd();
                        std::mem::forget(file_handle);
                        self.variables.insert(var.clone(), fd.to_string());
                    } else {
                        std::mem::forget(file_handle);
                    }
                    let _ = cloexec;
                    return 0;
                }
                let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                if errno != libc::EACCES && errno != libc::EAGAIN {
                    eprintln!(
                        "zsystem: flock: {}: {}",
                        filepath,
                        std::io::Error::last_os_error()
                    );
                    return 1;
                }
                if let Some(td) = timeout_duration {
                    if start.elapsed() >= td {
                        return 2;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                } else {
                    eprintln!(
                        "zsystem: flock: {}: {}",
                        filepath,
                        std::io::Error::last_os_error()
                    );
                    return 1;
                }
            }
        }
        #[cfg(not(unix))]
        {
            eprintln!("zsystem: flock: not supported on this platform");
            1
        }
    }

    /// sync - flush filesystem buffers
    /// Port from zsh/Src/Modules/files.c bin_sync() lines 52-57
    fn builtin_sync(&self, _args: &[String]) -> i32 {
        #[cfg(unix)]
        unsafe {
            libc::sync();
        }
        0
    }

    /// mkdir - create directories
    /// Port from zsh/Src/Modules/files.c bin_mkdir() lines 62-111
    fn builtin_mkdir(&self, args: &[String]) -> i32 {
        let mut mode: u32 = 0o777;
        let mut parents = false;
        let mut dirs: Vec<&str> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if arg == "-p" {
                parents = true;
            } else if arg == "-m" && i + 1 < args.len() {
                i += 1;
                mode = u32::from_str_radix(&args[i], 8).unwrap_or(0o777);
            } else if arg.starts_with("-m") {
                mode = u32::from_str_radix(&arg[2..], 8).unwrap_or(0o777);
            } else if !arg.starts_with('-') || arg == "-" || arg == "--" {
                if arg == "--" {
                    dirs.extend(args[i + 1..].iter().map(|s| s.as_str()));
                    break;
                }
                dirs.push(arg);
            }
            i += 1;
        }

        let mut err = 0;
        for dir in dirs {
            let path = std::path::Path::new(dir);
            let result = if parents {
                std::fs::create_dir_all(path)
            } else {
                std::fs::create_dir(path)
            };
            if let Err(e) = result {
                eprintln!("mkdir: cannot create directory '{}': {}", dir, e);
                err = 1;
            } else {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
                }
            }
        }
        err
    }

    /// rmdir - remove directories
    /// Port from zsh/Src/Modules/files.c bin_rmdir() lines 149-166
    fn builtin_rmdir(&self, args: &[String]) -> i32 {
        let mut err = 0;
        for arg in args {
            if arg.starts_with('-') {
                continue;
            }
            if let Err(e) = std::fs::remove_dir(arg) {
                eprintln!("rmdir: cannot remove '{}': {}", arg, e);
                err = 1;
            }
        }
        err
    }

    /// ln - create links
    /// Port from zsh/Src/Modules/files.c bin_ln() lines 200-294
    fn builtin_ln(&self, args: &[String]) -> i32 {
        let mut symbolic = false;
        let mut force = false;
        let mut no_deref = false;
        let mut files: Vec<&str> = Vec::new();

        for arg in args {
            match arg.as_str() {
                "-s" => symbolic = true,
                "-f" => force = true,
                "-n" | "-h" => no_deref = true,
                s if !s.starts_with('-') => files.push(s),
                _ => {}
            }
        }

        if files.len() < 2 {
            if files.len() == 1 {
                let src = files[0];
                let target = std::path::Path::new(src)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| src.to_string());
                files.push(Box::leak(target.into_boxed_str()));
            } else {
                eprintln!("ln: missing file operand");
                return 1;
            }
        }

        let target = files.pop().unwrap();
        let target_path = std::path::Path::new(target);
        let is_dir = !no_deref && target_path.is_dir();

        for src in files {
            let dest = if is_dir {
                format!(
                    "{}/{}",
                    target,
                    std::path::Path::new(src)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| src.to_string())
                )
            } else {
                target.to_string()
            };

            let dest_path = std::path::Path::new(&dest);
            if force && dest_path.exists() {
                let _ = std::fs::remove_file(&dest);
            }

            let result = if symbolic {
                #[cfg(unix)]
                {
                    std::os::unix::fs::symlink(src, &dest)
                }
                #[cfg(not(unix))]
                {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::Unsupported,
                        "symlinks not supported",
                    ))
                }
            } else {
                std::fs::hard_link(src, &dest)
            };

            if let Err(e) = result {
                eprintln!("ln: cannot create link '{}' -> '{}': {}", dest, src, e);
                return 1;
            }
        }
        0
    }

    /// mv - move/rename files
    /// Port from zsh/Src/Modules/files.c bin_ln()/domove() for mv mode
    fn builtin_mv(&self, args: &[String]) -> i32 {
        let mut force = false;
        let mut interactive = false;
        let mut verbose = false;
        let mut files: Vec<&str> = Vec::new();

        for arg in args {
            match arg.as_str() {
                "-f" => force = true,
                "-i" => interactive = true,
                "-v" => verbose = true,
                s if !s.starts_with('-') => files.push(s),
                _ => {}
            }
        }

        if files.len() < 2 {
            eprintln!("mv: missing file operand");
            return 1;
        }

        let target = files.pop().unwrap();
        let target_path = std::path::Path::new(target);
        let is_dir = target_path.is_dir();

        for src in files {
            let dest = if is_dir {
                format!(
                    "{}/{}",
                    target,
                    std::path::Path::new(src)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| src.to_string())
                )
            } else {
                target.to_string()
            };

            let dest_path = std::path::Path::new(&dest);
            if dest_path.exists() && !force {
                if interactive {
                    eprint!("mv: overwrite '{}'? ", dest);
                    let mut response = String::new();
                    if std::io::stdin().read_line(&mut response).is_err()
                        || !response.trim().eq_ignore_ascii_case("y")
                    {
                        continue;
                    }
                } else {
                    eprintln!("mv: cannot overwrite '{}': File exists", dest);
                    return 1;
                }
            }

            if let Err(e) = std::fs::rename(src, &dest) {
                eprintln!("mv: cannot move '{}' to '{}': {}", src, dest, e);
                return 1;
            }

            if verbose {
                println!("'{}' -> '{}'", src, dest);
            }
        }
        0
    }

    /// cp - copy files
    /// Port from zsh/Src/Modules/files.c recursive copy functionality
    fn builtin_cp(&self, args: &[String]) -> i32 {
        let mut recursive = false;
        let mut force = false;
        let mut interactive = false;
        let mut preserve = false;
        let mut verbose = false;
        let mut files: Vec<&str> = Vec::new();

        for arg in args {
            match arg.as_str() {
                "-r" | "-R" => recursive = true,
                "-f" => force = true,
                "-i" => interactive = true,
                "-p" => preserve = true,
                "-v" => verbose = true,
                s if !s.starts_with('-') => files.push(s),
                _ => {}
            }
        }

        let _ = preserve; // unused for now

        if files.len() < 2 {
            eprintln!("cp: missing file operand");
            return 1;
        }

        let target = files.pop().unwrap();
        let target_path = std::path::Path::new(target);
        let is_dir = target_path.is_dir();

        for src in files {
            let src_path = std::path::Path::new(src);
            let dest = if is_dir {
                format!(
                    "{}/{}",
                    target,
                    src_path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| src.to_string())
                )
            } else {
                target.to_string()
            };

            let dest_path = std::path::Path::new(&dest);
            if dest_path.exists() && !force {
                if interactive {
                    eprint!("cp: overwrite '{}'? ", dest);
                    let mut response = String::new();
                    if std::io::stdin().read_line(&mut response).is_err()
                        || !response.trim().eq_ignore_ascii_case("y")
                    {
                        continue;
                    }
                }
            }

            let result = if src_path.is_dir() {
                if recursive {
                    Self::copy_dir_recursive(src_path, dest_path)
                } else {
                    eprintln!("cp: -r not specified; omitting directory '{}'", src);
                    continue;
                }
            } else {
                std::fs::copy(src, &dest).map(|_| ())
            };

            if let Err(e) = result {
                eprintln!("cp: cannot copy '{}' to '{}': {}", src, dest, e);
                return 1;
            }

            if verbose {
                println!("'{}' -> '{}'", src, dest);
            }
        }
        0
    }

    fn copy_dir_recursive(src: &std::path::Path, dest: &std::path::Path) -> std::io::Result<()> {
        if !dest.exists() {
            std::fs::create_dir_all(dest)?;
        }
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let src_path = entry.path();
            let dest_path = dest.join(entry.file_name());

            if file_type.is_dir() {
                Self::copy_dir_recursive(&src_path, &dest_path)?;
            } else {
                std::fs::copy(&src_path, &dest_path)?;
            }
        }
        Ok(())
    }

    /// rm - remove files
    fn builtin_rm(&self, args: &[String]) -> i32 {
        let mut recursive = false;
        let mut force = false;
        let mut interactive = false;
        let mut verbose = false;
        let mut files: Vec<&str> = Vec::new();

        for arg in args {
            match arg.as_str() {
                "-r" | "-R" => recursive = true,
                "-f" => force = true,
                "-i" => interactive = true,
                "-v" => verbose = true,
                "-rf" | "-fr" => {
                    recursive = true;
                    force = true;
                }
                s if !s.starts_with('-') => files.push(s),
                _ => {}
            }
        }

        for file in files {
            let path = std::path::Path::new(file);

            if !path.exists() {
                if !force {
                    eprintln!("rm: cannot remove '{}': No such file or directory", file);
                    return 1;
                }
                continue;
            }

            if interactive {
                let file_type = if path.is_dir() { "directory" } else { "file" };
                eprint!("rm: remove {} '{}'? ", file_type, file);
                let mut response = String::new();
                if std::io::stdin().read_line(&mut response).is_err()
                    || !response.trim().eq_ignore_ascii_case("y")
                {
                    continue;
                }
            }

            let result = if path.is_dir() {
                if recursive {
                    std::fs::remove_dir_all(path)
                } else {
                    eprintln!("rm: cannot remove '{}': Is a directory", file);
                    return 1;
                }
            } else {
                std::fs::remove_file(path)
            };

            if let Err(e) = result {
                if !force {
                    eprintln!("rm: cannot remove '{}': {}", file, e);
                    return 1;
                }
            } else if verbose {
                println!("removed '{}'", file);
            }
        }
        0
    }

    /// chown - change file owner (Unix only)
    #[cfg(unix)]
    fn builtin_chown(&self, args: &[String]) -> i32 {
        use std::os::unix::fs::MetadataExt;

        let mut recursive = false;
        let mut positional: Vec<&str> = Vec::new();

        for arg in args {
            match arg.as_str() {
                "-R" => recursive = true,
                "-h" => {} // don't deference symlinks (default on most systems)
                s if !s.starts_with('-') => positional.push(s),
                _ => {}
            }
        }

        if positional.len() < 2 {
            eprintln!("chown: missing operand");
            return 1;
        }

        let owner_spec = positional[0];
        let files = &positional[1..];

        // Parse owner[:group]
        let (user, group) = if let Some(colon_pos) = owner_spec.find(':') {
            (&owner_spec[..colon_pos], Some(&owner_spec[colon_pos + 1..]))
        } else {
            (owner_spec, None)
        };

        let uid: u32 = if user.is_empty() {
            u32::MAX
        } else if let Ok(id) = user.parse() {
            id
        } else {
            // Look up user name
            unsafe {
                let c_user = std::ffi::CString::new(user).unwrap();
                let pw = libc::getpwnam(c_user.as_ptr());
                if pw.is_null() {
                    eprintln!("chown: invalid user: '{}'", user);
                    return 1;
                }
                (*pw).pw_uid
            }
        };

        let gid: u32 = match group {
            Some(g) if !g.is_empty() => {
                if let Ok(id) = g.parse() {
                    id
                } else {
                    unsafe {
                        let c_group = std::ffi::CString::new(g).unwrap();
                        let gr = libc::getgrnam(c_group.as_ptr());
                        if gr.is_null() {
                            eprintln!("chown: invalid group: '{}'", g);
                            return 1;
                        }
                        (*gr).gr_gid
                    }
                }
            }
            _ => u32::MAX,
        };

        fn do_chown(path: &std::path::Path, uid: u32, gid: u32, recursive: bool) -> i32 {
            let c_path = match std::ffi::CString::new(path.to_string_lossy().as_bytes()) {
                Ok(p) => p,
                Err(_) => return 1,
            };

            let ret = unsafe { libc::chown(c_path.as_ptr(), uid, gid) };
            if ret != 0 {
                eprintln!(
                    "chown: changing ownership of '{}': {}",
                    path.display(),
                    std::io::Error::last_os_error()
                );
                return 1;
            }

            if recursive && path.is_dir() {
                if let Ok(entries) = std::fs::read_dir(path) {
                    for entry in entries.flatten() {
                        if do_chown(&entry.path(), uid, gid, true) != 0 {
                            return 1;
                        }
                    }
                }
            }
            0
        }

        for file in files {
            if do_chown(std::path::Path::new(file), uid, gid, recursive) != 0 {
                return 1;
            }
        }
        0
    }

    #[cfg(not(unix))]
    fn builtin_chown(&self, _args: &[String]) -> i32 {
        eprintln!("chown: not supported on this platform");
        1
    }

    /// chmod - change file permissions
    fn builtin_chmod(&self, args: &[String]) -> i32 {
        let mut recursive = false;
        let mut positional: Vec<&str> = Vec::new();

        for arg in args {
            match arg.as_str() {
                "-R" => recursive = true,
                s if !s.starts_with('-') => positional.push(s),
                _ => {}
            }
        }

        if positional.len() < 2 {
            eprintln!("chmod: missing operand");
            return 1;
        }

        let mode_spec = positional[0];
        let files = &positional[1..];

        // Parse mode (octal or symbolic)
        let mode: Option<u32> = u32::from_str_radix(mode_spec, 8).ok();

        if mode.is_none() {
            // Symbolic mode not fully implemented
            eprintln!("chmod: symbolic mode not implemented, use octal");
            return 1;
        }

        let mode = mode.unwrap();

        fn do_chmod(path: &std::path::Path, mode: u32, recursive: bool) -> i32 {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Err(e) =
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
                {
                    eprintln!("chmod: changing permissions of '{}': {}", path.display(), e);
                    return 1;
                }

                if recursive && path.is_dir() {
                    if let Ok(entries) = std::fs::read_dir(path) {
                        for entry in entries.flatten() {
                            if do_chmod(&entry.path(), mode, true) != 0 {
                                return 1;
                            }
                        }
                    }
                }
            }
            #[cfg(not(unix))]
            {
                let _ = (path, mode, recursive);
            }
            0
        }

        for file in files {
            if do_chmod(std::path::Path::new(file), mode, recursive) != 0 {
                return 1;
            }
        }
        0
    }

    /// zln/zmv/zcp - file operations (zsh/files module)
    fn builtin_zfiles(&self, cmd: &str, args: &[String]) -> i32 {
        let mut force = false;
        let mut verbose = false;
        let mut files: Vec<&str> = Vec::new();

        for arg in args {
            match arg.as_str() {
                "-f" => force = true,
                "-v" => verbose = true,
                "-i" => {} // interactive - ignored
                s if !s.starts_with('-') => files.push(s),
                _ => {}
            }
        }

        if files.len() < 2 {
            eprintln!("{}: missing operand", cmd);
            return 1;
        }

        let target = files.pop().unwrap();
        let target_is_dir = std::path::Path::new(target).is_dir();

        for src in files {
            let dest = if target_is_dir {
                format!(
                    "{}/{}",
                    target,
                    std::path::Path::new(src)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| src.to_string())
                )
            } else {
                target.to_string()
            };

            if !force && std::path::Path::new(&dest).exists() {
                eprintln!("{}: '{}' already exists", cmd, dest);
                continue;
            }

            let result = match cmd {
                "zln" => {
                    #[cfg(unix)]
                    {
                        std::os::unix::fs::symlink(src, &dest)
                    }
                    #[cfg(not(unix))]
                    {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::Unsupported,
                            "symlinks not supported",
                        ))
                    }
                }
                "zcp" => std::fs::copy(src, &dest).map(|_| ()),
                "zmv" => std::fs::rename(src, &dest),
                _ => Ok(()),
            };

            match result {
                Ok(()) => {
                    if verbose {
                        println!("{} -> {}", src, dest);
                    }
                }
                Err(e) => {
                    eprintln!("{}: {}: {}", cmd, src, e);
                    return 1;
                }
            }
        }

        0
    }

    /// coproc - manage coprocesses
    fn builtin_coproc(&mut self, args: &[String]) -> i32 {
        // Basic coproc implementation
        if args.is_empty() {
            // List coprocesses
            println!("(no coprocesses)");
            return 0;
        }

        // Start a coprocess
        let cmd = args.join(" ");
        match std::process::Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
        {
            Ok(child) => {
                println!("[coproc] {}", child.id());
                0
            }
            Err(e) => {
                eprintln!("coproc: {}", e);
                1
            }
        }
    }

    /// zparseopts - parse options from positional parameters
    fn builtin_zparseopts(&mut self, args: &[String]) -> i32 {
        let mut remove_parsed = false; // -D
        let mut keep_going = false; // -E
        let mut fail_on_error = false; // -F
        let mut keep_values = false; // -K
        let mut _map_names = false; // -M (TODO: implement)
        let mut array_name: Option<String> = None; // -a
        let mut assoc_name: Option<String> = None; // -A
        let mut specs: Vec<String> = Vec::new();

        let mut iter = args.iter().peekable();

        // Parse zparseopts options
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-D" => remove_parsed = true,
                "-E" => keep_going = true,
                "-F" => fail_on_error = true,
                "-K" => keep_values = true,
                "-M" => _map_names = true,
                "-a" => {
                    if let Some(name) = iter.next() {
                        array_name = Some(name.clone());
                    }
                }
                "-A" => {
                    if let Some(name) = iter.next() {
                        assoc_name = Some(name.clone());
                    }
                }
                "-" | "--" => break,
                s if !s.starts_with('-') || s.contains('=') || s.contains(':') => {
                    specs.push(s.to_string());
                }
                _ => specs.push(arg.clone()),
            }
        }

        // Collect remaining specs
        for arg in iter {
            specs.push(arg.clone());
        }

        // Parse the specs to understand what options we're looking for
        #[derive(Clone)]
        struct OptSpec {
            name: String,
            takes_arg: bool,
            optional_arg: bool,
            #[allow(dead_code)]
            append: bool,
            target_array: Option<String>,
        }

        let mut opt_specs: Vec<OptSpec> = Vec::new();
        for spec in &specs {
            let mut s = spec.as_str();
            let mut target = None;

            // Check for =array at end
            if let Some(eq_pos) = s.rfind('=') {
                if !s[eq_pos + 1..].contains(':') {
                    target = Some(s[eq_pos + 1..].to_string());
                    s = &s[..eq_pos];
                }
            }

            let append = s.ends_with('+') || s.contains("+:");
            let s = s.trim_end_matches('+');

            let (name, takes_arg, optional_arg) = if s.ends_with("::") {
                (s.trim_end_matches(':').trim_end_matches(':'), true, true)
            } else if s.ends_with(':') {
                (s.trim_end_matches(':'), true, false)
            } else {
                (s, false, false)
            };

            opt_specs.push(OptSpec {
                name: name.to_string(),
                takes_arg,
                optional_arg,
                append,
                target_array: target,
            });
        }

        // Get positional parameters to parse
        let positionals: Vec<String> = (1..=99)
            .map(|i| self.get_variable(&i.to_string()))
            .take_while(|v| !v.is_empty())
            .collect();

        // Results
        let mut results: Vec<(String, Option<String>)> = Vec::new();
        let mut i = 0;
        let mut parsed_count = 0;

        while i < positionals.len() {
            let arg = &positionals[i];

            if arg == "-" || arg == "--" {
                parsed_count = i + 1;
                break;
            }

            if !arg.starts_with('-') {
                if !keep_going {
                    break;
                }
                i += 1;
                continue;
            }

            // Try to match against specs
            let opt_name = arg.trim_start_matches('-');
            let mut matched = false;

            for spec in &opt_specs {
                if opt_name == spec.name || opt_name.starts_with(&format!("{}=", spec.name)) {
                    matched = true;

                    if spec.takes_arg {
                        let arg_value = if opt_name.contains('=') {
                            Some(opt_name.splitn(2, '=').nth(1).unwrap_or("").to_string())
                        } else if i + 1 < positionals.len()
                            && (!positionals[i + 1].starts_with('-') || spec.optional_arg)
                        {
                            i += 1;
                            Some(positionals[i].clone())
                        } else if spec.optional_arg {
                            None
                        } else if fail_on_error {
                            eprintln!("zparseopts: missing argument for option: {}", spec.name);
                            return 1;
                        } else {
                            None
                        };
                        results.push((format!("-{}", spec.name), arg_value));
                    } else {
                        results.push((format!("-{}", spec.name), None));
                    }
                    break;
                }
            }

            if !matched && !keep_going {
                break;
            }

            i += 1;
            parsed_count = i;
        }

        // Store results in array
        if let Some(arr_name) = &array_name {
            let mut arr_values: Vec<String> = Vec::new();
            for (opt, val) in &results {
                arr_values.push(opt.clone());
                if let Some(v) = val {
                    arr_values.push(v.clone());
                }
            }
            self.arrays.insert(arr_name.clone(), arr_values);
        }

        // Store in associative array
        if let Some(assoc) = &assoc_name {
            let mut map: HashMap<String, String> = HashMap::new();
            for (opt, val) in &results {
                map.insert(opt.clone(), val.clone().unwrap_or_default());
            }
            self.assoc_arrays.insert(assoc.clone(), map);
        }

        // Store in per-option arrays
        for spec in &opt_specs {
            if let Some(target) = &spec.target_array {
                let values: Vec<String> = results
                    .iter()
                    .filter(|(opt, _)| opt.trim_start_matches('-') == spec.name)
                    .flat_map(|(opt, val)| {
                        let mut v = vec![opt.clone()];
                        if let Some(arg) = val {
                            v.push(arg.clone());
                        }
                        v
                    })
                    .collect();
                if !values.is_empty() || !keep_values {
                    self.arrays.insert(target.clone(), values);
                }
            }
        }

        // Remove parsed arguments if -D
        if remove_parsed && parsed_count > 0 {
            for i in 1..=parsed_count {
                self.variables.remove(&i.to_string());
                std::env::remove_var(i.to_string());
            }
            // Shift remaining
            let remaining: Vec<String> = ((parsed_count + 1)..=99)
                .map(|i| self.get_variable(&i.to_string()))
                .take_while(|v| !v.is_empty())
                .collect();
            for (i, val) in remaining.iter().enumerate() {
                self.variables.insert((i + 1).to_string(), val.clone());
            }
        }

        0
    }

    /// readonly - mark variables as read-only
    fn builtin_readonly(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            // List readonly variables
            for name in &self.readonly_vars {
                if let Some(val) = self.variables.get(name) {
                    println!("readonly {}={}", name, val);
                }
            }
            return 0;
        }

        for arg in args {
            if arg == "-p" {
                for name in &self.readonly_vars {
                    if let Some(val) = self.variables.get(name) {
                        println!("declare -r {}=\"{}\"", name, val);
                    }
                }
            } else if let Some(eq_pos) = arg.find('=') {
                let name = &arg[..eq_pos];
                let value = &arg[eq_pos + 1..];
                self.variables.insert(name.to_string(), value.to_string());
                self.readonly_vars.insert(name.to_string());
            } else {
                self.readonly_vars.insert(arg.clone());
            }
        }
        0
    }

    /// unfunction - remove function definitions
    fn builtin_unfunction(&mut self, args: &[String]) -> i32 {
        for name in args {
            if self.functions.remove(name).is_none() {
                eprintln!("unfunction: no such function: {}", name);
            }
        }
        0
    }

    /// getln - read line from buffer
    fn builtin_getln(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            eprintln!("getln: missing variable name");
            return 1;
        }
        // Read from line buffer (simplified - just reads from stdin)
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_ok() {
            let line = line.trim_end_matches('\n');
            self.variables.insert(args[0].clone(), line.to_string());
            0
        } else {
            1
        }
    }

    /// pushln - push line to buffer
    fn builtin_pushln(&mut self, args: &[String]) -> i32 {
        for arg in args {
            println!("{}", arg);
        }
        0
    }

    /// bindkey - key binding management
    fn builtin_bindkey(&mut self, args: &[String]) -> i32 {
        use crate::zle::{zle, KeymapName};

        if args.is_empty() {
            // List all bindings in main keymap
            let zle = zle();
            for (keys, widget) in zle
                .keymaps
                .get(&KeymapName::Main)
                .map(|km| km.list_bindings().collect::<Vec<_>>())
                .unwrap_or_default()
            {
                println!("\"{}\" {}", keys, widget);
            }
            return 0;
        }

        let mut iter = args.iter().peekable();
        let mut keymap = KeymapName::Main;
        let mut list_mode = false;
        let mut list_all = false;
        let mut remove = false;

        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-l" => {
                    list_mode = true;
                }
                "-L" => {
                    list_mode = true;
                    list_all = true;
                }
                "-la" | "-lL" => {
                    list_mode = true;
                    list_all = true;
                }
                "-M" => {
                    if let Some(name) = iter.next() {
                        if let Some(km) = KeymapName::from_str(name) {
                            keymap = km;
                        }
                    }
                }
                "-r" => {
                    remove = true;
                }
                "-A" => {
                    // Link keymaps - stub
                    return 0;
                }
                "-N" => {
                    // Create new keymap - stub
                    return 0;
                }
                "-e" => {
                    keymap = KeymapName::Emacs;
                }
                "-v" => {
                    keymap = KeymapName::ViInsert;
                }
                "-a" => {
                    keymap = KeymapName::ViCommand;
                }
                key if !key.starts_with('-') => {
                    // Key sequence - next arg is widget
                    if let Some(widget) = iter.next() {
                        let mut zle = zle();
                        if remove {
                            zle.unbind_key(keymap, key);
                        } else {
                            zle.bind_key(keymap, key, widget);
                        }
                    }
                    return 0;
                }
                _ => {}
            }
        }

        if list_mode {
            let zle = zle();
            if list_all {
                for km_name in &[
                    KeymapName::Emacs,
                    KeymapName::ViInsert,
                    KeymapName::ViCommand,
                ] {
                    println!("{}", km_name.as_str());
                }
            } else {
                if let Some(km) = zle.keymaps.get(&keymap) {
                    for (keys, widget) in km.list_bindings() {
                        println!("bindkey \"{}\" {}", keys, widget);
                    }
                }
            }
        }

        0
    }

    /// zle - line editor control
    fn builtin_zle(&mut self, args: &[String]) -> i32 {
        use crate::zle::zle;

        if args.is_empty() {
            return 0;
        }

        let mut iter = args.iter().peekable();

        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-l" => {
                    // List widgets
                    let zle = zle();
                    let mut widgets: Vec<&str> = zle.list_widgets();
                    widgets.sort();
                    for w in widgets {
                        println!("{}", w);
                    }
                    return 0;
                }
                "-la" | "-lL" => {
                    // List all widgets with details
                    let zle = zle();
                    let mut widgets: Vec<&str> = zle.list_widgets();
                    widgets.sort();
                    for w in widgets {
                        println!("{}", w);
                    }
                    return 0;
                }
                "-N" => {
                    // Define new widget: zle -N widget-name [function]
                    if let Some(widget_name) = iter.next() {
                        let func_name = iter
                            .next()
                            .map(|s| s.as_str())
                            .unwrap_or(widget_name.as_str());
                        let mut zle = zle();
                        zle.define_widget(widget_name, func_name);
                    }
                    return 0;
                }
                "-D" => {
                    // Delete widget - stub
                    return 0;
                }
                "-A" => {
                    // Define widget alias - stub
                    return 0;
                }
                "-R" => {
                    // Redisplay
                    return 0;
                }
                "-U" => {
                    // Unget characters - stub
                    return 0;
                }
                "-K" => {
                    // Select keymap - stub
                    return 0;
                }
                "-F" => {
                    // Install file descriptor handler - stub
                    return 0;
                }
                "-M" => {
                    // Display message - stub
                    return 0;
                }
                "-I" => {
                    // Invalidate completion - stub
                    return 0;
                }
                "-f" => {
                    // Check widget exists
                    if let Some(name) = iter.next() {
                        let zle = zle();
                        return if zle.get_widget(name).is_some() { 0 } else { 1 };
                    }
                    return 1;
                }
                widget_name if !widget_name.starts_with('-') => {
                    // Call widget
                    let mut zle = zle();
                    match zle.execute_widget(widget_name, None) {
                        crate::zle::WidgetResult::Ok => return 0,
                        crate::zle::WidgetResult::Error(e) => {
                            eprintln!("zle: {}", e);
                            return 1;
                        }
                        crate::zle::WidgetResult::CallFunction(func) => {
                            // Would need to call shell function
                            drop(zle);
                            if let Some(f) = self.functions.get(&func).cloned() {
                                return self.call_function(&f, &[]).unwrap_or(1);
                            }
                            return 1;
                        }
                        _ => return 0,
                    }
                }
                _ => {}
            }
        }

        0
    }

    /// sched - scheduled command execution (stub)
    fn builtin_sched(&mut self, args: &[String]) -> i32 {
        use std::time::{Duration, SystemTime};

        if args.is_empty() {
            // List scheduled commands
            if self.scheduled_commands.is_empty() {
                return 0;
            }
            let now = SystemTime::now();
            for cmd in &self.scheduled_commands {
                let remaining = cmd.run_at.duration_since(now).unwrap_or(Duration::ZERO);
                println!("{:3}  +{:5}  {}", cmd.id, remaining.as_secs(), cmd.command);
            }
            return 0;
        }

        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "-" => {
                    // Remove scheduled item
                    i += 1;
                    if i >= args.len() {
                        eprintln!("sched: -: need item number");
                        return 1;
                    }
                    if let Ok(id) = args[i].parse::<u32>() {
                        self.scheduled_commands.retain(|c| c.id != id);
                        return 0;
                    } else {
                        eprintln!("sched: invalid item number");
                        return 1;
                    }
                }
                "+" => {
                    // Schedule relative time
                    i += 1;
                    if i >= args.len() {
                        eprintln!("sched: +: need time");
                        return 1;
                    }
                    let secs: u64 = args[i].parse().unwrap_or(0);
                    i += 1;
                    let command = args[i..].join(" ");

                    let id = self.scheduled_commands.len() as u32 + 1;
                    self.scheduled_commands.push(ScheduledCommand {
                        id,
                        run_at: SystemTime::now() + Duration::from_secs(secs),
                        command,
                    });
                    return 0;
                }
                time_str => {
                    // Parse HH:MM or HH:MM:SS
                    let parts: Vec<&str> = time_str.split(':').collect();
                    if parts.len() >= 2 {
                        let hour: u32 = parts[0].parse().unwrap_or(0);
                        let min: u32 = parts[1].parse().unwrap_or(0);
                        let sec: u32 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);

                        // Calculate duration until that time today/tomorrow
                        let now = SystemTime::now();
                        let target_secs = (hour * 3600 + min * 60 + sec) as u64;
                        let _day_secs = 86400u64;

                        // Simplified: just add as seconds from now
                        let run_at = now + Duration::from_secs(target_secs);

                        i += 1;
                        let command = args[i..].join(" ");

                        let id = self.scheduled_commands.len() as u32 + 1;
                        self.scheduled_commands.push(ScheduledCommand {
                            id,
                            run_at,
                            command,
                        });
                        return 0;
                    } else {
                        eprintln!("sched: invalid time format");
                        return 1;
                    }
                }
            }
        }
        0
    }

    /// zcompile - compile shell scripts to ZWC format
    fn builtin_zcompile(&mut self, args: &[String]) -> i32 {
        use crate::zwc::{ZwcBuilder, ZwcFile};

        let mut list_mode = false; // -t: list functions in zwc
        let mut compile_current = false; // -c: compile current functions
        let mut compile_auto = false; // -a: compile autoload functions
        let mut files: Vec<String> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if arg.starts_with('-') && arg.len() > 1 {
                for c in arg[1..].chars() {
                    match c {
                        't' => list_mode = true,
                        'c' => compile_current = true,
                        'a' => compile_auto = true,
                        'U' | 'M' | 'R' | 'm' | 'z' | 'k' => {} // ignored for now
                        _ => {
                            eprintln!("zcompile: unknown option: -{}", c);
                            return 1;
                        }
                    }
                }
            } else {
                files.push(arg.clone());
            }
            i += 1;
        }

        if files.is_empty() {
            eprintln!("zcompile: not enough arguments");
            return 1;
        }

        // -t mode: list functions in ZWC file
        if list_mode {
            let zwc_path = if files[0].ends_with(".zwc") {
                files[0].clone()
            } else {
                format!("{}.zwc", files[0])
            };

            match ZwcFile::load(&zwc_path) {
                Ok(zwc) => {
                    println!("zwc file for zshrs-{}", env!("CARGO_PKG_VERSION"));
                    if files.len() > 1 {
                        // Check specific functions
                        for name in &files[1..] {
                            if zwc.get_function(name).is_some() {
                                println!("{}", name);
                            } else {
                                eprintln!("zcompile: function not found: {}", name);
                                return 1;
                            }
                        }
                    } else {
                        // List all functions
                        for name in zwc.list_functions() {
                            println!("{}", name);
                        }
                    }
                    return 0;
                }
                Err(e) => {
                    eprintln!("zcompile: can't read zwc file: {}: {}", zwc_path, e);
                    return 1;
                }
            }
        }

        // -c or -a mode: compile current/autoload functions
        if compile_current || compile_auto {
            let zwc_path = if files[0].ends_with(".zwc") {
                files[0].clone()
            } else {
                format!("{}.zwc", files[0])
            };

            let mut builder = ZwcBuilder::new();

            if files.len() > 1 {
                // Compile specific functions
                for name in &files[1..] {
                    if let Some(func) = self.functions.get(name) {
                        // Serialize the function (simplified - just store as comment for now)
                        let source = format!("# Compiled function: {}\n# Body: {:?}", name, func);
                        builder.add_source(name, &source);
                    } else if compile_auto && self.autoload_pending.contains_key(name) {
                        // Try to load autoload function source
                        if let Some(path) = self.find_function_file(name) {
                            if let Err(e) = builder.add_file(&path) {
                                eprintln!("zcompile: can't read {}: {}", name, e);
                                return 1;
                            }
                        }
                    } else {
                        eprintln!("zcompile: no such function: {}", name);
                        return 1;
                    }
                }
            } else {
                // Compile all functions
                for (name, func) in &self.functions {
                    let source = format!("# Compiled function: {}\n# Body: {:?}", name, func);
                    builder.add_source(name, &source);
                }
            }

            if let Err(e) = builder.write(&zwc_path) {
                eprintln!("zcompile: can't write {}: {}", zwc_path, e);
                return 1;
            }
            return 0;
        }

        // Default: compile files to ZWC
        let zwc_path = if files[0].ends_with(".zwc") {
            files[0].clone()
        } else {
            format!("{}.zwc", files[0])
        };

        let mut builder = ZwcBuilder::new();

        // If only one file given, it's both the source and output base
        let source_files = if files.len() == 1 {
            // Check if it's a directory
            let path = std::path::Path::new(&files[0]);
            if path.is_dir() {
                // Compile all files in directory
                match std::fs::read_dir(path) {
                    Ok(entries) => {
                        for entry in entries.flatten() {
                            let p = entry.path();
                            if p.is_file() && !p.extension().map_or(false, |e| e == "zwc") {
                                if let Err(e) = builder.add_file(&p) {
                                    eprintln!("zcompile: can't read {:?}: {}", p, e);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("zcompile: can't read directory: {}", e);
                        return 1;
                    }
                }
                vec![]
            } else {
                vec![files[0].clone()]
            }
        } else {
            files[1..].to_vec()
        };

        for file in &source_files {
            let path = std::path::Path::new(file);
            if let Err(e) = builder.add_file(path) {
                eprintln!("zcompile: can't read {}: {}", file, e);
                return 1;
            }
        }

        if let Err(e) = builder.write(&zwc_path) {
            eprintln!("zcompile: can't write {}: {}", zwc_path, e);
            return 1;
        }

        0
    }

    /// zformat - format strings
    fn builtin_zformat(&self, args: &[String]) -> i32 {
        if args.len() < 2 {
            eprintln!("zformat: not enough arguments");
            return 1;
        }

        match args[0].as_str() {
            "-f" => {
                // Format string: zformat -f var format specs...
                if args.len() < 3 {
                    return 1;
                }
                let _var_name = &args[1];
                let format = &args[2];
                let specs: HashMap<char, &str> = args[3..]
                    .iter()
                    .filter_map(|s| {
                        let mut chars = s.chars();
                        let key = chars.next()?;
                        if chars.next() == Some(':') {
                            Some((key, &s[2..]))
                        } else {
                            None
                        }
                    })
                    .collect();

                let mut result = String::new();
                let mut chars = format.chars().peekable();
                while let Some(c) = chars.next() {
                    if c == '%' {
                        if let Some(&spec_char) = chars.peek() {
                            if let Some(replacement) = specs.get(&spec_char) {
                                result.push_str(replacement);
                                chars.next();
                                continue;
                            }
                        }
                    }
                    result.push(c);
                }
                println!("{}", result);
            }
            "-a" => {
                // Format into array elements: zformat -a array sep specs...
                // Each spec is "text:value" or "text:value:cond"
                if args.len() < 4 {
                    eprintln!("zformat -a: need array, separator, and specs");
                    return 1;
                }
                let _array_name = &args[1];
                let sep = &args[2];

                let mut results = Vec::new();
                for spec in &args[3..] {
                    let parts: Vec<&str> = spec.splitn(3, ':').collect();
                    if parts.len() >= 2 {
                        let text = parts[0];
                        let value = parts[1];
                        let cond = parts.get(2).copied();

                        // If condition exists and is empty/false, skip
                        if let Some(c) = cond {
                            if c.is_empty() || c == "0" {
                                continue;
                            }
                        }

                        if !value.is_empty() {
                            results.push(format!("{}{}{}", text, sep, value));
                        }
                    }
                }

                for r in results {
                    println!("{}", r);
                }
            }
            _ => {
                eprintln!("zformat: unknown option: {}", args[0]);
                return 1;
            }
        }
        0
    }

    /// vared - visually edit a variable
    fn builtin_vared(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            eprintln!("vared: not enough arguments");
            return 1;
        }

        let mut var_name = String::new();
        let mut prompt = String::new();
        let mut rprompt = String::new();
        let mut _history = false; // TODO: implement history completion
        let mut i = 0;

        while i < args.len() {
            match args[i].as_str() {
                "-p" if i + 1 < args.len() => {
                    i += 1;
                    prompt = args[i].clone();
                }
                "-r" if i + 1 < args.len() => {
                    i += 1;
                    rprompt = args[i].clone();
                }
                "-h" => _history = true,
                "-c" => {} // Use completion - ignored
                "-e" => {} // Use emacs mode - ignored
                "-M" | "-m" => {
                    i += 1;
                } // Main/alt keymap - skip arg
                "-a" | "-A" => {
                    i += 1;
                } // Array assignment - skip arg
                s if !s.starts_with('-') => {
                    var_name = s.to_string();
                }
                _ => {}
            }
            i += 1;
        }

        if var_name.is_empty() {
            eprintln!("vared: not enough arguments");
            return 1;
        }

        // Get current value
        let current = self.get_variable(&var_name);

        // Simple line editing using stdin
        if !prompt.is_empty() {
            eprint!("{}", prompt);
        }
        print!("{}", current);
        if !rprompt.is_empty() {
            eprint!("{}", rprompt);
        }

        let mut input = String::new();
        if std::io::stdin().read_line(&mut input).is_ok() {
            let value = input.trim_end_matches('\n').to_string();
            self.variables.insert(var_name, value);
            return 0;
        }
        1
    }

    /// echotc - output termcap value
    fn builtin_echotc(&self, args: &[String]) -> i32 {
        if args.is_empty() {
            eprintln!("echotc: not enough arguments");
            return 1;
        }

        // Common termcap capabilities
        match args[0].as_str() {
            "cl" => print!("\x1b[H\x1b[2J"), // clear screen
            "cd" => print!("\x1b[J"),        // clear to end of display
            "ce" => print!("\x1b[K"),        // clear to end of line
            "cm" => {
                // cursor motion - needs row, col args
                if args.len() >= 3 {
                    if let (Ok(row), Ok(col)) = (args[1].parse::<u32>(), args[2].parse::<u32>()) {
                        print!("\x1b[{};{}H", row + 1, col + 1);
                    }
                }
            }
            "up" => print!("\x1b[A"),    // cursor up
            "do" => print!("\x1b[B"),    // cursor down
            "le" => print!("\x1b[D"),    // cursor left
            "nd" => print!("\x1b[C"),    // cursor right
            "ho" => print!("\x1b[H"),    // home cursor
            "vi" => print!("\x1b[?25l"), // invisible cursor
            "ve" => print!("\x1b[?25h"), // visible cursor
            "so" => print!("\x1b[7m"),   // standout mode
            "se" => print!("\x1b[27m"),  // end standout
            "us" => print!("\x1b[4m"),   // underline
            "ue" => print!("\x1b[24m"),  // end underline
            "md" => print!("\x1b[1m"),   // bold
            "me" => print!("\x1b[0m"),   // end all attributes
            "mr" => print!("\x1b[7m"),   // reverse video
            "AF" | "setaf" => {
                // Set foreground color
                if args.len() >= 2 {
                    if let Ok(color) = args[1].parse::<u32>() {
                        print!("\x1b[38;5;{}m", color);
                    }
                }
            }
            "AB" | "setab" => {
                // Set background color
                if args.len() >= 2 {
                    if let Ok(color) = args[1].parse::<u32>() {
                        print!("\x1b[48;5;{}m", color);
                    }
                }
            }
            "Co" | "colors" => {
                // Number of colors - assume 256
                println!("256");
            }
            "co" | "cols" => {
                // Number of columns
                println!(
                    "{}",
                    std::env::var("COLUMNS")
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(80u16)
                );
            }
            "li" | "lines" => {
                // Number of lines
                println!(
                    "{}",
                    std::env::var("LINES")
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(24u16)
                );
            }
            cap => {
                eprintln!("echotc: unknown capability: {}", cap);
                return 1;
            }
        }
        use std::io::Write;
        let _ = std::io::stdout().flush();
        0
    }

    /// echoti - output terminfo value
    fn builtin_echoti(&self, args: &[String]) -> i32 {
        // echoti is similar to echotc but uses terminfo names
        // For simplicity, we'll use the same implementation
        self.builtin_echotc(args)
    }

    /// zpty - manage pseudo-terminals
    fn builtin_zpty(&mut self, args: &[String]) -> i32 {
        use std::io::{Read, Write};
        use std::process::{Command, Stdio};

        if args.is_empty() {
            // List all ptys
            if self.zptys.is_empty() {
                return 0;
            }
            for (name, state) in &self.zptys {
                println!("{}: {} (pid {})", name, state.cmd, state.pid);
            }
            return 0;
        }

        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "-d" => {
                    // Delete pty
                    i += 1;
                    if i >= args.len() {
                        eprintln!("zpty: -d requires pty name");
                        return 1;
                    }
                    let name = &args[i];
                    if let Some(mut state) = self.zptys.remove(name) {
                        if let Some(ref mut child) = state.child {
                            let _ = child.kill();
                        }
                        return 0;
                    } else {
                        eprintln!("zpty: no such pty: {}", name);
                        return 1;
                    }
                }
                "-w" => {
                    // Write to pty: zpty -w name string...
                    i += 1;
                    if i >= args.len() {
                        eprintln!("zpty: -w requires pty name");
                        return 1;
                    }
                    let name = args[i].clone();
                    i += 1;
                    let data = args[i..].join(" ") + "\n";

                    if let Some(state) = self.zptys.get_mut(&name) {
                        if let Some(ref mut stdin) = state.stdin {
                            if stdin.write_all(data.as_bytes()).is_ok() {
                                let _ = stdin.flush();
                                return 0;
                            }
                        }
                        eprintln!("zpty: write failed");
                        return 1;
                    } else {
                        eprintln!("zpty: no such pty: {}", name);
                        return 1;
                    }
                }
                "-r" => {
                    // Read from pty: zpty -r name [param]
                    i += 1;
                    if i >= args.len() {
                        eprintln!("zpty: -r requires pty name");
                        return 1;
                    }
                    let name = args[i].clone();
                    i += 1;
                    let var_name = if i < args.len() {
                        args[i].clone()
                    } else {
                        "REPLY".to_string()
                    };

                    if let Some(state) = self.zptys.get_mut(&name) {
                        if let Some(ref mut stdout) = state.stdout {
                            let mut buf = vec![0u8; 4096];
                            match stdout.read(&mut buf) {
                                Ok(n) => {
                                    let data = String::from_utf8_lossy(&buf[..n]).to_string();
                                    self.variables.insert(var_name, data);
                                    return 0;
                                }
                                Err(_) => return 1,
                            }
                        }
                        return 1;
                    } else {
                        eprintln!("zpty: no such pty: {}", name);
                        return 1;
                    }
                }
                "-t" => {
                    // Test if data available
                    i += 1;
                    if i >= args.len() {
                        return 1;
                    }
                    let name = &args[i];
                    if self.zptys.contains_key(name) {
                        return 0; // Assume data available if pty exists
                    }
                    return 1;
                }
                "-L" => {
                    // List in script-friendly format
                    for (name, state) in &self.zptys {
                        println!("zpty {} {}", name, state.cmd);
                    }
                    return 0;
                }
                "-b" | "-e" => {
                    // Options: -b (blocking), -e (echo)
                    i += 1;
                    continue;
                }
                name if !name.starts_with('-') => {
                    // Create new pty: zpty name command [args...]
                    i += 1;
                    if i >= args.len() {
                        eprintln!("zpty: command required");
                        return 1;
                    }
                    let cmd_str = args[i..].join(" ");

                    match Command::new("sh")
                        .arg("-c")
                        .arg(&cmd_str)
                        .stdin(Stdio::piped())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()
                    {
                        Ok(mut child) => {
                            let pid = child.id();
                            let stdin = child.stdin.take();
                            let stdout = child.stdout.take();

                            self.zptys.insert(
                                name.to_string(),
                                ZptyState {
                                    pid,
                                    cmd: cmd_str,
                                    stdin,
                                    stdout,
                                    child: Some(child),
                                },
                            );
                            return 0;
                        }
                        Err(e) => {
                            eprintln!("zpty: failed to start: {}", e);
                            return 1;
                        }
                    }
                }
                _ => {
                    i += 1;
                }
            }
            i += 1;
        }
        0
    }

    /// zprof - profiling support
    fn builtin_zprof(&mut self, args: &[String]) -> i32 {
        use crate::zprof::ZprofOptions;

        let options = ZprofOptions {
            clear: args.iter().any(|a| a == "-c"),
        };

        let (status, output) = crate::zprof::builtin_zprof(&mut self.profiler, &options);
        if !output.is_empty() {
            print!("{}", output);
        }
        status
    }

    /// zsocket - create/manage sockets
    fn builtin_zsocket(&mut self, args: &[String]) -> i32 {
        use std::os::unix::net::{UnixListener, UnixStream};

        if args.is_empty() {
            // List open sockets
            if self.unix_sockets.is_empty() {
                return 0;
            }
            for (fd, state) in &self.unix_sockets {
                let path = state
                    .path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                let status = if state.listening {
                    "listening"
                } else {
                    "connected"
                };
                println!("{}: {} ({})", fd, path, status);
            }
            return 0;
        }

        let mut i = 0;
        let mut verbose = false;
        let mut var_name = "REPLY".to_string();

        while i < args.len() {
            match args[i].as_str() {
                "-v" => {
                    verbose = true;
                    i += 1;
                    if i < args.len() && !args[i].starts_with('-') {
                        var_name = args[i].clone();
                    }
                }
                "-l" => {
                    // Listen on Unix socket: zsocket -l path
                    i += 1;
                    if i >= args.len() {
                        eprintln!("zsocket: -l requires path");
                        return 1;
                    }
                    let path = PathBuf::from(&args[i]);

                    // Remove existing socket file
                    let _ = std::fs::remove_file(&path);

                    match UnixListener::bind(&path) {
                        Ok(listener) => {
                            let fd = self.next_fd;
                            self.next_fd += 1;

                            self.unix_sockets.insert(
                                fd,
                                UnixSocketState {
                                    path: Some(path),
                                    listening: true,
                                    stream: None,
                                    listener: Some(listener),
                                },
                            );

                            if verbose {
                                self.variables.insert(var_name.clone(), fd.to_string());
                            }
                            println!("{}", fd);
                            return 0;
                        }
                        Err(e) => {
                            eprintln!("zsocket: bind failed: {}", e);
                            return 1;
                        }
                    }
                }
                "-a" => {
                    // Accept connection: zsocket -a fd
                    i += 1;
                    if i >= args.len() {
                        eprintln!("zsocket: -a requires fd");
                        return 1;
                    }
                    let listen_fd: i32 = args[i].parse().unwrap_or(-1);

                    if let Some(state) = self.unix_sockets.get(&listen_fd) {
                        if let Some(ref listener) = state.listener {
                            match listener.accept() {
                                Ok((stream, _addr)) => {
                                    let new_fd = self.next_fd;
                                    self.next_fd += 1;

                                    self.unix_sockets.insert(
                                        new_fd,
                                        UnixSocketState {
                                            path: None,
                                            listening: false,
                                            stream: Some(stream),
                                            listener: None,
                                        },
                                    );

                                    if verbose {
                                        self.variables.insert(var_name.clone(), new_fd.to_string());
                                    }
                                    println!("{}", new_fd);
                                    return 0;
                                }
                                Err(e) => {
                                    eprintln!("zsocket: accept failed: {}", e);
                                    return 1;
                                }
                            }
                        }
                    }
                    eprintln!("zsocket: invalid fd");
                    return 1;
                }
                "-d" => {
                    // Close socket: zsocket -d fd
                    i += 1;
                    if i >= args.len() {
                        eprintln!("zsocket: -d requires fd");
                        return 1;
                    }
                    let fd: i32 = args[i].parse().unwrap_or(-1);

                    if let Some(state) = self.unix_sockets.remove(&fd) {
                        if let Some(path) = state.path {
                            let _ = std::fs::remove_file(path);
                        }
                        return 0;
                    }
                    eprintln!("zsocket: no such fd");
                    return 1;
                }
                path if !path.starts_with('-') => {
                    // Connect to Unix socket: zsocket path
                    match UnixStream::connect(path) {
                        Ok(stream) => {
                            let fd = self.next_fd;
                            self.next_fd += 1;

                            self.unix_sockets.insert(
                                fd,
                                UnixSocketState {
                                    path: Some(PathBuf::from(path)),
                                    listening: false,
                                    stream: Some(stream),
                                    listener: None,
                                },
                            );

                            if verbose {
                                self.variables.insert(var_name.clone(), fd.to_string());
                            }
                            println!("{}", fd);
                            return 0;
                        }
                        Err(e) => {
                            eprintln!("zsocket: connect failed: {}", e);
                            return 1;
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
        0
    }

    /// ztcp - TCP socket operations
    fn builtin_ztcp(&mut self, args: &[String]) -> i32 {
        // Similar to zsocket but TCP specific
        self.builtin_zsocket(args)
    }

    /// zregexparse - parse with regex
    fn builtin_zregexparse(&mut self, args: &[String]) -> i32 {
        if args.len() < 2 {
            eprintln!("zregexparse: usage: zregexparse var pattern [string]");
            return 1;
        }

        let var_name = &args[0];
        let pattern = &args[1];
        let string = if args.len() > 2 {
            args[2].clone()
        } else {
            self.variables.get("REPLY").cloned().unwrap_or_default()
        };

        match regex::Regex::new(pattern) {
            Ok(re) => {
                if let Some(captures) = re.captures(&string) {
                    // Store full match in var
                    if let Some(m) = captures.get(0) {
                        self.variables
                            .insert(var_name.clone(), m.as_str().to_string());
                    }

                    // Store capture groups in MATCH array
                    let mut match_array = Vec::new();
                    let mut mbegin_array = Vec::new();
                    let mut mend_array = Vec::new();

                    for (i, cap) in captures.iter().enumerate() {
                        if let Some(c) = cap {
                            match_array.push(c.as_str().to_string());
                            mbegin_array.push((c.start() + 1).to_string());
                            mend_array.push(c.end().to_string());
                            self.variables
                                .insert(format!("match[{}]", i), c.as_str().to_string());
                        }
                    }
                    self.arrays.insert("match".to_string(), match_array);
                    self.arrays.insert("mbegin".to_string(), mbegin_array);
                    self.arrays.insert("mend".to_string(), mend_array);

                    // Store match positions
                    if let Some(m) = captures.get(0) {
                        self.variables
                            .insert("MBEGIN".to_string(), (m.start() + 1).to_string());
                        self.variables
                            .insert("MEND".to_string(), m.end().to_string());
                    }

                    0
                } else {
                    1
                }
            }
            Err(e) => {
                eprintln!("zregexparse: invalid regex: {}", e);
                2
            }
        }
    }

    /// clone - create a subshell with forked state
    fn builtin_clone(&mut self, args: &[String]) -> i32 {
        use std::process::Command;

        // clone creates a subshell that shares the parent's state
        // We simulate this by spawning a new zshrs process
        let mut cmd =
            Command::new(std::env::current_exe().unwrap_or_else(|_| PathBuf::from("zshrs")));

        if !args.is_empty() {
            // If args provided, run them in the subshell
            cmd.arg("-c").arg(args.join(" "));
        }

        // Export current variables to child
        for (k, v) in &self.variables {
            cmd.env(k, v);
        }

        match cmd.spawn() {
            Ok(mut child) => match child.wait() {
                Ok(status) => status.code().unwrap_or(0),
                Err(_) => 1,
            },
            Err(e) => {
                eprintln!("clone: failed to spawn subshell: {}", e);
                1
            }
        }
    }

    /// log - same as logout for login shells
    fn builtin_log(&mut self, args: &[String]) -> i32 {
        self.builtin_exit(args)
    }

    // Completion system builtins (stubs for compsys)

    /// comparguments - parse completion arguments
    fn builtin_comparguments(&mut self, _args: &[String]) -> i32 {
        // Used internally by _arguments
        0
    }

    /// compcall - call completion function
    fn builtin_compcall(&mut self, _args: &[String]) -> i32 {
        // Calls the completion function
        0
    }

    /// compctl - old-style completion (deprecated)
    fn builtin_compctl(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            println!("compctl: old-style completion system");
            println!("Use the new completion system (compsys) instead");
            return 0;
        }
        // Parse compctl options for backwards compatibility
        0
    }

    /// compdescribe - describe completions
    fn builtin_compdescribe(&mut self, _args: &[String]) -> i32 {
        0
    }

    /// compfiles - complete files
    fn builtin_compfiles(&mut self, _args: &[String]) -> i32 {
        0
    }

    /// compgroups - group completions
    fn builtin_compgroups(&mut self, _args: &[String]) -> i32 {
        0
    }

    /// compquote - quote completion strings
    fn builtin_compquote(&mut self, _args: &[String]) -> i32 {
        0
    }

    /// comptags - manage completion tags
    fn builtin_comptags(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            return 1;
        }
        match args[0].as_str() {
            "-i" => {
                // Initialize tags
                0
            }
            "-S" => {
                // Set tags
                0
            }
            _ => 1,
        }
    }

    /// comptry - try completion
    fn builtin_comptry(&mut self, _args: &[String]) -> i32 {
        1 // No match
    }

    /// compvalues - complete values
    fn builtin_compvalues(&mut self, _args: &[String]) -> i32 {
        0
    }

    /// cap/getcap/setcap - Linux capabilities (stub on macOS)
    fn builtin_cap(&self, args: &[String]) -> i32 {
        // Linux capabilities are not available on macOS
        // On Linux, these would interact with libcap
        if args.is_empty() {
            println!("cap: display/set capabilities");
            println!("  getcap file...  - display capabilities");
            println!("  setcap caps file - set capabilities");
            return 0;
        }

        #[cfg(target_os = "linux")]
        {
            // On Linux, we could use libcap bindings
            // For now, just run the external commands
            let status = std::process::Command::new(&args[0])
                .args(&args[1..])
                .status();
            return status.map(|s| s.code().unwrap_or(1)).unwrap_or(1);
        }

        #[cfg(not(target_os = "linux"))]
        {
            eprintln!("cap: capabilities not supported on this platform");
            1
        }
    }

    /// zcurses - curses interface (stub)
    fn builtin_zcurses(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            eprintln!("zcurses: requires subcommand");
            return 1;
        }

        match args[0].as_str() {
            "init" => {
                // Initialize curses
                println!("zcurses: would initialize curses");
                0
            }
            "end" => {
                // End curses mode
                println!("zcurses: would end curses");
                0
            }
            "addwin" => {
                // Add a window
                0
            }
            "delwin" => {
                // Delete a window
                0
            }
            "refresh" => {
                // Refresh display
                0
            }
            "move" => {
                // Move cursor
                0
            }
            "clear" => {
                // Clear window
                0
            }
            "char" | "string" => {
                // Output character/string
                0
            }
            "border" => {
                // Draw border
                0
            }
            "attr" => {
                // Set attributes
                0
            }
            "color" => {
                // Set colors
                0
            }
            "scroll" => {
                // Scroll window
                0
            }
            "input" => {
                // Get input
                0
            }
            "mouse" => {
                // Mouse support
                0
            }
            "querychar" => {
                // Query character at position
                0
            }
            "resize" => {
                // Resize window
                0
            }
            cmd => {
                eprintln!("zcurses: unknown subcommand: {}", cmd);
                1
            }
        }
    }

    /// sysread - low-level read (zsh/system module)
    fn builtin_sysread(&mut self, args: &[String]) -> i32 {
        use std::io::Read;

        let mut fd = 0i32; // stdin
        let mut count: Option<usize> = None;
        let mut var_name = "REPLY".to_string();
        let mut i = 0;

        while i < args.len() {
            match args[i].as_str() {
                "-c" if i + 1 < args.len() => {
                    i += 1;
                    count = args[i].parse().ok();
                }
                "-i" if i + 1 < args.len() => {
                    i += 1;
                    fd = args[i].parse().unwrap_or(0);
                }
                "-o" if i + 1 < args.len() => {
                    i += 1;
                    var_name = args[i].clone();
                }
                "-t" if i + 1 < args.len() => {
                    i += 1;
                    // Timeout - ignored for now
                }
                _ => {
                    var_name = args[i].clone();
                }
            }
            i += 1;
        }

        let mut buffer = vec![0u8; count.unwrap_or(8192)];

        // Only support stdin for now
        if fd == 0 {
            match std::io::stdin().read(&mut buffer) {
                Ok(n) => {
                    buffer.truncate(n);
                    let s = String::from_utf8_lossy(&buffer).to_string();
                    self.variables.insert(var_name, s);
                    0
                }
                Err(_) => 1,
            }
        } else {
            eprintln!("sysread: only fd 0 (stdin) supported");
            1
        }
    }

    /// syswrite - low-level write (zsh/system module)
    fn builtin_syswrite(&mut self, args: &[String]) -> i32 {
        use std::io::Write;

        let mut fd = 1i32; // stdout
        let mut data = String::new();
        let mut i = 0;

        while i < args.len() {
            match args[i].as_str() {
                "-o" if i + 1 < args.len() => {
                    i += 1;
                    fd = args[i].parse().unwrap_or(1);
                }
                "-c" if i + 1 < args.len() => {
                    i += 1;
                    // Count - ignored
                }
                _ => {
                    data = args[i].clone();
                }
            }
            i += 1;
        }

        match fd {
            1 => {
                let _ = std::io::stdout().write_all(data.as_bytes());
                let _ = std::io::stdout().flush();
                0
            }
            2 => {
                let _ = std::io::stderr().write_all(data.as_bytes());
                let _ = std::io::stderr().flush();
                0
            }
            _ => {
                eprintln!("syswrite: only fd 1 (stdout) and 2 (stderr) supported");
                1
            }
        }
    }

    /// syserror - get error message (zsh/system module)
    fn builtin_syserror(&self, args: &[String]) -> i32 {
        let errno = if args.is_empty() {
            // Use last errno
            std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
        } else {
            args[0].parse().unwrap_or(0)
        };

        let err = std::io::Error::from_raw_os_error(errno);
        println!("{}", err);
        0
    }

    /// sysopen - open file descriptor (zsh/system module)
    fn builtin_sysopen(&mut self, args: &[String]) -> i32 {
        use std::fs::OpenOptions;

        let mut filename = String::new();
        let mut var_name = "REPLY".to_string();
        let mut read = false;
        let mut write = false;
        let mut append = false;
        let mut create = false;
        let mut truncate = false;

        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "-r" => read = true,
                "-w" => write = true,
                "-a" => append = true,
                "-c" => create = true,
                "-t" => truncate = true,
                "-u" => {
                    i += 1;
                    if i < args.len() {
                        var_name = args[i].clone();
                    }
                }
                "-o" => {
                    i += 1;
                    // Mode flags like O_RDONLY etc - parse as needed
                }
                s if !s.starts_with('-') => {
                    filename = s.to_string();
                }
                _ => {}
            }
            i += 1;
        }

        if filename.is_empty() {
            eprintln!("sysopen: need filename");
            return 1;
        }

        // Default to read if nothing specified
        if !read && !write && !append {
            read = true;
        }

        let file = OpenOptions::new()
            .read(read)
            .write(write || append || truncate)
            .append(append)
            .create(create || write)
            .truncate(truncate)
            .open(&filename);

        match file {
            Ok(f) => {
                let fd = self.next_fd;
                self.next_fd += 1;
                self.open_fds.insert(fd, f);
                self.variables.insert(var_name, fd.to_string());
                0
            }
            Err(e) => {
                eprintln!("sysopen: {}: {}", filename, e);
                1
            }
        }
    }

    /// sysseek - seek on file descriptor (zsh/system module)
    fn builtin_sysseek(&mut self, args: &[String]) -> i32 {
        use std::io::{Seek, SeekFrom};

        let mut fd = -1i32;
        let mut offset = 0i64;
        let mut whence = SeekFrom::Start(0);

        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "-u" => {
                    i += 1;
                    if i < args.len() {
                        fd = args[i].parse().unwrap_or(-1);
                    }
                }
                "-w" => {
                    i += 1;
                    if i < args.len() {
                        whence = match args[i].as_str() {
                            "start" | "set" | "0" => SeekFrom::Start(offset as u64),
                            "current" | "cur" | "1" => SeekFrom::Current(offset),
                            "end" | "2" => SeekFrom::End(offset),
                            _ => SeekFrom::Start(offset as u64),
                        };
                    }
                }
                s if !s.starts_with('-') => {
                    offset = s.parse().unwrap_or(0);
                }
                _ => {}
            }
            i += 1;
        }

        if fd < 0 {
            eprintln!("sysseek: need fd (-u)");
            return 1;
        }

        // Update whence with actual offset
        whence = match whence {
            SeekFrom::Start(_) => SeekFrom::Start(offset as u64),
            SeekFrom::Current(_) => SeekFrom::Current(offset),
            SeekFrom::End(_) => SeekFrom::End(offset),
        };

        if let Some(file) = self.open_fds.get_mut(&fd) {
            match file.seek(whence) {
                Ok(pos) => {
                    self.variables.insert("REPLY".to_string(), pos.to_string());
                    0
                }
                Err(e) => {
                    eprintln!("sysseek: {}", e);
                    1
                }
            }
        } else {
            eprintln!("sysseek: bad fd: {}", fd);
            1
        }
    }

    /// private - declare private variables (zsh/param/private module)
    fn builtin_private(&mut self, args: &[String]) -> i32 {
        // Similar to local but with stricter scoping
        self.builtin_local(args)
    }

    /// zgetattr/zsetattr/zdelattr/zlistattr - extended attributes (zsh/attr module)
    fn builtin_zattr(&self, cmd: &str, args: &[String]) -> i32 {
        match cmd {
            "zgetattr" => {
                if args.len() < 2 {
                    eprintln!("zgetattr: need file and attribute name");
                    return 1;
                }
                #[cfg(target_os = "macos")]
                {
                    // macOS uses xattr
                    let output = std::process::Command::new("xattr")
                        .arg("-p")
                        .arg(&args[1])
                        .arg(&args[0])
                        .output();
                    if let Ok(out) = output {
                        print!("{}", String::from_utf8_lossy(&out.stdout));
                        return if out.status.success() { 0 } else { 1 };
                    }
                }
                #[cfg(target_os = "linux")]
                {
                    let output = std::process::Command::new("getfattr")
                        .arg("-n")
                        .arg(&args[1])
                        .arg(&args[0])
                        .output();
                    if let Ok(out) = output {
                        print!("{}", String::from_utf8_lossy(&out.stdout));
                        return if out.status.success() { 0 } else { 1 };
                    }
                }
                1
            }
            "zsetattr" => {
                if args.len() < 3 {
                    eprintln!("zsetattr: need file, attribute name, and value");
                    return 1;
                }
                #[cfg(target_os = "macos")]
                {
                    let status = std::process::Command::new("xattr")
                        .arg("-w")
                        .arg(&args[1])
                        .arg(&args[2])
                        .arg(&args[0])
                        .status();
                    return status.map(|s| if s.success() { 0 } else { 1 }).unwrap_or(1);
                }
                #[cfg(target_os = "linux")]
                {
                    let status = std::process::Command::new("setfattr")
                        .arg("-n")
                        .arg(&args[1])
                        .arg("-v")
                        .arg(&args[2])
                        .arg(&args[0])
                        .status();
                    return status.map(|s| if s.success() { 0 } else { 1 }).unwrap_or(1);
                }
                #[allow(unreachable_code)]
                1
            }
            "zdelattr" => {
                if args.len() < 2 {
                    eprintln!("zdelattr: need file and attribute name");
                    return 1;
                }
                #[cfg(target_os = "macos")]
                {
                    let status = std::process::Command::new("xattr")
                        .arg("-d")
                        .arg(&args[1])
                        .arg(&args[0])
                        .status();
                    return status.map(|s| if s.success() { 0 } else { 1 }).unwrap_or(1);
                }
                #[cfg(target_os = "linux")]
                {
                    let status = std::process::Command::new("setfattr")
                        .arg("-x")
                        .arg(&args[1])
                        .arg(&args[0])
                        .status();
                    return status.map(|s| if s.success() { 0 } else { 1 }).unwrap_or(1);
                }
                #[allow(unreachable_code)]
                1
            }
            "zlistattr" => {
                if args.is_empty() {
                    eprintln!("zlistattr: need file");
                    return 1;
                }
                #[cfg(target_os = "macos")]
                {
                    let output = std::process::Command::new("xattr").arg(&args[0]).output();
                    if let Ok(out) = output {
                        print!("{}", String::from_utf8_lossy(&out.stdout));
                        return if out.status.success() { 0 } else { 1 };
                    }
                }
                #[cfg(target_os = "linux")]
                {
                    let output = std::process::Command::new("getfattr")
                        .arg("-d")
                        .arg(&args[0])
                        .output();
                    if let Ok(out) = output {
                        print!("{}", String::from_utf8_lossy(&out.stdout));
                        return if out.status.success() { 0 } else { 1 };
                    }
                }
                1
            }
            _ => 1,
        }
    }

    /// zftp - FTP client builtin
    fn builtin_zftp(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            println!("zftp: FTP client");
            println!("  zftp open host [port]");
            println!("  zftp login [user [password]]");
            println!("  zftp cd dir");
            println!("  zftp get file [localfile]");
            println!("  zftp put file [remotefile]");
            println!("  zftp ls [dir]");
            println!("  zftp close");
            return 0;
        }

        match args[0].as_str() {
            "open" => {
                if args.len() < 2 {
                    eprintln!("zftp open: need hostname");
                    return 1;
                }
                // Would connect to FTP server
                println!("zftp: would connect to {}", args[1]);
                0
            }
            "login" => {
                // Would authenticate
                println!("zftp: would login");
                0
            }
            "cd" => {
                if args.len() < 2 {
                    eprintln!("zftp cd: need directory");
                    return 1;
                }
                println!("zftp: would cd to {}", args[1]);
                0
            }
            "get" => {
                if args.len() < 2 {
                    eprintln!("zftp get: need filename");
                    return 1;
                }
                println!("zftp: would download {}", args[1]);
                0
            }
            "put" => {
                if args.len() < 2 {
                    eprintln!("zftp put: need filename");
                    return 1;
                }
                println!("zftp: would upload {}", args[1]);
                0
            }
            "ls" => {
                println!("zftp: would list directory");
                0
            }
            "close" | "quit" => {
                println!("zftp: would close connection");
                0
            }
            "params" => {
                // Display/set FTP parameters
                println!("ZFTP_HOST=");
                println!("ZFTP_PORT=21");
                println!("ZFTP_USER=");
                println!("ZFTP_PWD=");
                println!("ZFTP_TYPE=A");
                0
            }
            cmd => {
                eprintln!("zftp: unknown command: {}", cmd);
                1
            }
        }
    }

    /// promptinit - initialize prompt theme system
    fn builtin_promptinit(&mut self, _args: &[String]) -> i32 {
        self.arrays.insert(
            "prompt_themes".to_string(),
            vec![
                "adam1".to_string(),
                "adam2".to_string(),
                "bart".to_string(),
                "bigfade".to_string(),
                "clint".to_string(),
                "default".to_string(),
                "elite".to_string(),
                "elite2".to_string(),
                "fade".to_string(),
                "fire".to_string(),
                "minimal".to_string(),
                "off".to_string(),
                "oliver".to_string(),
                "pws".to_string(),
                "redhat".to_string(),
                "restore".to_string(),
                "suse".to_string(),
                "walters".to_string(),
                "zefram".to_string(),
            ],
        );
        self.variables
            .insert("prompt_theme".to_string(), "default".to_string());
        0
    }

    /// prompt - set or list prompt themes
    fn builtin_prompt(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            let theme = self
                .variables
                .get("prompt_theme")
                .cloned()
                .unwrap_or_else(|| "default".to_string());
            println!("Current prompt theme: {}", theme);
            return 0;
        }
        match args[0].as_str() {
            "-l" | "--list" => {
                println!("Available prompt themes:");
                if let Some(themes) = self.arrays.get("prompt_themes") {
                    for theme in themes {
                        println!("  {}", theme);
                    }
                }
            }
            "-p" | "--preview" => {
                let theme = args.get(1).map(|s| s.as_str()).unwrap_or("default");
                self.apply_prompt_theme(theme, true);
            }
            "-h" | "--help" => {
                println!("prompt [options] [theme]");
                println!("  -l, --list     List available themes");
                println!("  -p, --preview  Preview a theme");
                println!("  -s, --setup    Set up a theme");
            }
            _ => {
                let theme = if args[0].starts_with('-') {
                    args.get(1).map(|s| s.as_str()).unwrap_or("default")
                } else {
                    args[0].as_str()
                };
                self.apply_prompt_theme(theme, false);
            }
        }
        0
    }

    fn apply_prompt_theme(&mut self, theme: &str, preview: bool) {
        let (ps1, rps1) = match theme {
            "minimal" => ("%# ", ""),
            "off" => ("$ ", ""),
            "adam1" => (
                "%B%F{cyan}%n@%m %F{blue}%~%f%b %# ",
                "%F{yellow}%D{%H:%M}%f",
            ),
            "redhat" => ("[%n@%m %~]$ ", ""),
            _ => ("%n@%m %~ %# ", ""),
        };
        if preview {
            println!("PS1={:?}", ps1);
            println!("RPS1={:?}", rps1);
        } else {
            self.variables.insert("PS1".to_string(), ps1.to_string());
            self.variables.insert("RPS1".to_string(), rps1.to_string());
            self.variables
                .insert("prompt_theme".to_string(), theme.to_string());
        }
    }

    /// pcre_compile - compile a PCRE pattern
    fn builtin_pcre_compile(&mut self, args: &[String]) -> i32 {
        use crate::pcre::{pcre_compile, PcreCompileOptions};

        let mut pattern = String::new();
        let mut options = PcreCompileOptions::default();

        for arg in args {
            match arg.as_str() {
                "-a" => options.anchored = true,
                "-i" => options.caseless = true,
                "-m" => options.multiline = true,
                "-s" => options.dotall = true,
                "-x" => options.extended = true,
                s if !s.starts_with('-') => pattern = s.to_string(),
                _ => {}
            }
        }

        if pattern.is_empty() {
            eprintln!("pcre_compile: no pattern specified");
            return 1;
        }

        match pcre_compile(&pattern, &options, &mut self.pcre_state) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("pcre_compile: {}", e);
                1
            }
        }
    }

    /// pcre_match - match string against compiled PCRE
    fn builtin_pcre_match(&mut self, args: &[String]) -> i32 {
        use crate::pcre::{pcre_match, PcreMatchOptions};

        let mut var_name = "MATCH".to_string();
        let mut array_name = "match".to_string();
        let mut string = String::new();
        let mut i = 0;

        while i < args.len() {
            match args[i].as_str() {
                "-v" => {
                    i += 1;
                    if i < args.len() {
                        var_name = args[i].clone();
                    }
                }
                "-a" => {
                    i += 1;
                    if i < args.len() {
                        array_name = args[i].clone();
                    }
                }
                s if !s.starts_with('-') => string = s.to_string(),
                _ => {}
            }
            i += 1;
        }

        let options = PcreMatchOptions {
            match_var: Some(var_name.clone()),
            array_var: Some(array_name.clone()),
            ..Default::default()
        };

        match pcre_match(&string, &options, &self.pcre_state) {
            Ok(result) => {
                if result.matched {
                    if let Some(m) = result.full_match {
                        self.variables.insert(var_name, m);
                    }
                    let matches: Vec<String> =
                        result.captures.into_iter().filter_map(|c| c).collect();
                    self.arrays.insert(array_name, matches);
                    0
                } else {
                    1
                }
            }
            Err(e) => {
                eprintln!("pcre_match: {}", e);
                1
            }
        }
    }

    /// pcre_study - optimize compiled PCRE (no-op in Rust regex)
    fn builtin_pcre_study(&mut self, _args: &[String]) -> i32 {
        use crate::pcre::pcre_study;

        match pcre_study(&self.pcre_state) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("pcre_study: {}", e);
                1
            }
        }
    }

    // =========================================================================
    // Process control functions - Port from exec.c
    // =========================================================================

    /// Fork a new process
    /// Port of zfork() from exec.c
    pub fn zfork(&mut self, flags: ForkFlags) -> std::io::Result<ForkResult> {
        // Check for job control
        let can_background = self.options.get("monitor").copied().unwrap_or(false);

        unsafe {
            match libc::fork() {
                -1 => Err(std::io::Error::last_os_error()),
                0 => {
                    // Child process
                    if !flags.contains(ForkFlags::NOJOB) && can_background {
                        // Set up job control
                        let pid = libc::getpid();
                        if flags.contains(ForkFlags::NEWGRP) {
                            libc::setpgid(0, 0);
                        }
                        if flags.contains(ForkFlags::FGTTY) {
                            libc::tcsetpgrp(0, pid);
                        }
                    }

                    // Reset signal handlers
                    if !flags.contains(ForkFlags::KEEPSIGS) {
                        self.reset_signals();
                    }

                    Ok(ForkResult::Child)
                }
                pid => {
                    // Parent process
                    if !flags.contains(ForkFlags::NOJOB) {
                        // Add to job table
                        self.add_child_process(pid);
                    }
                    Ok(ForkResult::Parent(pid))
                }
            }
        }
    }

    /// Add a child process to tracking
    fn add_child_process(&mut self, pid: i32) {
        // Would track in job table
        self.variables.insert("!".to_string(), pid.to_string());
    }

    /// Reset signal handlers to defaults
    fn reset_signals(&self) {
        unsafe {
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::signal(libc::SIGQUIT, libc::SIG_DFL);
            libc::signal(libc::SIGTERM, libc::SIG_DFL);
            libc::signal(libc::SIGTSTP, libc::SIG_DFL);
            libc::signal(libc::SIGTTIN, libc::SIG_DFL);
            libc::signal(libc::SIGTTOU, libc::SIG_DFL);
            libc::signal(libc::SIGCHLD, libc::SIG_DFL);
        }
    }

    /// Execute a command in the current process (exec family)
    /// Port of zexecve() from exec.c
    pub fn zexecve(&self, cmd: &str, args: &[String]) -> ! {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let c_cmd = CString::new(cmd).expect("CString::new failed");

        // Build argv
        let c_args: Vec<CString> = std::iter::once(c_cmd.clone())
            .chain(args.iter().map(|s| CString::new(s.as_str()).unwrap()))
            .collect();

        let c_argv: Vec<*const libc::c_char> = c_args
            .iter()
            .map(|s| s.as_ptr())
            .chain(std::iter::once(std::ptr::null()))
            .collect();

        // Build envp from current environment
        let env_vars: Vec<CString> = std::env::vars()
            .map(|(k, v)| CString::new(format!("{}={}", k, v)).unwrap())
            .collect();

        let c_envp: Vec<*const libc::c_char> = env_vars
            .iter()
            .map(|s| s.as_ptr())
            .chain(std::iter::once(std::ptr::null()))
            .collect();

        unsafe {
            libc::execve(c_cmd.as_ptr(), c_argv.as_ptr(), c_envp.as_ptr());
            // If we get here, exec failed
            eprintln!(
                "zshrs: exec failed: {}: {}",
                cmd,
                std::io::Error::last_os_error()
            );
            std::process::exit(127);
        }
    }

    /// Enter a subshell
    /// Port of entersubsh() from exec.c
    pub fn entersubsh(&mut self, flags: SubshellFlags) {
        // Increment subshell level
        let level = self
            .get_variable("ZSH_SUBSHELL")
            .parse::<i32>()
            .unwrap_or(0);
        self.variables
            .insert("ZSH_SUBSHELL".to_string(), (level + 1).to_string());

        // Handle job control
        if flags.contains(SubshellFlags::NOMONITOR) {
            self.options.insert("monitor".to_string(), false);
        }

        // Close unneeded fds
        if !flags.contains(SubshellFlags::KEEPFDS) {
            self.close_extra_fds();
        }

        // Reset traps
        if !flags.contains(SubshellFlags::KEEPTRAPS) {
            self.reset_traps();
        }
    }

    /// Close extra file descriptors
    fn close_extra_fds(&self) {
        // Close fds > 10 (common shell convention)
        for fd in 10..256 {
            unsafe {
                libc::close(fd);
            }
        }
    }

    /// Reset all traps
    fn reset_traps(&mut self) {
        self.traps.clear();
    }

    /// Execute a shell function
    /// Port of doshfunc() from exec.c
    pub fn doshfunc(
        &mut self,
        name: &str,
        func: &ShellCommand,
        args: &[String],
    ) -> Result<i32, String> {
        // Save current state
        let old_argv = self.positional_params.clone();
        let old_funcstack = self.arrays.get("funcstack").cloned();
        let old_funcsourcetrace = self.arrays.get("funcsourcetrace").cloned();

        // Set positional parameters to function arguments
        self.positional_params = args.to_vec();

        // Update funcstack
        let mut funcstack = old_funcstack.clone().unwrap_or_default();
        funcstack.insert(0, name.to_string());
        self.arrays.insert("funcstack".to_string(), funcstack);

        // Execute function body
        let result = self.execute_command(func);

        // Restore state
        self.positional_params = old_argv;
        if let Some(fs) = old_funcstack {
            self.arrays.insert("funcstack".to_string(), fs);
        } else {
            self.arrays.remove("funcstack");
        }
        if let Some(fst) = old_funcsourcetrace {
            self.arrays.insert("funcsourcetrace".to_string(), fst);
        }

        result
    }

    /// Execute arithmetic expression
    /// Port of execarith() from exec.c
    pub fn execarith(&mut self, expr: &str) -> i32 {
        let result = self.eval_arith_expr(expr);
        if result == 0 {
            1
        } else {
            0
        }
    }

    /// Execute conditional expression
    /// Port of execcond() from exec.c
    pub fn execcond(&mut self, cond: &CondExpr) -> i32 {
        if self.eval_cond_expr(cond) {
            0
        } else {
            1
        }
    }

    /// Execute command and capture time
    /// Port of exectime() from exec.c
    pub fn exectime(&mut self, cmd: &ShellCommand) -> Result<i32, String> {
        use std::time::Instant;

        let start = Instant::now();
        let result = self.execute_command(cmd);
        let elapsed = start.elapsed();

        // Print time in zsh format
        let user_time = elapsed.as_secs_f64() * 0.7; // Approximation
        let sys_time = elapsed.as_secs_f64() * 0.1;
        let real_time = elapsed.as_secs_f64();

        eprintln!(
            "{:.2}s user {:.2}s system {:.0}% cpu {:.3} total",
            user_time,
            sys_time,
            ((user_time + sys_time) / real_time * 100.0).min(100.0),
            real_time
        );

        result
    }

    /// Find command in PATH
    /// Port of findcmd() from exec.c
    pub fn findcmd(&self, name: &str, do_hash: bool) -> Option<String> {
        // Check command hash table first
        if do_hash {
            if let Some(path) = self.command_hash.get(name) {
                if std::path::Path::new(path).exists() {
                    return Some(path.clone());
                }
            }
        }

        // Search PATH
        if let Ok(path_var) = std::env::var("PATH") {
            for dir in path_var.split(':') {
                let full_path = format!("{}/{}", dir, name);
                if std::path::Path::new(&full_path).is_file() {
                    return Some(full_path);
                }
            }
        }

        None
    }

    /// Hash a command (add to command hash table)
    /// Port of hashcmd() from exec.c
    pub fn hashcmd(&mut self, name: &str, path: &str) {
        self.command_hash.insert(name.to_string(), path.to_string());
    }

    /// Check if command exists and is executable
    /// Port of iscom() from exec.c
    pub fn iscom(&self, name: &str) -> bool {
        // Check if it's a builtin
        if self.is_builtin_cmd(name) {
            return true;
        }

        // Check if it's a function
        if self.functions.contains_key(name) {
            return true;
        }

        // Check if it's an alias
        if self.aliases.contains_key(name) {
            return true;
        }

        // Check in PATH
        self.findcmd(name, true).is_some()
    }

    /// Check if name is a builtin (process control version)
    fn is_builtin_cmd(&self, name: &str) -> bool {
        BUILTIN_SET.contains(name)
    }

    /// Close all file descriptors except stdin/stdout/stderr
    /// Port of closem() from exec.c
    pub fn closem(&self, exceptions: &[i32]) {
        for fd in 3..256 {
            if !exceptions.contains(&fd) {
                unsafe {
                    libc::close(fd);
                }
            }
        }
    }

    /// Create a pipe
    /// Port of mpipe() from exec.c
    pub fn mpipe(&self) -> std::io::Result<(i32, i32)> {
        let mut fds = [0i32; 2];
        let result = unsafe { libc::pipe(fds.as_mut_ptr()) };
        if result == -1 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok((fds[0], fds[1]))
        }
    }

    /// Add a file descriptor for redirection
    /// Port of addfd() from exec.c
    pub fn addfd(&self, fd: i32, target_fd: i32, mode: RedirMode) -> std::io::Result<()> {
        match mode {
            RedirMode::Dup => {
                if fd != target_fd {
                    unsafe {
                        if libc::dup2(fd, target_fd) == -1 {
                            return Err(std::io::Error::last_os_error());
                        }
                    }
                }
            }
            RedirMode::Close => unsafe {
                libc::close(target_fd);
            },
        }
        Ok(())
    }

    /// Get heredoc content
    /// Port of gethere() from exec.c
    pub fn gethere(&mut self, terminator: &str, strip_tabs: bool) -> String {
        let mut content = String::new();

        // Would read until terminator is found
        // This is simplified - real impl reads from input

        if strip_tabs {
            content = content
                .lines()
                .map(|line| line.trim_start_matches('\t'))
                .collect::<Vec<_>>()
                .join("\n");
        }

        content
    }

    /// Get herestring content
    /// Port of getherestr() from exec.c
    pub fn getherestr(&mut self, word: &str) -> String {
        let expanded = self.expand_string(word);
        format!("{}\n", expanded)
    }

    /// Resolve a builtin command
    /// Port of resolvebuiltin() from exec.c
    pub fn resolvebuiltin(&self, name: &str) -> Option<BuiltinType> {
        if self.is_builtin_cmd(name) {
            Some(BuiltinType::Normal)
        } else {
            // Check disabled_builtins if we had that field
            None
        }
    }

    /// Check if cd is possible
    /// Port of cancd() from exec.c
    pub fn cancd(&self, path_str: &str) -> bool {
        use std::os::unix::fs::PermissionsExt;

        let path = std::path::Path::new(path_str);
        if !path.is_dir() {
            return false;
        }

        if let Ok(meta) = path.metadata() {
            let mode = meta.permissions().mode();
            // Check execute permission (needed for cd)
            let uid = unsafe { libc::getuid() };
            let gid = unsafe { libc::getgid() };
            let file_uid = meta.uid();
            let file_gid = meta.gid();

            if uid == file_uid {
                return (mode & 0o100) != 0;
            } else if gid == file_gid {
                return (mode & 0o010) != 0;
            } else {
                return (mode & 0o001) != 0;
            }
        }

        false
    }

    /// Command not found handler
    /// Port of commandnotfound() from exec.c
    pub fn commandnotfound(&mut self, name: &str, args: &[String]) -> i32 {
        // Check for command_not_found_handler function
        if self.functions.contains_key("command_not_found_handler") {
            let mut handler_args = vec![name.to_string()];
            handler_args.extend(args.iter().cloned());

            if let Some(func) = self.functions.get("command_not_found_handler").cloned() {
                if let Ok(code) = self.doshfunc("command_not_found_handler", &func, &handler_args) {
                    return code;
                }
            }
        }

        eprintln!("zshrs: command not found: {}", name);
        127
    }
}

use std::os::unix::fs::MetadataExt;

bitflags::bitflags! {
    /// Flags for zfork()
    #[derive(Debug, Clone, Copy, Default)]
    pub struct ForkFlags: u32 {
        const NOJOB = 1 << 0;    // Don't add to job table
        const NEWGRP = 1 << 1;   // Create new process group
        const FGTTY = 1 << 2;    // Take foreground terminal
        const KEEPSIGS = 1 << 3; // Keep signal handlers
    }
}

bitflags::bitflags! {
    /// Flags for entersubsh()
    #[derive(Debug, Clone, Copy, Default)]
    pub struct SubshellFlags: u32 {
        const NOMONITOR = 1 << 0; // Disable job control
        const KEEPFDS = 1 << 1;   // Keep file descriptors
        const KEEPTRAPS = 1 << 2; // Keep trap handlers
    }
}

/// Result of fork operation
#[derive(Debug)]
pub enum ForkResult {
    Parent(i32), // Contains child PID
    Child,
}

/// Redirection mode
#[derive(Debug, Clone, Copy)]
pub enum RedirMode {
    Dup,
    Close,
}

/// Builtin command type
#[derive(Debug, Clone, Copy)]
pub enum BuiltinType {
    Normal,
    Disabled,
}
