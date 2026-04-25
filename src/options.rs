//! Shell options for zshrs
//!
//! Direct port from zsh/Src/options.c
//!
//! Manages all shell options including:
//! - Option lookup by name and single-letter
//! - Emulation modes (zsh, ksh, sh, csh)
//! - Option aliases (bash/ksh compatibility)
//! - setopt/unsetopt builtins

use std::collections::HashMap;

/// Emulation modes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Emulation {
    Zsh = 1,
    Csh = 2,
    Ksh = 4,
    Sh = 8,
}

/// Emulation flags for option defaults
const OPT_CSH: u8 = 1;
const OPT_KSH: u8 = 2;
const OPT_SH: u8 = 4;
const OPT_ZSH: u8 = 8;
const OPT_ALL: u8 = OPT_CSH | OPT_KSH | OPT_SH | OPT_ZSH;
const OPT_BOURNE: u8 = OPT_KSH | OPT_SH;
const OPT_BSHELL: u8 = OPT_KSH | OPT_SH | OPT_ZSH;
const OPT_NONBOURNE: u8 = OPT_ALL & !OPT_BOURNE;
const OPT_NONZSH: u8 = OPT_ALL & !OPT_ZSH;

/// Option flags
const OPT_EMULATE: u16 = 0x100; // Relevant to emulation
const OPT_SPECIAL: u16 = 0x200; // Never set by emulate()
const OPT_ALIAS: u16 = 0x400; // Alias to another option

/// All shell option names
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum ShellOption {
    // A
    Aliases = 1,
    AliasFuncDef,
    AllExport,
    AlwaysLastPrompt,
    AlwaysToEnd,
    AppendCreate,
    AppendHistory,
    AutoCd,
    AutoContinue,
    AutoList,
    AutoMenu,
    AutoNamedDirs,
    AutoParamKeys,
    AutoParamSlash,
    AutoPushd,
    AutoRemoveSlash,
    AutoResume,
    // B
    BadPattern,
    BangHist,
    BareGlobQual,
    BashAutoList,
    BashRematch,
    Beep,
    BgNice,
    BraceCcl,
    BsdEcho,
    // C
    CaseGlob,
    CaseMatch,
    CasePaths,
    CBases,
    CPrecedences,
    CdAbleVars,
    CdSilent,
    ChaseDots,
    ChaseLinks,
    CheckJobs,
    CheckRunningJobs,
    Clobber,
    ClobberEmpty,
    CombiningChars,
    CompleteAliases,
    CompleteInWord,
    ContinueOnError,
    Correct,
    CorrectAll,
    CshJunkieHistory,
    CshJunkieLoops,
    CshJunkieQuotes,
    CshNullCmd,
    CshNullGlob,
    // D
    DebugBeforeCmd,
    // E
    Emacs,
    Equals,
    ErrExit,
    ErrReturn,
    Exec,
    ExtendedGlob,
    ExtendedHistory,
    EvalLineno,
    // F
    FlowControl,
    ForceFloat,
    FunctionArgZero,
    // G
    Glob,
    GlobalExport,
    GlobalRcs,
    GlobAssign,
    GlobComplete,
    GlobDots,
    GlobStarShort,
    GlobSubst,
    // H
    HashCmds,
    HashDirs,
    HashExecutablesOnly,
    HashListAll,
    HistAllowClobber,
    HistBeep,
    HistExpireDupsFirst,
    HistFcntlLock,
    HistFindNoDups,
    HistIgnoreAllDups,
    HistIgnoreDups,
    HistIgnoreSpace,
    HistLexWords,
    HistNoFunctions,
    HistNoStore,
    HistSubstPattern,
    HistReduceBlanks,
    HistSaveByCopy,
    HistSaveNoDups,
    HistVerify,
    Hup,
    // I
    IgnoreBraces,
    IgnoreCloseBraces,
    IgnoreEof,
    IncAppendHistory,
    IncAppendHistoryTime,
    Interactive,
    InteractiveComments,
    // K
    KshArrays,
    KshAutoload,
    KshGlob,
    KshOptionPrint,
    KshTypeset,
    KshZeroSubscript,
    // L
    ListAmbiguous,
    ListBeep,
    ListPacked,
    ListRowsFirst,
    ListTypes,
    LocalOptions,
    LocalLoops,
    LocalPatterns,
    LocalTraps,
    Login,
    LongListJobs,
    // M
    MagicEqualSubst,
    MailWarning,
    MarkDirs,
    MenuComplete,
    Monitor,
    MultiByte,
    MultiFuncDef,
    MultiOs,
    // N
    NoMatch,
    Notify,
    NullGlob,
    NumericGlobSort,
    // O
    OctalZeroes,
    OverStrike,
    // P
    PathDirs,
    PathScript,
    PipeFail,
    PosixAliases,
    PosixArgZero,
    PosixBuiltins,
    PosixCd,
    PosixIdentifiers,
    PosixJobs,
    PosixStrings,
    PosixTraps,
    PrintEightBit,
    PrintExitValue,
    Privileged,
    PromptBang,
    PromptCr,
    PromptPercent,
    PromptSp,
    PromptSubst,
    PushdIgnoreDups,
    PushdMinus,
    PushdSilent,
    PushdToHome,
    // R
    RcExpandParam,
    RcQuotes,
    Rcs,
    RecExact,
    RematchPcre,
    RmStarSilent,
    RmStarWait,
    // S
    ShareHistory,
    ShFileExpansion,
    ShGlob,
    ShInstdin,
    ShNullCmd,
    ShOptionLetters,
    ShortLoops,
    ShortRepeat,
    ShWordSplit,
    SingleCommand,
    SingleLineZle,
    SourceTrace,
    SunKeyboardHack,
    // T
    TransientRprompt,
    TrapsAsync,
    TypesetSilent,
    TypesetToUnset,
    // U
    Unset,
    // V
    Verbose,
    Vi,
    // W
    WarnCreateGlobal,
    WarnNestedVar,
    // X
    Xtrace,
    // Z
    Zle,
    Dvorak,
}

impl ShellOption {
    /// Get the canonical name of this option
    pub fn name(self) -> &'static str {
        match self {
            Self::Aliases => "aliases",
            Self::AliasFuncDef => "aliasfuncdef",
            Self::AllExport => "allexport",
            Self::AlwaysLastPrompt => "alwayslastprompt",
            Self::AlwaysToEnd => "alwaystoend",
            Self::AppendCreate => "appendcreate",
            Self::AppendHistory => "appendhistory",
            Self::AutoCd => "autocd",
            Self::AutoContinue => "autocontinue",
            Self::AutoList => "autolist",
            Self::AutoMenu => "automenu",
            Self::AutoNamedDirs => "autonamedirs",
            Self::AutoParamKeys => "autoparamkeys",
            Self::AutoParamSlash => "autoparamslash",
            Self::AutoPushd => "autopushd",
            Self::AutoRemoveSlash => "autoremoveslash",
            Self::AutoResume => "autoresume",
            Self::BadPattern => "badpattern",
            Self::BangHist => "banghist",
            Self::BareGlobQual => "bareglobqual",
            Self::BashAutoList => "bashautolist",
            Self::BashRematch => "bashrematch",
            Self::Beep => "beep",
            Self::BgNice => "bgnice",
            Self::BraceCcl => "braceccl",
            Self::BsdEcho => "bsdecho",
            Self::CaseGlob => "caseglob",
            Self::CaseMatch => "casematch",
            Self::CasePaths => "casepaths",
            Self::CBases => "cbases",
            Self::CPrecedences => "cprecedences",
            Self::CdAbleVars => "cdablevars",
            Self::CdSilent => "cdsilent",
            Self::ChaseDots => "chasedots",
            Self::ChaseLinks => "chaselinks",
            Self::CheckJobs => "checkjobs",
            Self::CheckRunningJobs => "checkrunningjobs",
            Self::Clobber => "clobber",
            Self::ClobberEmpty => "clobberempty",
            Self::CombiningChars => "combiningchars",
            Self::CompleteAliases => "completealiases",
            Self::CompleteInWord => "completeinword",
            Self::ContinueOnError => "continueonerror",
            Self::Correct => "correct",
            Self::CorrectAll => "correctall",
            Self::CshJunkieHistory => "cshjunkiehistory",
            Self::CshJunkieLoops => "cshjunkieloops",
            Self::CshJunkieQuotes => "cshjunkiequotes",
            Self::CshNullCmd => "cshnullcmd",
            Self::CshNullGlob => "cshnullglob",
            Self::DebugBeforeCmd => "debugbeforecmd",
            Self::Emacs => "emacs",
            Self::Equals => "equals",
            Self::ErrExit => "errexit",
            Self::ErrReturn => "errreturn",
            Self::Exec => "exec",
            Self::ExtendedGlob => "extendedglob",
            Self::ExtendedHistory => "extendedhistory",
            Self::EvalLineno => "evallineno",
            Self::FlowControl => "flowcontrol",
            Self::ForceFloat => "forcefloat",
            Self::FunctionArgZero => "functionargzero",
            Self::Glob => "glob",
            Self::GlobalExport => "globalexport",
            Self::GlobalRcs => "globalrcs",
            Self::GlobAssign => "globassign",
            Self::GlobComplete => "globcomplete",
            Self::GlobDots => "globdots",
            Self::GlobStarShort => "globstarshort",
            Self::GlobSubst => "globsubst",
            Self::HashCmds => "hashcmds",
            Self::HashDirs => "hashdirs",
            Self::HashExecutablesOnly => "hashexecutablesonly",
            Self::HashListAll => "hashlistall",
            Self::HistAllowClobber => "histallowclobber",
            Self::HistBeep => "histbeep",
            Self::HistExpireDupsFirst => "histexpiredupsfirst",
            Self::HistFcntlLock => "histfcntllock",
            Self::HistFindNoDups => "histfindnodups",
            Self::HistIgnoreAllDups => "histignorealldups",
            Self::HistIgnoreDups => "histignoredups",
            Self::HistIgnoreSpace => "histignorespace",
            Self::HistLexWords => "histlexwords",
            Self::HistNoFunctions => "histnofunctions",
            Self::HistNoStore => "histnostore",
            Self::HistSubstPattern => "histsubstpattern",
            Self::HistReduceBlanks => "histreduceblanks",
            Self::HistSaveByCopy => "histsavebycopy",
            Self::HistSaveNoDups => "histsavenodups",
            Self::HistVerify => "histverify",
            Self::Hup => "hup",
            Self::IgnoreBraces => "ignorebraces",
            Self::IgnoreCloseBraces => "ignoreclosebraces",
            Self::IgnoreEof => "ignoreeof",
            Self::IncAppendHistory => "incappendhistory",
            Self::IncAppendHistoryTime => "incappendhistorytime",
            Self::Interactive => "interactive",
            Self::InteractiveComments => "interactivecomments",
            Self::KshArrays => "ksharrays",
            Self::KshAutoload => "kshautoload",
            Self::KshGlob => "kshglob",
            Self::KshOptionPrint => "kshoptionprint",
            Self::KshTypeset => "kshtypeset",
            Self::KshZeroSubscript => "kshzerosubscript",
            Self::ListAmbiguous => "listambiguous",
            Self::ListBeep => "listbeep",
            Self::ListPacked => "listpacked",
            Self::ListRowsFirst => "listrowsfirst",
            Self::ListTypes => "listtypes",
            Self::LocalOptions => "localoptions",
            Self::LocalLoops => "localloops",
            Self::LocalPatterns => "localpatterns",
            Self::LocalTraps => "localtraps",
            Self::Login => "login",
            Self::LongListJobs => "longlistjobs",
            Self::MagicEqualSubst => "magicequalsubst",
            Self::MailWarning => "mailwarning",
            Self::MarkDirs => "markdirs",
            Self::MenuComplete => "menucomplete",
            Self::Monitor => "monitor",
            Self::MultiByte => "multibyte",
            Self::MultiFuncDef => "multifuncdef",
            Self::MultiOs => "multios",
            Self::NoMatch => "nomatch",
            Self::Notify => "notify",
            Self::NullGlob => "nullglob",
            Self::NumericGlobSort => "numericglobsort",
            Self::OctalZeroes => "octalzeroes",
            Self::OverStrike => "overstrike",
            Self::PathDirs => "pathdirs",
            Self::PathScript => "pathscript",
            Self::PipeFail => "pipefail",
            Self::PosixAliases => "posixaliases",
            Self::PosixArgZero => "posixargzero",
            Self::PosixBuiltins => "posixbuiltins",
            Self::PosixCd => "posixcd",
            Self::PosixIdentifiers => "posixidentifiers",
            Self::PosixJobs => "posixjobs",
            Self::PosixStrings => "posixstrings",
            Self::PosixTraps => "posixtraps",
            Self::PrintEightBit => "printeightbit",
            Self::PrintExitValue => "printexitvalue",
            Self::Privileged => "privileged",
            Self::PromptBang => "promptbang",
            Self::PromptCr => "promptcr",
            Self::PromptPercent => "promptpercent",
            Self::PromptSp => "promptsp",
            Self::PromptSubst => "promptsubst",
            Self::PushdIgnoreDups => "pushdignoredups",
            Self::PushdMinus => "pushdminus",
            Self::PushdSilent => "pushdsilent",
            Self::PushdToHome => "pushdtohome",
            Self::RcExpandParam => "rcexpandparam",
            Self::RcQuotes => "rcquotes",
            Self::Rcs => "rcs",
            Self::RecExact => "recexact",
            Self::RematchPcre => "rematchpcre",
            Self::RmStarSilent => "rmstarsilent",
            Self::RmStarWait => "rmstarwait",
            Self::ShareHistory => "sharehistory",
            Self::ShFileExpansion => "shfileexpansion",
            Self::ShGlob => "shglob",
            Self::ShInstdin => "shinstdin",
            Self::ShNullCmd => "shnullcmd",
            Self::ShOptionLetters => "shoptionletters",
            Self::ShortLoops => "shortloops",
            Self::ShortRepeat => "shortrepeat",
            Self::ShWordSplit => "shwordsplit",
            Self::SingleCommand => "singlecommand",
            Self::SingleLineZle => "singlelinezle",
            Self::SourceTrace => "sourcetrace",
            Self::SunKeyboardHack => "sunkeyboardhack",
            Self::TransientRprompt => "transientrprompt",
            Self::TrapsAsync => "trapsasync",
            Self::TypesetSilent => "typesetsilent",
            Self::TypesetToUnset => "typesettounset",
            Self::Unset => "unset",
            Self::Verbose => "verbose",
            Self::Vi => "vi",
            Self::WarnCreateGlobal => "warncreateglobal",
            Self::WarnNestedVar => "warnnestedvar",
            Self::Xtrace => "xtrace",
            Self::Zle => "zle",
            Self::Dvorak => "dvorak",
        }
    }
}

/// Option aliases for bash/ksh compatibility
pub static OPTION_ALIASES: &[(&str, &str, bool)] = &[
    ("braceexpand", "ignorebraces", true),  // ksh/bash, negated
    ("dotglob", "globdots", false),         // bash
    ("hashall", "hashcmds", false),         // bash
    ("histappend", "appendhistory", false), // bash
    ("histexpand", "banghist", false),      // bash
    ("log", "histnofunctions", true),       // ksh, negated
    ("mailwarn", "mailwarning", false),     // bash
    ("onecmd", "singlecommand", false),     // bash
    ("physical", "chaselinks", false),      // ksh/bash
    ("promptvars", "promptsubst", false),   // bash
    ("stdin", "shinstdin", false),          // ksh
    ("trackall", "hashcmds", false),        // ksh
];

/// Zsh single-letter options (zshletters in C)
pub static ZSH_LETTERS: &[(char, &str, bool)] = &[
    ('0', "correct", false),
    ('1', "printexitvalue", false),
    ('2', "badpattern", true),
    ('3', "nomatch", true),
    ('4', "globdots", false),
    ('5', "notify", false),
    ('6', "bgnice", false),
    ('7', "ignoreeof", false),
    ('8', "markdirs", false),
    ('9', "autolist", false),
    ('B', "beep", true),
    ('C', "clobber", true),
    ('D', "pushdtohome", false),
    ('E', "pushdsilent", false),
    ('F', "glob", true),
    ('G', "nullglob", false),
    ('H', "rmstarsilent", false),
    ('I', "ignorebraces", false),
    ('J', "autocd", false),
    ('K', "banghist", true),
    ('L', "sunkeyboardhack", false),
    ('M', "singlelinezle", false),
    ('N', "autopushd", false),
    ('O', "correctall", false),
    ('P', "rcexpandparam", false),
    ('Q', "pathdirs", false),
    ('R', "longlistjobs", false),
    ('S', "recexact", false),
    ('T', "cdablevars", false),
    ('U', "mailwarning", false),
    ('V', "promptcr", true),
    ('W', "autoresume", false),
    ('X', "listtypes", false),
    ('Y', "menucomplete", false),
    ('Z', "zle", false),
    ('a', "allexport", false),
    ('d', "globalrcs", true),
    ('e', "errexit", false),
    ('f', "rcs", true),
    ('g', "histignorespace", false),
    ('h', "histignoredups", false),
    ('i', "interactive", false),
    ('k', "interactivecomments", false),
    ('l', "login", false),
    ('m', "monitor", false),
    ('n', "exec", true),
    ('p', "privileged", false),
    ('s', "shinstdin", false),
    ('t', "singlecommand", false),
    ('u', "unset", true),
    ('v', "verbose", false),
    ('w', "chaselinks", false),
    ('x', "xtrace", false),
    ('y', "shwordsplit", false),
];

/// Ksh single-letter options
pub static KSH_LETTERS: &[(char, &str, bool)] = &[
    ('C', "clobber", true),
    ('T', "trapsasync", false),
    ('X', "markdirs", false),
    ('a', "allexport", false),
    ('b', "notify", false),
    ('e', "errexit", false),
    ('f', "glob", true),
    ('i', "interactive", false),
    ('l', "login", false),
    ('m', "monitor", false),
    ('n', "exec", true),
    ('p', "privileged", false),
    ('s', "shinstdin", false),
    ('t', "singlecommand", false),
    ('u', "unset", true),
    ('v', "verbose", false),
    ('x', "xtrace", false),
];

/// Shell options manager
#[derive(Debug, Clone)]
pub struct ShellOptions {
    /// Current option values (true = set)
    options: HashMap<String, bool>,
    /// Current emulation mode
    pub emulation: Emulation,
    /// Is fully emulating (vs just setting some options)
    pub fully_emulating: bool,
}

impl Default for ShellOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl ShellOptions {
    /// Create a new options manager with zsh defaults
    pub fn new() -> Self {
        let mut opts = ShellOptions {
            options: HashMap::new(),
            emulation: Emulation::Zsh,
            fully_emulating: false,
        };
        opts.set_zsh_defaults();
        opts
    }

    /// Set zsh default options
    pub fn set_zsh_defaults(&mut self) {
        // Options that default to ON in zsh
        let default_on = [
            "aliases",
            "alwayslastprompt",
            "appendhistory",
            "autolist",
            "automenu",
            "autoparamkeys",
            "autoparamslash",
            "autoremoveslash",
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
            "multifuncdef",
            "multios",
            "nomatch",
            "notify",
            "promptcr",
            "promptpercent",
            "promptsp",
            "rcs",
            "shortloops",
            "unset",
            "zle",
        ];

        for opt in default_on {
            self.options.insert(opt.to_string(), true);
        }
    }

    /// Look up an option by name (case insensitive, underscores ignored)
    pub fn lookup(&self, name: &str) -> Option<bool> {
        let normalized = normalize_option_name(name);

        // Check for "no" prefix
        if let Some(stripped) = normalized.strip_prefix("no") {
            self.options.get(stripped).map(|v| !v)
        } else {
            self.options.get(&normalized).copied()
        }
    }

    /// Check if an option is set
    pub fn is_set(&self, name: &str) -> bool {
        self.lookup(name).unwrap_or(false)
    }

    /// Set an option value
    pub fn set(&mut self, name: &str, value: bool) -> Result<(), String> {
        let normalized = normalize_option_name(name);

        // Handle "no" prefix
        let (actual_name, actual_value) = if let Some(stripped) = normalized.strip_prefix("no") {
            (stripped.to_string(), !value)
        } else {
            (normalized, value)
        };

        // Check for aliases
        for (alias, target, negated) in OPTION_ALIASES {
            if actual_name == *alias {
                let target_value = if *negated {
                    !actual_value
                } else {
                    actual_value
                };
                self.options.insert(target.to_string(), target_value);
                return Ok(());
            }
        }

        // Special options that can't be changed
        let special = ["interactive", "login", "shinstdin", "singlecommand"];
        if special.contains(&actual_name.as_str()) {
            if self.options.get(&actual_name) == Some(&actual_value) {
                return Ok(());
            }
            return Err(format!("can't change option: {}", actual_name));
        }

        self.options.insert(actual_name, actual_value);
        Ok(())
    }

    /// Unset an option (same as set(name, false))
    pub fn unset(&mut self, name: &str) -> Result<(), String> {
        self.set(name, false)
    }

    /// Look up option by single letter
    pub fn lookup_letter(&self, c: char) -> Option<(&'static str, bool)> {
        let letters = if self.is_set("shoptionletters") {
            KSH_LETTERS
        } else {
            ZSH_LETTERS
        };

        for (ch, name, negated) in letters {
            if *ch == c {
                return Some((name, *negated));
            }
        }
        None
    }

    /// Set option by single letter
    pub fn set_by_letter(&mut self, c: char, value: bool) -> Result<(), String> {
        if let Some((name, negated)) = self.lookup_letter(c) {
            let actual_value = if negated { !value } else { value };
            self.set(name, actual_value)
        } else {
            Err(format!("bad option: -{}", c))
        }
    }

    /// Set emulation mode
    pub fn emulate(&mut self, mode: &str, fully: bool) {
        let ch = mode.chars().next().unwrap_or('z');
        let ch = if ch == 'r' {
            mode.chars().nth(1).unwrap_or('z')
        } else {
            ch
        };

        self.emulation = match ch {
            'c' => Emulation::Csh,
            'k' => Emulation::Ksh,
            's' | 'b' => Emulation::Sh,
            _ => Emulation::Zsh,
        };
        self.fully_emulating = fully;

        // Reset options to emulation defaults
        self.install_emulation_defaults();
    }

    /// Install default options for current emulation
    fn install_emulation_defaults(&mut self) {
        // This would set all the emulation-specific defaults
        // For now, just set some key differences
        match self.emulation {
            Emulation::Sh | Emulation::Ksh => {
                self.options.insert("shwordsplit".to_string(), true);
                self.options.insert("globsubst".to_string(), true);
                self.options.insert("ksharrays".to_string(), true);
                self.options.insert("posixbuiltins".to_string(), true);
                self.options.insert("promptpercent".to_string(), false);
                self.options.insert("banghist".to_string(), false);
            }
            Emulation::Csh => {
                self.options.insert("cshjunkiehistory".to_string(), true);
                self.options.insert("cshjunkieloops".to_string(), true);
                self.options.insert("cshnullcmd".to_string(), true);
            }
            Emulation::Zsh => {
                self.set_zsh_defaults();
            }
        }
    }

    /// Get the $- parameter value (active single-letter options)
    pub fn dash_string(&self) -> String {
        let mut result = String::new();
        let letters = if self.is_set("shoptionletters") {
            KSH_LETTERS
        } else {
            ZSH_LETTERS
        };

        for (c, name, negated) in letters {
            let is_set = self.is_set(name);
            if (*negated && !is_set) || (!*negated && is_set) {
                result.push(*c);
            }
        }
        result
    }

    /// List all options and their current state
    pub fn list(&self) -> Vec<(String, bool)> {
        let mut result: Vec<_> = self.options.iter().map(|(k, v)| (k.clone(), *v)).collect();
        result.sort_by(|a, b| a.0.cmp(&b.0));
        result
    }

    /// Get all option names
    pub fn all_names(&self) -> Vec<&str> {
        // Return all known option names
        let mut names: Vec<_> = self.options.keys().map(|s| s.as_str()).collect();
        names.sort();
        names
    }
}

/// Normalize an option name: lowercase, remove underscores
pub fn normalize_option_name(name: &str) -> String {
    name.chars()
        .filter(|&c| c != '_')
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Parse option arguments from setopt/unsetopt
pub fn parse_option_args(
    opts: &mut ShellOptions,
    args: &[&str],
    is_unset: bool,
) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    for arg in args {
        let (name, value) = if let Some(stripped) = arg.strip_prefix("no") {
            (stripped, is_unset) // "nofoo" with unsetopt means set foo
        } else {
            (*arg, !is_unset)
        };

        if let Err(e) = opts.set(name, value) {
            errors.push(e);
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_options() {
        let opts = ShellOptions::new();
        assert!(opts.is_set("glob"));
        assert!(opts.is_set("exec"));
        assert!(opts.is_set("zle"));
        assert!(!opts.is_set("xtrace"));
    }

    #[test]
    fn test_set_option() {
        let mut opts = ShellOptions::new();
        opts.set("xtrace", true).unwrap();
        assert!(opts.is_set("xtrace"));
        opts.set("xtrace", false).unwrap();
        assert!(!opts.is_set("xtrace"));
    }

    #[test]
    fn test_no_prefix() {
        let mut opts = ShellOptions::new();
        opts.set("noglob", true).unwrap();
        assert!(!opts.is_set("glob"));

        assert!(opts.lookup("noglob") == Some(true));
    }

    #[test]
    fn test_case_insensitive() {
        let opts = ShellOptions::new();
        assert_eq!(opts.lookup("GLOB"), opts.lookup("glob"));
        assert_eq!(opts.lookup("GlOb"), opts.lookup("glob"));
    }

    #[test]
    fn test_underscore_ignored() {
        let opts = ShellOptions::new();
        assert_eq!(opts.lookup("auto_list"), opts.lookup("autolist"));
        assert_eq!(opts.lookup("AUTO_LIST"), opts.lookup("autolist"));
    }

    #[test]
    fn test_option_alias() {
        let mut opts = ShellOptions::new();

        // braceexpand is alias for noignorebraces
        opts.set("braceexpand", true).unwrap();
        assert!(!opts.is_set("ignorebraces"));
    }

    #[test]
    fn test_single_letter() {
        let mut opts = ShellOptions::new();

        // -x is xtrace
        opts.set_by_letter('x', true).unwrap();
        assert!(opts.is_set("xtrace"));

        // -n is noexec (negated)
        opts.set_by_letter('n', true).unwrap();
        assert!(!opts.is_set("exec"));
    }

    #[test]
    fn test_emulation() {
        let mut opts = ShellOptions::new();

        opts.emulate("sh", true);
        assert_eq!(opts.emulation, Emulation::Sh);
        assert!(opts.is_set("shwordsplit"));

        opts.emulate("zsh", true);
        assert_eq!(opts.emulation, Emulation::Zsh);
    }

    #[test]
    fn test_dash_string() {
        let mut opts = ShellOptions::new();
        opts.set("interactive", true).unwrap();
        opts.set("monitor", true).unwrap();

        let dash = opts.dash_string();
        assert!(dash.contains('i'));
        assert!(dash.contains('m'));
    }

    #[test]
    fn test_normalize_name() {
        assert_eq!(normalize_option_name("AUTO_LIST"), "autolist");
        assert_eq!(normalize_option_name("AutoList"), "autolist");
        assert_eq!(normalize_option_name("auto__list"), "autolist");
    }
}
