//! Termcap module - port of Modules/termcap.c
//!
//! Provides termcap manipulation through the echotc builtin and termcap hash.

use std::collections::HashMap;

/// Termcap boolean capability codes
pub static BOOL_CODES: &[&str] = &[
    "bw", "am", "ut", "cc", "xs", "YA", "YF", "YB", "xt", "xn", "eo", "gn", "hc", "HC", "km", "YC",
    "hs", "hl", "in", "YG", "da", "db", "mi", "ms", "nx", "xb", "NP", "ND", "NR", "os", "5i", "YD",
    "YE", "es", "hz", "ul", "xo",
];

/// Termcap numeric capability codes
pub static NUM_CODES: &[&str] = &[
    "co", "it", "lh", "lw", "li", "lm", "sg", "ma", "Co", "pa", "MW", "NC", "Nl", "pb", "vt", "ws",
    "Yo", "Yp", "Ya", "BT", "Yc", "Yb", "Yd", "Ye", "Yf", "Yg", "Yh", "Yi", "Yk", "Yj", "Yl", "Ym",
    "Yn",
];

/// Termcap string capability codes
pub static STR_CODES: &[&str] = &[
    "ac", "bt", "bl", "cr", "ZA", "ZB", "ZC", "ZD", "cs", "rP", "ct", "MC", "cl", "cb", "ce", "cd",
    "ch", "CC", "CW", "cm", "do", "ho", "vi", "le", "CM", "ve", "nd", "ll", "up", "vs", "ZE", "dc",
    "dl", "DI", "ds", "DK", "hd", "eA", "as", "SA", "mb", "md", "ti", "dm", "mh", "ZF", "ZG", "im",
    "ZH", "ZI", "ZJ", "ZK", "ZL", "mp", "mr", "mk", "ZM", "so", "ZN", "ZO", "us", "ZP", "SX", "ec",
    "ae", "RA", "me", "te", "ed", "ZQ", "ei", "ZR", "ZS", "ZT", "ZU", "se", "ZV", "ZW", "ue", "ZX",
    "RX", "PA", "fh", "vb", "ff", "fs", "WG", "HU", "i1", "is", "i3", "if", "iP", "Ic", "Ip", "ic",
    "al", "ip", "K1", "K3", "K2", "kb", "kB", "K4", "K5", "ka", "kC", "kt", "kD", "kL", "kd", "kM",
    "kE", "kS", "k0", "k1", "k2", "k3", "k4", "k5", "k6", "k7", "k8", "k9", "kh", "kI", "kA", "kl",
    "kH", "kN", "kP", "kr", "kF", "kR", "kT", "ku", "ke", "ks", "l0", "l1", "l2", "l3", "l4", "l5",
    "l6", "l7", "l8", "l9", "nw", "oc", "op", "pc", "DC", "DL", "DO", "IC", "SF", "AL", "LE", "RI",
    "SR", "UP", "pk", "pl", "px", "pn", "ps", "pO", "pf", "po", "rc", "cv", "sc", "sf", "sr", "sa",
    "st", "ta", "ts", "uc", "hu",
];

/// Termcap capability value
#[derive(Debug, Clone)]
pub enum TermcapValue {
    Boolean(bool),
    Number(i32),
    String(String),
}

/// Termcap interface using basic ANSI escape sequences
#[derive(Debug, Default)]
pub struct Termcap {
    initialized: bool,
    terminal: Option<String>,
    capabilities: HashMap<String, TermcapValue>,
}

impl Termcap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Initialize termcap for the given terminal
    pub fn init(&mut self, term: Option<&str>) -> bool {
        let terminal = term
            .map(|s| s.to_string())
            .or_else(|| std::env::var("TERM").ok());

        if let Some(t) = terminal {
            self.terminal = Some(t.clone());
            self.load_capabilities(&t);
            self.initialized = true;
            return true;
        }

        false
    }

    fn load_capabilities(&mut self, term: &str) {
        let is_xterm =
            term.contains("xterm") || term.contains("256color") || term.contains("screen");
        let is_ansi = is_xterm || term.contains("ansi") || term.contains("vt100");

        self.capabilities
            .insert("am".to_string(), TermcapValue::Boolean(true));
        self.capabilities
            .insert("km".to_string(), TermcapValue::Boolean(true));
        self.capabilities
            .insert("mi".to_string(), TermcapValue::Boolean(true));
        self.capabilities
            .insert("ms".to_string(), TermcapValue::Boolean(true));
        self.capabilities
            .insert("xn".to_string(), TermcapValue::Boolean(true));
        self.capabilities
            .insert("ut".to_string(), TermcapValue::Boolean(is_xterm));

        let cols = std::env::var("COLUMNS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(80);
        let lines = std::env::var("LINES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(24);
        let colors = if term.contains("256") {
            256
        } else if is_xterm {
            8
        } else {
            2
        };

        self.capabilities
            .insert("co".to_string(), TermcapValue::Number(cols));
        self.capabilities
            .insert("li".to_string(), TermcapValue::Number(lines));
        self.capabilities
            .insert("Co".to_string(), TermcapValue::Number(colors));
        self.capabilities
            .insert("it".to_string(), TermcapValue::Number(8));

        if is_ansi {
            self.capabilities.insert(
                "cl".to_string(),
                TermcapValue::String("\x1b[H\x1b[2J".to_string()),
            );
            self.capabilities.insert(
                "cm".to_string(),
                TermcapValue::String("\x1b[%i%d;%dH".to_string()),
            );
            self.capabilities
                .insert("up".to_string(), TermcapValue::String("\x1b[A".to_string()));
            self.capabilities
                .insert("do".to_string(), TermcapValue::String("\x1b[B".to_string()));
            self.capabilities
                .insert("nd".to_string(), TermcapValue::String("\x1b[C".to_string()));
            self.capabilities
                .insert("le".to_string(), TermcapValue::String("\x1b[D".to_string()));
            self.capabilities
                .insert("ho".to_string(), TermcapValue::String("\x1b[H".to_string()));
            self.capabilities
                .insert("ce".to_string(), TermcapValue::String("\x1b[K".to_string()));
            self.capabilities
                .insert("cd".to_string(), TermcapValue::String("\x1b[J".to_string()));
            self.capabilities
                .insert("me".to_string(), TermcapValue::String("\x1b[m".to_string()));
            self.capabilities.insert(
                "md".to_string(),
                TermcapValue::String("\x1b[1m".to_string()),
            );
            self.capabilities.insert(
                "mr".to_string(),
                TermcapValue::String("\x1b[7m".to_string()),
            );
            self.capabilities.insert(
                "us".to_string(),
                TermcapValue::String("\x1b[4m".to_string()),
            );
            self.capabilities.insert(
                "ue".to_string(),
                TermcapValue::String("\x1b[24m".to_string()),
            );
            self.capabilities.insert(
                "so".to_string(),
                TermcapValue::String("\x1b[7m".to_string()),
            );
            self.capabilities.insert(
                "se".to_string(),
                TermcapValue::String("\x1b[27m".to_string()),
            );
            self.capabilities.insert(
                "vi".to_string(),
                TermcapValue::String("\x1b[?25l".to_string()),
            );
            self.capabilities.insert(
                "ve".to_string(),
                TermcapValue::String("\x1b[?25h".to_string()),
            );
            self.capabilities.insert(
                "ti".to_string(),
                TermcapValue::String("\x1b[?1049h".to_string()),
            );
            self.capabilities.insert(
                "te".to_string(),
                TermcapValue::String("\x1b[?1049l".to_string()),
            );
            self.capabilities
                .insert("bl".to_string(), TermcapValue::String("\x07".to_string()));
            self.capabilities
                .insert("cr".to_string(), TermcapValue::String("\r".to_string()));
        }
    }

    /// Get a boolean capability
    pub fn get_flag(&self, name: &str) -> Option<bool> {
        match self.capabilities.get(name)? {
            TermcapValue::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// Get a numeric capability
    pub fn get_num(&self, name: &str) -> Option<i32> {
        match self.capabilities.get(name)? {
            TermcapValue::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// Get a string capability
    pub fn get_str(&self, name: &str) -> Option<String> {
        match self.capabilities.get(name)? {
            TermcapValue::String(s) => Some(s.clone()),
            _ => None,
        }
    }

    /// Get any capability
    pub fn get(&self, name: &str) -> Option<&TermcapValue> {
        self.capabilities.get(name)
    }

    /// Is termcap initialized?
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Get all boolean capabilities
    pub fn booleans(&self) -> HashMap<String, bool> {
        self.capabilities
            .iter()
            .filter_map(|(k, v)| {
                if let TermcapValue::Boolean(b) = v {
                    Some((k.clone(), *b))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get all numeric capabilities
    pub fn numbers(&self) -> HashMap<String, i32> {
        self.capabilities
            .iter()
            .filter_map(|(k, v)| {
                if let TermcapValue::Number(n) = v {
                    Some((k.clone(), *n))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get all string capabilities
    pub fn strings(&self) -> HashMap<String, String> {
        self.capabilities
            .iter()
            .filter_map(|(k, v)| {
                if let TermcapValue::String(s) = v {
                    Some((k.clone(), s.clone()))
                } else {
                    None
                }
            })
            .collect()
    }
}

/// Apply tgoto-style parameter substitution
pub fn tgoto(cap: &str, col: i32, row: i32) -> String {
    let mut result = String::new();
    let mut chars = cap.chars().peekable();
    let mut use_row = true;

    while let Some(c) = chars.next() {
        if c == '%' {
            if let Some(&next) = chars.peek() {
                chars.next();
                match next {
                    'd' => {
                        let val = if use_row { row } else { col };
                        result.push_str(&val.to_string());
                        use_row = false;
                    }
                    '2' => {
                        let val = if use_row { row } else { col };
                        result.push_str(&format!("{:02}", val));
                        use_row = false;
                    }
                    '3' => {
                        let val = if use_row { row } else { col };
                        result.push_str(&format!("{:03}", val));
                        use_row = false;
                    }
                    '.' => {
                        let val = if use_row { row } else { col };
                        result.push((val as u8) as char);
                        use_row = false;
                    }
                    '+' => {
                        if let Some(offset) = chars.next() {
                            let val = if use_row { row } else { col };
                            result.push(((val + offset as i32) as u8) as char);
                            use_row = false;
                        }
                    }
                    'i' => {}
                    '%' => {
                        result.push('%');
                    }
                    _ => {
                        result.push('%');
                        result.push(next);
                    }
                }
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Execute echotc builtin
pub fn builtin_echotc(args: &[&str], tc: &Termcap) -> (i32, String) {
    if args.is_empty() {
        return (1, "echotc: capability name required\n".to_string());
    }

    if !tc.is_initialized() {
        return (1, "echotc: terminal not initialized\n".to_string());
    }

    let cap_name = args[0];

    if let Some(n) = tc.get_num(cap_name) {
        return (0, format!("{}\n", n));
    }

    if let Some(b) = tc.get_flag(cap_name) {
        return (0, format!("{}\n", if b { "yes" } else { "no" }));
    }

    if let Some(s) = tc.get_str(cap_name) {
        if args.len() == 1 {
            return (0, s);
        }

        let mut required_args = 0;
        for c in s.chars() {
            if c == '%' {
                required_args += 1;
            }
        }
        required_args /= 2;

        if args.len() - 1 != required_args {
            if args.len() - 1 < required_args {
                return (1, "echotc: not enough arguments\n".to_string());
            } else {
                return (1, "echotc: too many arguments\n".to_string());
            }
        }

        if required_args >= 2 {
            let row: i32 = args[1].parse().unwrap_or(0);
            let col: i32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(row);
            return (0, tgoto(&s, col, row));
        }

        return (0, s);
    }

    (1, format!("echotc: no such capability: {}\n", cap_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_termcap_new() {
        let tc = Termcap::new();
        assert!(!tc.is_initialized());
    }

    #[test]
    fn test_termcap_init() {
        let mut tc = Termcap::new();
        let result = tc.init(Some("xterm-256color"));
        assert!(result);
        assert!(tc.is_initialized());
    }

    #[test]
    fn test_termcap_get_num() {
        let mut tc = Termcap::new();
        tc.init(Some("xterm"));

        assert!(tc.get_num("co").is_some());
        assert!(tc.get_num("li").is_some());
    }

    #[test]
    fn test_termcap_get_flag() {
        let mut tc = Termcap::new();
        tc.init(Some("xterm"));

        assert_eq!(tc.get_flag("am"), Some(true));
    }

    #[test]
    fn test_termcap_get_str() {
        let mut tc = Termcap::new();
        tc.init(Some("xterm"));

        assert!(tc.get_str("cl").is_some());
        assert!(tc.get_str("cm").is_some());
    }

    #[test]
    fn test_tgoto() {
        let result = tgoto("\x1b[%d;%dH", 10, 5);
        assert!(result.contains("5") && result.contains("10"));
    }

    #[test]
    fn test_builtin_echotc_no_args() {
        let tc = Termcap::new();
        let (status, _) = builtin_echotc(&[], &tc);
        assert_eq!(status, 1);
    }

    #[test]
    fn test_builtin_echotc_not_initialized() {
        let tc = Termcap::new();
        let (status, output) = builtin_echotc(&["co"], &tc);
        assert_eq!(status, 1);
        assert!(output.contains("not initialized"));
    }

    #[test]
    fn test_builtin_echotc_numeric() {
        let mut tc = Termcap::new();
        tc.init(Some("xterm"));
        let (status, output) = builtin_echotc(&["co"], &tc);
        assert_eq!(status, 0);
        assert!(output.contains("80") || output.parse::<i32>().is_ok());
    }

    #[test]
    fn test_builtin_echotc_boolean() {
        let mut tc = Termcap::new();
        tc.init(Some("xterm"));
        let (status, output) = builtin_echotc(&["am"], &tc);
        assert_eq!(status, 0);
        assert!(output.contains("yes") || output.contains("no"));
    }

    #[test]
    fn test_bool_codes() {
        assert!(BOOL_CODES.contains(&"am"));
        assert!(BOOL_CODES.contains(&"bw"));
    }

    #[test]
    fn test_num_codes() {
        assert!(NUM_CODES.contains(&"co"));
        assert!(NUM_CODES.contains(&"li"));
    }

    #[test]
    fn test_str_codes() {
        assert!(STR_CODES.contains(&"cl"));
        assert!(STR_CODES.contains(&"cm"));
    }
}
