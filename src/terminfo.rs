//! Terminfo module - port of Modules/terminfo.c
//!
//! Provides access to terminal capabilities via terminfo database.

use std::collections::HashMap;

/// Terminfo capability types
#[derive(Debug, Clone)]
pub enum TermCapability {
    Boolean(bool),
    Number(i32),
    String(String),
}

/// Terminfo interface - using environment and basic capabilities
#[derive(Debug, Default)]
pub struct Terminfo {
    initialized: bool,
    terminal: Option<String>,
    capabilities: HashMap<String, TermCapability>,
}

impl Terminfo {
    pub fn new() -> Self {
        Self::default()
    }

    /// Initialize terminfo for the given terminal
    pub fn init(&mut self, term: Option<&str>) -> bool {
        let terminal = term
            .map(|s| s.to_string())
            .or_else(|| std::env::var("TERM").ok());

        if let Some(t) = terminal {
            self.terminal = Some(t.clone());
            self.load_basic_capabilities(&t);
            self.initialized = true;
            return true;
        }

        false
    }

    fn load_basic_capabilities(&mut self, term: &str) {
        let is_xterm = term.contains("xterm") || term.contains("256color");
        let _is_vt100 = term.contains("vt100") || term.contains("vt220");

        self.capabilities
            .insert("am".to_string(), TermCapability::Boolean(true));
        self.capabilities
            .insert("bce".to_string(), TermCapability::Boolean(is_xterm));
        self.capabilities
            .insert("km".to_string(), TermCapability::Boolean(true));
        self.capabilities
            .insert("mir".to_string(), TermCapability::Boolean(true));
        self.capabilities
            .insert("msgr".to_string(), TermCapability::Boolean(true));
        self.capabilities
            .insert("xenl".to_string(), TermCapability::Boolean(true));

        let cols = std::env::var("COLUMNS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(80);
        let lines = std::env::var("LINES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(24);
        let colors = if is_xterm && term.contains("256") {
            256
        } else if is_xterm {
            8
        } else {
            2
        };

        self.capabilities
            .insert("cols".to_string(), TermCapability::Number(cols));
        self.capabilities
            .insert("lines".to_string(), TermCapability::Number(lines));
        self.capabilities
            .insert("colors".to_string(), TermCapability::Number(colors));
        self.capabilities
            .insert("it".to_string(), TermCapability::Number(8));

        self.capabilities.insert(
            "clear".to_string(),
            TermCapability::String("\x1b[H\x1b[2J".to_string()),
        );
        self.capabilities.insert(
            "cup".to_string(),
            TermCapability::String("\x1b[%i%p1%d;%p2%dH".to_string()),
        );
        self.capabilities.insert(
            "cuu1".to_string(),
            TermCapability::String("\x1b[A".to_string()),
        );
        self.capabilities.insert(
            "cud1".to_string(),
            TermCapability::String("\x1b[B".to_string()),
        );
        self.capabilities.insert(
            "cuf1".to_string(),
            TermCapability::String("\x1b[C".to_string()),
        );
        self.capabilities.insert(
            "cub1".to_string(),
            TermCapability::String("\x1b[D".to_string()),
        );
        self.capabilities.insert(
            "home".to_string(),
            TermCapability::String("\x1b[H".to_string()),
        );
        self.capabilities.insert(
            "el".to_string(),
            TermCapability::String("\x1b[K".to_string()),
        );
        self.capabilities.insert(
            "ed".to_string(),
            TermCapability::String("\x1b[J".to_string()),
        );
        self.capabilities.insert(
            "sgr0".to_string(),
            TermCapability::String("\x1b[m".to_string()),
        );
        self.capabilities.insert(
            "bold".to_string(),
            TermCapability::String("\x1b[1m".to_string()),
        );
        self.capabilities.insert(
            "rev".to_string(),
            TermCapability::String("\x1b[7m".to_string()),
        );
        self.capabilities.insert(
            "smul".to_string(),
            TermCapability::String("\x1b[4m".to_string()),
        );
        self.capabilities.insert(
            "rmul".to_string(),
            TermCapability::String("\x1b[24m".to_string()),
        );
        self.capabilities.insert(
            "smso".to_string(),
            TermCapability::String("\x1b[7m".to_string()),
        );
        self.capabilities.insert(
            "rmso".to_string(),
            TermCapability::String("\x1b[27m".to_string()),
        );

        if is_xterm {
            self.capabilities.insert(
                "setaf".to_string(),
                TermCapability::String("\x1b[3%p1%dm".to_string()),
            );
            self.capabilities.insert(
                "setab".to_string(),
                TermCapability::String("\x1b[4%p1%dm".to_string()),
            );
        }
    }

    /// Get a boolean capability
    pub fn get_flag(&self, name: &str) -> Option<bool> {
        if !self.initialized {
            return None;
        }

        match self.capabilities.get(name)? {
            TermCapability::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// Get a numeric capability
    pub fn get_num(&self, name: &str) -> Option<i32> {
        if !self.initialized {
            return None;
        }

        match self.capabilities.get(name)? {
            TermCapability::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// Get a string capability
    pub fn get_str(&self, name: &str) -> Option<String> {
        if !self.initialized {
            return None;
        }

        match self.capabilities.get(name)? {
            TermCapability::String(s) => Some(s.clone()),
            _ => None,
        }
    }

    /// Get any capability (auto-detect type)
    pub fn get(&self, name: &str) -> Option<TermCapability> {
        if let Some(n) = self.get_num(name) {
            return Some(TermCapability::Number(n));
        }
        if let Some(b) = self.get_flag(name) {
            return Some(TermCapability::Boolean(b));
        }
        if let Some(s) = self.get_str(name) {
            return Some(TermCapability::String(s));
        }
        None
    }

    /// Get all boolean capabilities
    pub fn booleans(&self) -> HashMap<String, bool> {
        let mut result = HashMap::new();
        for name in BOOL_NAMES.iter() {
            if let Some(val) = self.get_flag(name) {
                result.insert(name.to_string(), val);
            }
        }
        result
    }

    /// Get all numeric capabilities
    pub fn numbers(&self) -> HashMap<String, i32> {
        let mut result = HashMap::new();
        for name in NUM_NAMES.iter() {
            if let Some(val) = self.get_num(name) {
                result.insert(name.to_string(), val);
            }
        }
        result
    }

    /// Get all string capabilities
    pub fn strings(&self) -> HashMap<String, String> {
        let mut result = HashMap::new();
        for name in STR_NAMES.iter() {
            if let Some(val) = self.get_str(name) {
                result.insert(name.to_string(), val);
            }
        }
        result
    }

    /// Is terminfo initialized?
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Get current terminal name
    pub fn terminal(&self) -> Option<&str> {
        self.terminal.as_deref()
    }
}

/// Boolean capability names
pub static BOOL_NAMES: &[&str] = &[
    "bw", "am", "bce", "ccc", "xhp", "xhpa", "cpix", "crxm", "xt", "xenl", "eo", "gn", "hc",
    "chts", "km", "daisy", "hs", "hls", "in", "lpix", "da", "db", "mir", "msgr", "nxon", "xsb",
    "npc", "ndscr", "nrrmc", "os", "mc5i", "xvpa", "sam", "eslok", "hz", "ul", "xon",
];

/// Numeric capability names
pub static NUM_NAMES: &[&str] = &[
    "cols", "it", "lh", "lw", "lines", "lm", "xmc", "ma", "colors", "pairs", "wnum", "ncv", "nlab",
    "pb", "vt", "wsl", "bitwin", "bitype", "bufsz", "btns", "spinh", "spinv", "maddr", "mjump",
    "mcs", "mls", "npins", "orc", "orhi", "orl", "orvi", "cps", "widcs",
];

/// String capability names
pub static STR_NAMES: &[&str] = &[
    "acsc", "cbt", "bel", "cr", "cpi", "lpi", "chr", "cvr", "csr", "rmp", "tbc", "mgc", "clear",
    "el1", "el", "ed", "hpa", "cmdch", "cwin", "cup", "cud1", "home", "civis", "cub1", "mrcup",
    "cnorm", "cuf1", "ll", "cuu1", "cvvis", "defc", "dch1", "dl1", "dial", "dsl", "dclk", "hd",
    "enacs", "smacs", "smam", "blink", "bold", "smcup", "smdc", "dim", "swidm", "sdrfq", "smir",
    "sitm", "slm", "smicm", "snlq", "snrmq", "prot", "rev", "invis", "sshm", "smso", "ssubm",
    "ssupm", "smul", "sum", "smxon", "ech", "rmacs", "rmam", "sgr0", "rmcup", "rmdc", "rwidm",
    "rmir", "ritm", "rlm", "rmicm", "rshm", "rmso", "rsubm", "rsupm", "rmul", "rum", "rmxon",
    "pause", "hook", "flash", "ff", "fsl", "wingo", "hup", "is1", "is2", "is3", "if", "iprog",
    "initc", "initp", "ich1", "il1", "ip", "ka1", "ka3", "kb2", "kbs", "kbeg", "kcbt", "kc1",
    "kc3", "kcan", "ktbc", "kclr", "kclo", "kcmd", "kcpy", "kcrt", "kctab", "kdch1", "kdl1",
    "kcud1", "krmir", "kend", "kent", "kel", "ked", "kext", "kf0", "kf1", "kf10", "kf11", "kf12",
    "kf13", "kf14", "kf15", "kf16", "kf17", "kf18", "kf19", "kf2", "kf20", "kf21", "kf22", "kf23",
    "kf24", "kf25", "kf26", "kf27", "kf28", "kf29", "kf3", "kf30", "kf31", "kf32", "kf33", "kf34",
    "kf35", "kf36", "kf37", "kf38", "kf39", "kf4", "kf40", "kf41", "kf42", "kf43", "kf44", "kf45",
    "kf46", "kf47", "kf48", "kf49", "kf5", "kf50", "kf51", "kf52", "kf53", "kf54", "kf55", "kf56",
    "kf57", "kf58", "kf59", "kf6", "kf60", "kf61", "kf62", "kf63", "kf7", "kf8", "kf9", "kfnd",
    "khlp", "khome", "kich1", "kil1", "kcub1", "kll", "kmrk", "kmsg", "kmov", "knxt", "knp",
    "kopn", "kopt", "kpp", "kprv", "kprt", "krdo", "kref", "krfr", "krpl", "krst", "kres", "kcuf1",
    "ksav", "kBEG", "kCAN", "kCMD", "kCPY", "kCRT", "kDC", "kDL", "kslt", "kEND", "kEOL", "kEXT",
    "kind", "kFND", "kHLP", "kHOM", "kIC", "kLFT", "kMSG", "kMOV", "kNXT", "kOPT", "kPRV", "kPRT",
    "kri", "kRDO", "kRPL", "kRIT", "kRES", "kSAV", "kSPD", "khts", "kUND", "kspd", "kund", "kcuu1",
    "rmkx", "smkx", "lf0", "lf1", "lf10", "lf2", "lf3", "lf4", "lf5", "lf6", "lf7", "lf8", "lf9",
    "fln", "rmln", "smln", "rmm", "smm", "mhpa", "mcud1", "mcub1", "mcuf1", "mvpa", "mcuu1", "nel",
    "porder", "oc", "op", "pad", "dch", "dl", "cud", "mcud", "ich", "indn", "il", "cub", "mcub",
    "cuf", "mcuf", "rin", "cuu", "mcuu", "pfkey", "pfloc", "pfx", "pln", "mc0", "mc5p", "mc4",
    "mc5", "pulse", "qdial", "rmclk", "rep", "rfi", "rs1", "rs2", "rs3", "rf", "rc", "vpa", "sc",
    "ind", "ri", "scs", "sgr", "setb", "smgb", "smgbp", "sclk", "scp", "setf", "smgl", "smglp",
    "smgr", "smgrp", "hts", "smgt", "smgtp", "wind", "sbim", "scsd", "rbim", "rcsd", "subcs",
    "supcs", "ht", "docr", "tsl", "tone", "uc", "hu", "u0", "u1", "u2", "u3", "u4", "u5", "u6",
    "u7", "u8", "u9", "wait", "xoffc", "xonc", "zerom", "scesa", "bicr", "binel", "birep", "csnm",
    "csin", "colornm", "defbi", "devt", "dispc", "endbi", "smpch", "smsc", "rmpch", "rmsc", "getm",
    "kmous", "minfo", "pctrm", "pfxl", "reqmp", "scesc", "s0ds", "s1ds", "s2ds", "s3ds", "setab",
    "setaf", "setcolor", "smglr", "slines", "smgtb", "ehhlm", "elhlm", "elohlm", "erhlm", "ethlm",
    "evhlm", "sgr1", "slength",
];

/// Execute echoti builtin
pub fn builtin_echoti(args: &[&str]) -> (i32, String) {
    if args.is_empty() {
        return (1, "echoti: capability name required\n".to_string());
    }

    let cap_name = args[0];
    let mut ti = Terminfo::new();

    if !ti.init(None) {
        return (1, "echoti: terminal not initialized\n".to_string());
    }

    if let Some(n) = ti.get_num(cap_name) {
        return (0, format!("{}\n", n));
    }

    if let Some(b) = ti.get_flag(cap_name) {
        return (0, format!("{}\n", if b { "yes" } else { "no" }));
    }

    if let Some(s) = ti.get_str(cap_name) {
        if args.len() == 1 {
            return (0, s);
        }
    }

    (
        1,
        format!("echoti: no such terminfo capability: {}\n", cap_name),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terminfo_new() {
        let ti = Terminfo::new();
        assert!(!ti.is_initialized());
    }

    #[test]
    fn test_term_capability_types() {
        let b = TermCapability::Boolean(true);
        let n = TermCapability::Number(80);
        let s = TermCapability::String("test".to_string());

        matches!(b, TermCapability::Boolean(true));
        matches!(n, TermCapability::Number(80));
        matches!(s, TermCapability::String(_));
    }

    #[test]
    fn test_bool_names() {
        assert!(BOOL_NAMES.contains(&"am"));
        assert!(BOOL_NAMES.contains(&"bw"));
    }

    #[test]
    fn test_num_names() {
        assert!(NUM_NAMES.contains(&"cols"));
        assert!(NUM_NAMES.contains(&"lines"));
        assert!(NUM_NAMES.contains(&"colors"));
    }

    #[test]
    fn test_str_names() {
        assert!(STR_NAMES.contains(&"clear"));
        assert!(STR_NAMES.contains(&"cup"));
        assert!(STR_NAMES.contains(&"sgr0"));
    }

    #[test]
    fn test_builtin_echoti_no_args() {
        let (status, _) = builtin_echoti(&[]);
        assert_eq!(status, 1);
    }
}
