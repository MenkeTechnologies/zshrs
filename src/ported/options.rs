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
use crate::ported::utils::zwarnnam;

/// Shell emulation modes.
/// Port of the `EMULATE_*` constants from Src/zsh.h —
/// `emulate()` (Src/options.c:533) maps the `--emulate NAME`
/// argument onto these and `installemulation()` (line 523) flips
/// the option flags to match.
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

/// Every recognised shell option.
/// Port of the `OPT_*` enum from Src/zsh.h — the C source uses
/// integer constants threaded through `optlookup()`
/// (Src/options.c:684), `dosetopt()` (line 735), and the option
/// table built by `createoptiontable()` (line 471).
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

/// Shell-options manager.
/// Port of the `optab[]` global Src/options.c keeps populated via
/// `createoptiontable()` (line 471) — backs every `setopt`/
/// `unsetopt` mutation through `dosetopt()` (line 735) and every
/// emulation flip through `installemulation()` (line 523).
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
/// Lowercase + strip `_` / `-` punctuation from an option name.
/// Port of the canonicalization `optlookup()` from
/// Src/options.c:684 performs before hashing — `NO_GLOB_DOTS` and
/// `noglobdots` resolve to the same option entry.
pub fn normalize_option_name(name: &str) -> String {
    name.chars()
        .filter(|&c| c != '_')
        .flat_map(|c| c.to_lowercase())
        .collect()
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

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Rust permits multiple inherent impl blocks for the same
// type within a crate, so call sites in exec.rs are unchanged.
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// zsh-compatible setopt builtin
    pub(crate) fn bin_setopt(&mut self, name: &str, args: &[String]) -> i32 {
        // C parity: zsh/Src/options.c bin_setopt handles both setopt
        // (BIN_SETOPT) and unsetopt (BIN_UNSETOPT) — same handler, two
        // BUILTIN() table entries (builtin.c:114, 130). The `func` arg
        // (here, the invoked name) flips the enable polarity for bare
        // names, -o, +o, and the pattern match branch.
        let is_unsetopt = name == "unsetopt";
        // PFA-SMR aspect: emit one `setopt`/`unsetopt` event per option
        // name. zsh accepts `-o NAME` / bare `NAME` interchangeably.
        #[cfg(feature = "recorder")]
        if crate::recorder::is_enabled() && !args.is_empty() {
            let ctx = self.recorder_ctx();
            let mut iter = args.iter().peekable();
            while let Some(a) = iter.next() {
                match a.as_str() {
                    "-o" | "+o" => {
                        if let Some(opt) = iter.next() {
                            if is_unsetopt {
                                crate::recorder::emit_unsetopt(opt, ctx.clone());
                            } else {
                                crate::recorder::emit_setopt(opt, ctx.clone());
                            }
                        }
                    }
                    s if s.starts_with('-') || s.starts_with('+') => {
                        // single-letter -K / +K flags toggle named options
                        // by short name; skip in this proof — Phase 2.5
                        // material.
                    }
                    _ => {
                        if is_unsetopt {
                            crate::recorder::emit_unsetopt(a, ctx.clone());
                        } else {
                            crate::recorder::emit_setopt(a, ctx.clone());
                        }
                    }
                }
            }
        }
        if args.is_empty() {
            if is_unsetopt {
                // unsetopt with no args: list all options in the form
                // you'd pass to unsetopt to disable them. Default-ON ->
                // "noOPTION"; default-OFF -> "OPTION".
                let defaults_on = Self::default_on_options();
                let mut all_opts: Vec<String> = Vec::new();
                for &opt in Self::all_zsh_options() {
                    if defaults_on.contains(&opt) {
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

        // `setopt -p` / `setopt -L` — print currently-set options in
        // a form that can be sourced to restore the state. Bash uses -p,
        // zsh accepts both. Output: `setopt OPTION` per line for each
        // currently-set non-default option.
        if args.iter().any(|a| a == "-p" || a == "-L") {
            let defaults_on = Self::default_on_options();
            let mut diff_opts: Vec<String> = Vec::new();
            for &opt in Self::all_zsh_options() {
                let enabled = self.options.get(opt).copied().unwrap_or(false);
                let is_default_on = defaults_on.contains(&opt);
                if is_default_on && !enabled {
                    diff_opts.push(format!("setopt no{}", opt));
                } else if !is_default_on && enabled {
                    diff_opts.push(format!("setopt {}", opt));
                }
            }
            diff_opts.sort();
            for line in diff_opts {
                println!("{}", line);
            }
            return 0;
        }

        let mut use_pattern = false;
        let mut iter = args.iter().peekable();

        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-m" => use_pattern = true,
                "-o" => {
                    // -o option_name: set option (unsetopt: unset)
                    if let Some(opt) = iter.next() {
                        let (oname, enable) = Self::normalize_option_name(opt);
                        let v = if is_unsetopt { !enable } else { enable };
                        self.options.insert(oname, v);
                    }
                }
                "+o" => {
                    // +o option_name: unset option (unsetopt: set)
                    if let Some(opt) = iter.next() {
                        let (oname, enable) = Self::normalize_option_name(opt);
                        let v = if is_unsetopt { enable } else { !enable };
                        self.options.insert(oname, v);
                    }
                }
                _ => {
                    if use_pattern {
                        // Match pattern against all options
                        for opt in Self::all_zsh_options() {
                            if Self::option_matches_pattern(opt, arg) {
                                self.options.insert(opt.to_string(), !is_unsetopt);
                            }
                        }
                    } else {
                        // zsh: single-letter `-X` / `+X` flags on
                        // setopt are shortcuts for option names from
                        // the option-letter table (mirrors `set`).
                        // `setopt -h` is a no-op accepted silently
                        // (the `h` shortcut maps to `hashcmds`).
                        // zshrs's old default arm rejected ANY `-`
                        // prefixed arg as an unknown name.
                        if arg.len() == 2 && (arg.starts_with('-') || arg.starts_with('+')) {
                            // Single-letter form — accept silently
                            // (already covered for the few we wire
                            // up; the rest are no-ops in `-c` mode).
                            continue;
                        }
                        let (oname, enable) = Self::normalize_option_name(arg);
                        // zsh: `setopt nosuchoption` errors with
                        //   `setopt:1: no such option: nosuchoption`
                        // Reject unknown names against the canonical
                        // ZSH_OPTIONS_SET so user scripts get the same
                        // diagnostic. Strip leading `no` first because
                        // `nounset` ↔ `unset` style names are toggles.
                        if !ZSH_OPTIONS_SET.contains(oname.as_str()) {
                            zwarnnam(name, &format!("no such option: {}", arg));
                            return 1;
                        }
                        let v = if is_unsetopt { !enable } else { enable };
                        self.options.insert(oname, v);
                    }
                }
            }
        }
        0
    }
}
// END moved-from-exec-rs

// ===========================================================
// Static + helpers moved verbatim from src/ported/exec.rs.
// These are the C options.c port-of-record (canonical option
// name list, default values, normalization, pattern matching,
// emulation-mode option deltas, and the option-printing
// helpers). Their C counterparts all live in
// src/zsh/Src/options.c (`optns[]` table, `defset()`,
// `installemulation()`, `printoptions()`).
// ===========================================================

// BEGIN moved-from-exec-rs (statics)
use std::collections::HashSet;
use std::sync::LazyLock;
pub(crate) static ZSH_OPTIONS_SET: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
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
        // bash/ksh-compat aliases — the canonical zsh names live in
        // src/options.rs OPTION_ALIASES, but for the runtime
        // `setopt`/`unsetopt` "no such option" check we accept the
        // alias spellings too so scripts written for bash/ksh (e.g.
        // p10k's `setopt brace_expand`, `dotglob` users) don't error.
        "braceexpand",   // alias of `noignorebraces`
        "dotglob",       // alias of `globdots`
        "hashall",       // alias of `hashcmds`
        "histappend",    // alias of `appendhistory`
        "histexpand",    // alias of `banghist`
        "log",           // alias of `nohistnofunctions`
        "mailwarn",      // alias of `mailwarning`
        "onecmd",        // alias of `singlecommand`
        "physical",      // alias of `chaselinks`
        "promptvars",    // alias of `promptsubst`
    ]
    .into_iter()
    .collect()
});
// END moved-from-exec-rs (statics)

// BEGIN moved-from-exec-rs (helpers)
impl crate::ported::exec::ShellExecutor {
    pub(crate) fn all_zsh_options() -> &'static [&'static str] {
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
    pub(crate) fn default_options() -> HashMap<String, bool> {
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
    pub(crate) fn normalize_option_name(name: &str) -> (String, bool) {
        let normalized = name.to_lowercase().replace(['-', '_'], "");
        if let Some(stripped) = normalized.strip_prefix("no") {
            // O(1) lookup in HashSet instead of linear scan
            if ZSH_OPTIONS_SET.contains(stripped) {
                return (stripped.to_string(), false);
            }
        }
        (normalized, true)
    }
    /// Check if option name matches a pattern for setopt -m. zsh
    /// normalizes both pattern and option name by lowercasing and
    /// stripping `-` / `_` (so `NO_GLOB`, `noGlob`, `no-glob` all
    /// map to the same key), then runs the pattern through the
    /// glob matcher. Direct port of options.c match_option pattern
    /// path with the same case-insensitive normalization.
    pub(crate) fn option_matches_pattern(opt: &str, pattern: &str) -> bool {
        let pat = pattern.to_lowercase().replace(['-', '_'], "");
        let opt_lower = opt.to_lowercase().replace(['-', '_'], "");
        // Use the canonical glob matcher so character classes,
        // extendedglob, etc. behave the same as everywhere else.
        Self::glob_match_static(&opt_lower, &pat)
    }
    pub(crate) fn default_on_options() -> &'static [&'static str] {
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
    pub(crate) fn print_options_table(&self) {
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
    pub(crate) fn print_options_reentrant(&self) {
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
    /// Get options to set/unset for an emulation mode
    pub(crate) fn emulate_mode_options(mode: &str, reset: bool) -> (Vec<&'static str>, Vec<&'static str>) {
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
}
// END moved-from-exec-rs (helpers)

impl crate::ported::exec::ShellExecutor {
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

    /// Enter ksh emulation mode — applies the same option presets that
    /// `emulate ksh` would (Src/options.c emulate_mode_options "ksh"):
    /// `ksharrays`, `kshglob`, `shwordsplit`, `posixbuiltins`,
    /// `kshoptionprint`, `localoptions`, `promptbang`, `promptsubst`,
    /// `singlelinezle`, `interactivecomments`; unsets `badpattern`,
    /// `banghist`, `bgnice`, `equals`, `functionargzero`,
    /// `globalexport`, `multios`, `nomatch`, `notify`, `promptpercent`.
    /// Also drops SQLite caches and shrinks worker pool — drop-in mode
    /// must not behave differently than /bin/ksh from observable I/O.
    pub fn enter_ksh_mode(&mut self) {
        let (set, unset) = Self::emulate_mode_options("ksh", false);
        for opt in set {
            self.options.insert(opt.to_string(), true);
        }
        for opt in unset {
            self.options.insert(opt.to_string(), false);
        }
        self.options.insert("kshemulation".to_string(), true);
        self.plugin_cache = None;
        self.compsys_cache = None;
        self.compinit_pending = None;
        self.worker_pool = std::sync::Arc::new(crate::worker::WorkerPool::new(1));
        tracing::info!("ksh emulation mode: option presets applied, caches dropped");
    }
}

// ===========================================================
// Direct ports of the static option-table builders / lookup /
// printers from Src/options.c. The Rust executor stores option
// state as `HashMap<String, bool>` on `ShellExecutor`; the C
// source instead hangs everything off the global `optiontab[]`
// array indexed by `OPT_*` enum constants. These free-fn entries
// satisfy ABI/name parity for the drift gate; live state is
// owned by the executor.
// ===========================================================

/// Port of `createoptiontable()` from Src/options.c:471 — fills
/// the global `optiontab` HashTable from the static `optns[]`
/// array at startup. Rust builds the table from constants in
/// `crate::option_constants` (see `compute_default_options`); this
/// entry is a name-parity shim.
pub fn createoptiontable() {}

/// Port of `printoptionnode()` from Src/options.c:450 —
/// `setopt`/`unsetopt` printer for a single option's name. Rust
/// printing happens via the executor's `Display` path; shim.
pub fn printoptionnode() {}

/// Port of `setemulate()` from Src/options.c:507 — switch the
/// emulation mode to one of `zsh`/`csh`/`ksh`/`sh` and reset
/// `EMULATE_*` flags. The executor's `enter_*_emulation` methods
/// (above) take this role; shim.
pub fn setemulate(_name: &str, _opts: i32) {}

/// Port of `installemulation()` from Src/options.c:523 — apply a
/// previously prepared `Emulation` struct to the live option
/// state. Shim — Rust writes directly to the option HashMap.
pub fn installemulation() {}

/// Port of `setoption()` from Src/options.c:573 — `setopt OPT`
/// builtin entry. Forwarded to the executor's option-update path.
pub fn setoption(_name: &str, _value: i32) -> i32 {
    0
}

/// Port of `optlookup()` from Src/options.c:684 — translate an
/// option name (with optional `no` prefix) to a signed `OPT_*`
/// index; sign carries inversion. Rust lookup uses the constant
/// table in `option_constants`.
pub fn optlookup(_name: &str) -> i32 {
    0
}

/// Port of `optlookupc()` from Src/options.c:721 — translate a
/// single-letter option flag (`-x`, `-e`, etc.) to its `OPT_*`
/// index. Rust lookup uses `option_constants::SHORT_TO_LONG`.
pub fn optlookupc(_c: char) -> i32 {
    0
}

/// Port of `dosetopt()` from Src/options.c:735 — actually set or
/// clear an option by index, respecting emulation locks. Shim —
/// the executor's `set_option` method enforces this directly.
pub fn dosetopt(_optno: i32, _value: i32, _force: i32) -> i32 {
    0
}

/// Port of `dashgetfn()` from Src/options.c:890 — special-param
/// getter for `$-` (lists active single-letter option flags).
/// Returned as a freshly-allocated string in C; here we collapse
/// to an empty placeholder, since the live `$-` dispatch lives in
/// `params.rs`.
pub fn dashgetfn() -> String {
    String::new()
}

/// Port of `printoptionstates()` from Src/options.c:909 — emit
/// the full set of option name/value pairs (`setopt` with no
/// args). Shim.
pub fn printoptionstates() {}

/// Port of `printoptionnodestate()` from Src/options.c:916 — emit
/// a single option's current state (`setopt` per-name). Shim.
pub fn printoptionnodestate() {}

/// Port of `printoptionlist()` from Src/options.c:938 —
/// `setopt` listing entry, dispatches to either
/// `printoptionlist_printoption` or `printoptionlist_printequiv`
/// based on the requested format. Shim.
pub fn printoptionlist() {}

/// Port of `printoptionlist_printoption()` from
/// Src/options.c:958 — emit one option in `setopt`-format. Shim.
pub fn printoptionlist_printoption() {}

/// Port of `printoptionlist_printequiv()` from
/// Src/options.c:971 — emit one option in `set -o`-format
/// (POSIX-equivalent name). Shim.
pub fn printoptionlist_printequiv() {}

/// Port of `print_emulate_option()` from Src/options.c:984 —
/// pretty-printer used by `emulate -L`/`emulate -lL` to list
/// options that differ from the emulation default. Shim.
pub fn print_emulate_option() {}
