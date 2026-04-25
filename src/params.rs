//! Parameter management for zshrs
//!
//! Port from zsh/Src/params.c (6511 lines → full Rust port)
//!
//! Provides shell parameters (variables), special parameters, arrays,
//! associative arrays, parameter attributes, namerefs, scoping,
//! tied parameters, and all special parameter get/set functions.

use std::collections::{HashMap, HashSet};
use std::env;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Parameter flags (from zsh.h PM_* flags)
// ---------------------------------------------------------------------------

pub mod flags {
    pub const SCALAR: u32 = 1 << 0;
    pub const INTEGER: u32 = 1 << 1;
    pub const EFLOAT: u32 = 1 << 2; // %e float format
    pub const FFLOAT: u32 = 1 << 3; // %f float format
    pub const ARRAY: u32 = 1 << 4;
    pub const HASHED: u32 = 1 << 5; // Associative array (PM_HASHED)
    pub const READONLY: u32 = 1 << 6;
    pub const SPECIAL: u32 = 1 << 7;
    pub const LOCAL: u32 = 1 << 8;
    pub const EXPORT: u32 = 1 << 9; // Exported to environment
    pub const UNSET: u32 = 1 << 10;
    pub const TIED: u32 = 1 << 11;
    pub const UNIQUE: u32 = 1 << 12; // Array elements unique
    pub const LOWER: u32 = 1 << 13; // Lowercase value
    pub const UPPER: u32 = 1 << 14; // Uppercase value
    pub const TAG: u32 = 1 << 15; // Tagged parameter
    pub const HIDE: u32 = 1 << 16;
    pub const HIDEVAL: u32 = 1 << 17;
    pub const NORESTORE: u32 = 1 << 18;
    pub const NAMEREF: u32 = 1 << 19; // Named reference
    pub const LEFT: u32 = 1 << 20; // Left justified
    pub const RIGHT_B: u32 = 1 << 21; // Right justified with blanks
    pub const RIGHT_Z: u32 = 1 << 22; // Right justified with zeros
    pub const AUTOLOAD: u32 = 1 << 23; // Autoloaded parameter
    pub const DECLARED: u32 = 1 << 24; // Explicitly declared
    pub const REMOVABLE: u32 = 1 << 25; // Can be removed from table
    pub const HASHELEM: u32 = 1 << 26; // Element of hash
    pub const NAMEDDIR: u32 = 1 << 27; // Named directory
    pub const DONTIMPORT: u32 = 1 << 28;
    pub const DEFAULTED: u32 = 1 << 29;
    pub const DONTIMPORT_SUID: u32 = 1 << 30;

    // Convenience combo - like PM_READONLY_SPECIAL in C
    pub const READONLY_SPECIAL: u32 = READONLY | SPECIAL;

    // Type mask
    pub const TYPE_MASK: u32 = SCALAR | INTEGER | EFLOAT | FFLOAT | ARRAY | HASHED | NAMEREF;

    /// Extract just the type bits
    pub fn pm_type(flags: u32) -> u32 {
        flags & TYPE_MASK
    }

    /// For backwards compat with old code using FLOAT
    pub const FLOAT: u32 = FFLOAT;
    /// For backwards compat with old code using ASSOC
    pub const ASSOC: u32 = HASHED;
}

// ---------------------------------------------------------------------------
// Subscription flags (SCANPM_*)
// ---------------------------------------------------------------------------

pub mod scan_flags {
    pub const WANTVALS: u32 = 1 << 0;
    pub const WANTKEYS: u32 = 1 << 1;
    pub const WANTINDEX: u32 = 1 << 2;
    pub const MATCHKEY: u32 = 1 << 3;
    pub const MATCHVAL: u32 = 1 << 4;
    pub const MATCHMANY: u32 = 1 << 5;
    pub const KEYMATCH: u32 = 1 << 6;
    pub const ARRONLY: u32 = 1 << 7;
    pub const ISVAR_AT: u32 = 1 << 8;
    pub const DQUOTED: u32 = 1 << 9;
    pub const NOEXEC: u32 = 1 << 10;
    pub const CHECKING: u32 = 1 << 11;
    pub const ASSIGNING: u32 = 1 << 12;
    pub const NONAMEREF: u32 = 1 << 13;
    pub const NONAMESPC: u32 = 1 << 14;
}

// ---------------------------------------------------------------------------
// Assignment flags (ASSPM_*)
// ---------------------------------------------------------------------------

pub mod assign_flags {
    pub const AUGMENT: u32 = 1 << 0; // += assignment
    pub const WARN: u32 = 1 << 1; // Warn about global creation
    pub const ENV_IMPORT: u32 = 1 << 2; // Importing from environment
    pub const KEY_VALUE: u32 = 1 << 3; // key=value assignment syntax
}

// ---------------------------------------------------------------------------
// Value flags (VALFLAG_*)
// ---------------------------------------------------------------------------

pub mod val_flags {
    pub const INV: u32 = 1 << 0; // Inverse subscript
    pub const EMPTY: u32 = 1 << 1; // Empty subscript range
    pub const SUBST: u32 = 1 << 2; // Apply formatting
    pub const REFSLICE: u32 = 1 << 3; // Nameref with subscript
}

// ---------------------------------------------------------------------------
// Print flags (PRINT_*)
// ---------------------------------------------------------------------------

pub mod print_flags {
    pub const TYPE: u32 = 1 << 0;
    pub const TYPESET: u32 = 1 << 1;
    pub const NAMEONLY: u32 = 1 << 2;
    pub const KV_PAIR: u32 = 1 << 3;
    pub const LINE: u32 = 1 << 4;
    pub const INCLUDEVALUE: u32 = 1 << 5;
    pub const POSIX_READONLY: u32 = 1 << 6;
    pub const POSIX_EXPORT: u32 = 1 << 7;
    pub const WITH_NAMESPACE: u32 = 1 << 8;
}

// ---------------------------------------------------------------------------
// Parameter value types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum ParamValue {
    Scalar(String),
    Integer(i64),
    Float(f64),
    Array(Vec<String>),
    Assoc(HashMap<String, String>),
    Unset,
}

impl Default for ParamValue {
    fn default() -> Self {
        ParamValue::Unset
    }
}

impl ParamValue {
    pub fn as_string(&self) -> String {
        match self {
            ParamValue::Scalar(s) => s.clone(),
            ParamValue::Integer(i) => i.to_string(),
            ParamValue::Float(f) => format_float(*f, 0, 0),
            ParamValue::Array(a) => a.join(" "),
            ParamValue::Assoc(h) => {
                let mut vals: Vec<&String> = h.values().collect();
                vals.sort();
                vals.into_iter().cloned().collect::<Vec<_>>().join(" ")
            }
            ParamValue::Unset => String::new(),
        }
    }

    pub fn as_integer(&self) -> i64 {
        match self {
            ParamValue::Scalar(s) => s.parse().unwrap_or(0),
            ParamValue::Integer(i) => *i,
            ParamValue::Float(f) => *f as i64,
            ParamValue::Array(a) => a.len() as i64,
            ParamValue::Assoc(h) => h.len() as i64,
            ParamValue::Unset => 0,
        }
    }

    pub fn as_float(&self) -> f64 {
        match self {
            ParamValue::Scalar(s) => s.parse().unwrap_or(0.0),
            ParamValue::Integer(i) => *i as f64,
            ParamValue::Float(f) => *f,
            ParamValue::Array(a) => a.len() as f64,
            ParamValue::Assoc(h) => h.len() as f64,
            ParamValue::Unset => 0.0,
        }
    }

    pub fn as_array(&self) -> Vec<String> {
        match self {
            ParamValue::Scalar(s) => {
                if s.is_empty() {
                    Vec::new()
                } else {
                    vec![s.clone()]
                }
            }
            ParamValue::Integer(i) => vec![i.to_string()],
            ParamValue::Float(f) => vec![format_float(*f, 0, 0)],
            ParamValue::Array(a) => a.clone(),
            ParamValue::Assoc(h) => h.values().cloned().collect(),
            ParamValue::Unset => Vec::new(),
        }
    }

    pub fn is_set(&self) -> bool {
        !matches!(self, ParamValue::Unset)
    }

    /// Get the type flag for this value
    pub fn type_flag(&self) -> u32 {
        match self {
            ParamValue::Scalar(_) => flags::SCALAR,
            ParamValue::Integer(_) => flags::INTEGER,
            ParamValue::Float(_) => flags::FFLOAT,
            ParamValue::Array(_) => flags::ARRAY,
            ParamValue::Assoc(_) => flags::HASHED,
            ParamValue::Unset => flags::SCALAR,
        }
    }
}

// ---------------------------------------------------------------------------
// Numeric type for parameters (from params.c mnumber)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum MNumber {
    Integer(i64),
    Float(f64),
}

impl Default for MNumber {
    fn default() -> Self {
        MNumber::Integer(0)
    }
}

impl MNumber {
    pub fn as_integer(&self) -> i64 {
        match self {
            MNumber::Integer(i) => *i,
            MNumber::Float(f) => *f as i64,
        }
    }

    pub fn as_float(&self) -> f64 {
        match self {
            MNumber::Integer(i) => *i as f64,
            MNumber::Float(f) => *f,
        }
    }

    pub fn is_float(&self) -> bool {
        matches!(self, MNumber::Float(_))
    }
}

// ---------------------------------------------------------------------------
// Value struct - mirrors C's struct value for subscript access
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Value {
    pub pm_name: String,
    pub start: i64,
    pub end: i64,
    pub scan_flags: u32,
    pub val_flags: u32,
}

impl Value {
    pub fn new(name: &str) -> Self {
        Value {
            pm_name: name.to_string(),
            start: 0,
            end: -1,
            scan_flags: 0,
            val_flags: 0,
        }
    }

    pub fn is_all(&self) -> bool {
        self.start == 0 && self.end == -1
    }
}

// ---------------------------------------------------------------------------
// Shell parameter
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    pub value: ParamValue,
    pub flags: u32,
    pub base: i32,               // Output base for integers
    pub width: i32,              // Output field width
    pub level: i32,              // Scope level
    pub ename: Option<String>,   // Environment/tied name
    pub old: Option<Box<Param>>, // Previous parameter at higher scope
}

impl Param {
    pub fn new_scalar(name: &str, value: &str) -> Self {
        Param {
            name: name.to_string(),
            value: ParamValue::Scalar(value.to_string()),
            flags: flags::SCALAR,
            base: 10,
            width: 0,
            level: 0,
            ename: None,
            old: None,
        }
    }

    pub fn new_integer(name: &str, value: i64) -> Self {
        Param {
            name: name.to_string(),
            value: ParamValue::Integer(value),
            flags: flags::INTEGER,
            base: 10,
            width: 0,
            level: 0,
            ename: None,
            old: None,
        }
    }

    pub fn new_float(name: &str, value: f64) -> Self {
        Param {
            name: name.to_string(),
            value: ParamValue::Float(value),
            flags: flags::FFLOAT,
            base: 10,
            width: 0,
            level: 0,
            ename: None,
            old: None,
        }
    }

    pub fn new_array(name: &str, value: Vec<String>) -> Self {
        Param {
            name: name.to_string(),
            value: ParamValue::Array(value),
            flags: flags::ARRAY,
            base: 10,
            width: 0,
            level: 0,
            ename: None,
            old: None,
        }
    }

    pub fn new_assoc(name: &str, value: HashMap<String, String>) -> Self {
        Param {
            name: name.to_string(),
            value: ParamValue::Assoc(value),
            flags: flags::HASHED,
            base: 10,
            width: 0,
            level: 0,
            ename: None,
            old: None,
        }
    }

    pub fn new_nameref(name: &str, target: &str) -> Self {
        Param {
            name: name.to_string(),
            value: ParamValue::Scalar(target.to_string()),
            flags: flags::NAMEREF,
            base: 0,
            width: 0,
            level: 0,
            ename: None,
            old: None,
        }
    }

    pub fn is_readonly(&self) -> bool {
        (self.flags & flags::READONLY) != 0
    }

    pub fn is_exported(&self) -> bool {
        (self.flags & flags::EXPORT) != 0
    }

    pub fn is_local(&self) -> bool {
        (self.flags & flags::LOCAL) != 0
    }

    pub fn is_special(&self) -> bool {
        (self.flags & flags::SPECIAL) != 0
    }

    pub fn is_integer(&self) -> bool {
        flags::pm_type(self.flags) == flags::INTEGER
    }

    pub fn is_float(&self) -> bool {
        let t = flags::pm_type(self.flags);
        t == flags::EFLOAT || t == flags::FFLOAT
    }

    pub fn is_array(&self) -> bool {
        flags::pm_type(self.flags) == flags::ARRAY
    }

    pub fn is_assoc(&self) -> bool {
        flags::pm_type(self.flags) == flags::HASHED
    }

    pub fn is_nameref(&self) -> bool {
        (self.flags & flags::NAMEREF) != 0
    }

    pub fn is_unset(&self) -> bool {
        (self.flags & flags::UNSET) != 0
    }

    pub fn is_tied(&self) -> bool {
        (self.flags & flags::TIED) != 0
    }

    pub fn is_hidden(&self) -> bool {
        (self.flags & flags::HIDE) != 0
    }

    pub fn is_unique(&self) -> bool {
        (self.flags & flags::UNIQUE) != 0
    }

    /// Get the string representation, applying formatting flags
    pub fn get_str_value(&self) -> String {
        let s = self.value.as_string();
        self.apply_case_transform(&s)
    }

    fn apply_case_transform(&self, s: &str) -> String {
        if (self.flags & flags::LOWER) != 0 {
            s.to_lowercase()
        } else if (self.flags & flags::UPPER) != 0 && !self.is_nameref() {
            s.to_uppercase()
        } else {
            s.to_string()
        }
    }

    /// Get the integer representation with base formatting
    pub fn get_int_str(&self) -> String {
        let val = self.value.as_integer();
        convbase(val, self.base as u32)
    }
}

// ---------------------------------------------------------------------------
// Tied parameter data
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct TiedData {
    pub join_char: char,
    pub scalar_name: String,
    pub array_name: String,
}

// ---------------------------------------------------------------------------
// Subscript flags for getarg()
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct SubscriptFlags {
    pub reverse: bool,   // (r) or (R) - reverse search
    pub down: bool,      // (R), (K), (I) - search from end
    pub index: bool,     // (i) or (I) - return index
    pub key_match: bool, // (k) or (K) - match keys in hash
    pub word: bool,      // (w) - word subscript
    pub num: i64,        // (n) - occurrence count
    pub begin: i64,      // (b) - begin offset
    pub has_begin: bool,
    pub separator: Option<String>, // (s) - word separator
    pub quote_arg: bool,           // (e) - exact/escape
}

// ---------------------------------------------------------------------------
// Subscript index result
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SubscriptIndex {
    pub start: i64,
    pub end: i64,
    pub is_all: bool,
}

impl SubscriptIndex {
    pub fn single(idx: i64) -> Self {
        SubscriptIndex {
            start: idx,
            end: idx + 1,
            is_all: false,
        }
    }

    pub fn range(start: i64, end: i64) -> Self {
        SubscriptIndex {
            start,
            end,
            is_all: false,
        }
    }

    pub fn all() -> Self {
        SubscriptIndex {
            start: 0,
            end: -1,
            is_all: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Parameter table print types (from printparamnode)
// ---------------------------------------------------------------------------

pub struct ParamTypeInfo {
    pub bin_flag: u32,
    pub string: &'static str,
    pub type_flag: char,
    pub use_base: bool,
    pub use_width: bool,
    pub test_level: bool,
}

pub const PM_TYPES: &[ParamTypeInfo] = &[
    ParamTypeInfo {
        bin_flag: flags::AUTOLOAD,
        string: "undefined",
        type_flag: '\0',
        use_base: false,
        use_width: false,
        test_level: false,
    },
    ParamTypeInfo {
        bin_flag: flags::INTEGER,
        string: "integer",
        type_flag: 'i',
        use_base: true,
        use_width: false,
        test_level: false,
    },
    ParamTypeInfo {
        bin_flag: flags::EFLOAT,
        string: "float",
        type_flag: 'E',
        use_base: false,
        use_width: false,
        test_level: false,
    },
    ParamTypeInfo {
        bin_flag: flags::FFLOAT,
        string: "float",
        type_flag: 'F',
        use_base: false,
        use_width: false,
        test_level: false,
    },
    ParamTypeInfo {
        bin_flag: flags::ARRAY,
        string: "array",
        type_flag: 'a',
        use_base: false,
        use_width: false,
        test_level: false,
    },
    ParamTypeInfo {
        bin_flag: flags::HASHED,
        string: "association",
        type_flag: 'A',
        use_base: false,
        use_width: false,
        test_level: false,
    },
    ParamTypeInfo {
        bin_flag: 0,
        string: "local",
        type_flag: '\0',
        use_base: false,
        use_width: false,
        test_level: true,
    },
    ParamTypeInfo {
        bin_flag: flags::HIDE,
        string: "hide",
        type_flag: 'h',
        use_base: false,
        use_width: false,
        test_level: false,
    },
    ParamTypeInfo {
        bin_flag: flags::LEFT,
        string: "left justified",
        type_flag: 'L',
        use_base: false,
        use_width: true,
        test_level: false,
    },
    ParamTypeInfo {
        bin_flag: flags::RIGHT_B,
        string: "right justified",
        type_flag: 'R',
        use_base: false,
        use_width: true,
        test_level: false,
    },
    ParamTypeInfo {
        bin_flag: flags::RIGHT_Z,
        string: "zero filled",
        type_flag: 'Z',
        use_base: false,
        use_width: true,
        test_level: false,
    },
    ParamTypeInfo {
        bin_flag: flags::LOWER,
        string: "lowercase",
        type_flag: 'l',
        use_base: false,
        use_width: false,
        test_level: false,
    },
    ParamTypeInfo {
        bin_flag: flags::UPPER,
        string: "uppercase",
        type_flag: 'u',
        use_base: false,
        use_width: false,
        test_level: false,
    },
    ParamTypeInfo {
        bin_flag: flags::READONLY,
        string: "readonly",
        type_flag: 'r',
        use_base: false,
        use_width: false,
        test_level: false,
    },
    ParamTypeInfo {
        bin_flag: flags::TAG,
        string: "tagged",
        type_flag: 't',
        use_base: false,
        use_width: false,
        test_level: false,
    },
    ParamTypeInfo {
        bin_flag: flags::EXPORT,
        string: "exported",
        type_flag: 'x',
        use_base: false,
        use_width: false,
        test_level: false,
    },
    ParamTypeInfo {
        bin_flag: flags::UNIQUE,
        string: "unique",
        type_flag: 'U',
        use_base: false,
        use_width: false,
        test_level: false,
    },
    ParamTypeInfo {
        bin_flag: flags::TIED,
        string: "tied",
        type_flag: 'T',
        use_base: false,
        use_width: false,
        test_level: false,
    },
    ParamTypeInfo {
        bin_flag: flags::NAMEREF,
        string: "nameref",
        type_flag: 'n',
        use_base: false,
        use_width: false,
        test_level: false,
    },
];

// ---------------------------------------------------------------------------
// Special parameter definitions table (mirrors special_params[] in C)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct SpecialParamDef {
    pub name: &'static str,
    pub pm_type: u32,  // PM_INTEGER | PM_SCALAR | PM_ARRAY
    pub pm_flags: u32, // PM_READONLY_SPECIAL, PM_DONTIMPORT, etc.
    pub tied_name: Option<&'static str>,
}

/// All special parameters from params.c special_params[]
pub const SPECIAL_PARAMS: &[SpecialParamDef] = &[
    // Integer specials with custom GSU
    SpecialParamDef {
        name: "#",
        pm_type: flags::INTEGER,
        pm_flags: flags::READONLY,
        tied_name: None,
    },
    SpecialParamDef {
        name: "ERRNO",
        pm_type: flags::INTEGER,
        pm_flags: flags::UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "GID",
        pm_type: flags::INTEGER,
        pm_flags: flags::DONTIMPORT,
        tied_name: None,
    },
    SpecialParamDef {
        name: "EGID",
        pm_type: flags::INTEGER,
        pm_flags: flags::DONTIMPORT,
        tied_name: None,
    },
    SpecialParamDef {
        name: "HISTSIZE",
        pm_type: flags::INTEGER,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "RANDOM",
        pm_type: flags::INTEGER,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "SAVEHIST",
        pm_type: flags::INTEGER,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "SECONDS",
        pm_type: flags::INTEGER,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "UID",
        pm_type: flags::INTEGER,
        pm_flags: flags::DONTIMPORT,
        tied_name: None,
    },
    SpecialParamDef {
        name: "EUID",
        pm_type: flags::INTEGER,
        pm_flags: flags::DONTIMPORT,
        tied_name: None,
    },
    SpecialParamDef {
        name: "TTYIDLE",
        pm_type: flags::INTEGER,
        pm_flags: flags::READONLY,
        tied_name: None,
    },
    // Scalar specials with custom GSU
    SpecialParamDef {
        name: "USERNAME",
        pm_type: flags::SCALAR,
        pm_flags: flags::DONTIMPORT,
        tied_name: None,
    },
    SpecialParamDef {
        name: "-",
        pm_type: flags::SCALAR,
        pm_flags: flags::READONLY,
        tied_name: None,
    },
    SpecialParamDef {
        name: "histchars",
        pm_type: flags::SCALAR,
        pm_flags: flags::DONTIMPORT,
        tied_name: None,
    },
    SpecialParamDef {
        name: "HOME",
        pm_type: flags::SCALAR,
        pm_flags: flags::UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "TERM",
        pm_type: flags::SCALAR,
        pm_flags: flags::UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "TERMINFO",
        pm_type: flags::SCALAR,
        pm_flags: flags::UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "TERMINFO_DIRS",
        pm_type: flags::SCALAR,
        pm_flags: flags::UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "WORDCHARS",
        pm_type: flags::SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "IFS",
        pm_type: flags::SCALAR,
        pm_flags: flags::DONTIMPORT,
        tied_name: None,
    },
    SpecialParamDef {
        name: "_",
        pm_type: flags::SCALAR,
        pm_flags: flags::DONTIMPORT,
        tied_name: None,
    },
    SpecialParamDef {
        name: "KEYBOARD_HACK",
        pm_type: flags::SCALAR,
        pm_flags: flags::DONTIMPORT,
        tied_name: None,
    },
    SpecialParamDef {
        name: "0",
        pm_type: flags::SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    // Readonly integer variables bound to C globals
    SpecialParamDef {
        name: "!",
        pm_type: flags::INTEGER,
        pm_flags: flags::READONLY,
        tied_name: None,
    },
    SpecialParamDef {
        name: "$",
        pm_type: flags::INTEGER,
        pm_flags: flags::READONLY,
        tied_name: None,
    },
    SpecialParamDef {
        name: "?",
        pm_type: flags::INTEGER,
        pm_flags: flags::READONLY,
        tied_name: None,
    },
    SpecialParamDef {
        name: "HISTCMD",
        pm_type: flags::INTEGER,
        pm_flags: flags::READONLY,
        tied_name: None,
    },
    SpecialParamDef {
        name: "LINENO",
        pm_type: flags::INTEGER,
        pm_flags: flags::READONLY,
        tied_name: None,
    },
    SpecialParamDef {
        name: "PPID",
        pm_type: flags::INTEGER,
        pm_flags: flags::READONLY,
        tied_name: None,
    },
    SpecialParamDef {
        name: "ZSH_SUBSHELL",
        pm_type: flags::INTEGER,
        pm_flags: flags::READONLY,
        tied_name: None,
    },
    // Settable integer variables
    SpecialParamDef {
        name: "COLUMNS",
        pm_type: flags::INTEGER,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "LINES",
        pm_type: flags::INTEGER,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "ZLE_RPROMPT_INDENT",
        pm_type: flags::INTEGER,
        pm_flags: flags::UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "SHLVL",
        pm_type: flags::INTEGER,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "FUNCNEST",
        pm_type: flags::INTEGER,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "OPTIND",
        pm_type: flags::INTEGER,
        pm_flags: flags::DONTIMPORT,
        tied_name: None,
    },
    SpecialParamDef {
        name: "TRY_BLOCK_ERROR",
        pm_type: flags::INTEGER,
        pm_flags: flags::DONTIMPORT,
        tied_name: None,
    },
    SpecialParamDef {
        name: "TRY_BLOCK_INTERRUPT",
        pm_type: flags::INTEGER,
        pm_flags: flags::DONTIMPORT,
        tied_name: None,
    },
    // Scalar variables bound to C globals
    SpecialParamDef {
        name: "OPTARG",
        pm_type: flags::SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "NULLCMD",
        pm_type: flags::SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "POSTEDIT",
        pm_type: flags::SCALAR,
        pm_flags: flags::UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "READNULLCMD",
        pm_type: flags::SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "PS1",
        pm_type: flags::SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "RPS1",
        pm_type: flags::SCALAR,
        pm_flags: flags::UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "RPROMPT",
        pm_type: flags::SCALAR,
        pm_flags: flags::UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "PS2",
        pm_type: flags::SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "RPS2",
        pm_type: flags::SCALAR,
        pm_flags: flags::UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "RPROMPT2",
        pm_type: flags::SCALAR,
        pm_flags: flags::UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "PS3",
        pm_type: flags::SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "PS4",
        pm_type: flags::SCALAR,
        pm_flags: flags::DONTIMPORT_SUID,
        tied_name: None,
    },
    SpecialParamDef {
        name: "SPROMPT",
        pm_type: flags::SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    // Readonly arrays
    SpecialParamDef {
        name: "*",
        pm_type: flags::ARRAY,
        pm_flags: flags::READONLY | flags::DONTIMPORT,
        tied_name: None,
    },
    SpecialParamDef {
        name: "@",
        pm_type: flags::ARRAY,
        pm_flags: flags::READONLY | flags::DONTIMPORT,
        tied_name: None,
    },
    // Tied colon-separated/array pairs
    SpecialParamDef {
        name: "CDPATH",
        pm_type: flags::SCALAR,
        pm_flags: flags::TIED,
        tied_name: Some("cdpath"),
    },
    SpecialParamDef {
        name: "FIGNORE",
        pm_type: flags::SCALAR,
        pm_flags: flags::TIED,
        tied_name: Some("fignore"),
    },
    SpecialParamDef {
        name: "FPATH",
        pm_type: flags::SCALAR,
        pm_flags: flags::TIED,
        tied_name: Some("fpath"),
    },
    SpecialParamDef {
        name: "MAILPATH",
        pm_type: flags::SCALAR,
        pm_flags: flags::TIED,
        tied_name: Some("mailpath"),
    },
    SpecialParamDef {
        name: "PATH",
        pm_type: flags::SCALAR,
        pm_flags: flags::TIED,
        tied_name: Some("path"),
    },
    SpecialParamDef {
        name: "PSVAR",
        pm_type: flags::SCALAR,
        pm_flags: flags::TIED,
        tied_name: Some("psvar"),
    },
    SpecialParamDef {
        name: "ZSH_EVAL_CONTEXT",
        pm_type: flags::SCALAR,
        pm_flags: flags::READONLY | flags::TIED,
        tied_name: Some("zsh_eval_context"),
    },
    SpecialParamDef {
        name: "MODULE_PATH",
        pm_type: flags::SCALAR,
        pm_flags: flags::DONTIMPORT | flags::TIED,
        tied_name: Some("module_path"),
    },
    SpecialParamDef {
        name: "MANPATH",
        pm_type: flags::SCALAR,
        pm_flags: flags::TIED,
        tied_name: Some("manpath"),
    },
    // Locale
    SpecialParamDef {
        name: "LANG",
        pm_type: flags::SCALAR,
        pm_flags: flags::UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "LC_ALL",
        pm_type: flags::SCALAR,
        pm_flags: flags::UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "LC_COLLATE",
        pm_type: flags::SCALAR,
        pm_flags: flags::UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "LC_CTYPE",
        pm_type: flags::SCALAR,
        pm_flags: flags::UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "LC_MESSAGES",
        pm_type: flags::SCALAR,
        pm_flags: flags::UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "LC_NUMERIC",
        pm_type: flags::SCALAR,
        pm_flags: flags::UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "LC_TIME",
        pm_type: flags::SCALAR,
        pm_flags: flags::UNSET,
        tied_name: None,
    },
    // Zsh-only aliases
    SpecialParamDef {
        name: "ARGC",
        pm_type: flags::INTEGER,
        pm_flags: flags::READONLY,
        tied_name: None,
    },
    SpecialParamDef {
        name: "HISTCHARS",
        pm_type: flags::SCALAR,
        pm_flags: flags::DONTIMPORT,
        tied_name: None,
    },
    SpecialParamDef {
        name: "status",
        pm_type: flags::INTEGER,
        pm_flags: flags::READONLY,
        tied_name: None,
    },
    SpecialParamDef {
        name: "prompt",
        pm_type: flags::SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "PROMPT",
        pm_type: flags::SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "PROMPT2",
        pm_type: flags::SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "PROMPT3",
        pm_type: flags::SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "PROMPT4",
        pm_type: flags::SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "argv",
        pm_type: flags::ARRAY,
        pm_flags: 0,
        tied_name: None,
    },
    // pipestatus array
    SpecialParamDef {
        name: "pipestatus",
        pm_type: flags::ARRAY,
        pm_flags: 0,
        tied_name: None,
    },
];

// ---------------------------------------------------------------------------
// Parameter table
// ---------------------------------------------------------------------------

pub struct ParamTable {
    params: HashMap<String, Param>,
    pub local_level: i32,
    shtimer_secs: u64,
    shtimer_instant: Instant,
    seconds_is_float: bool,
    /// Shell histchars: [bangchar, hatchar, hashchar]
    pub histchars: [u8; 3],
    /// Last exit status ($?)
    pub lastval: i64,
    /// PID ($$)
    pub mypid: i64,
    /// Last background PID ($!)
    pub lastpid: i64,
    /// Current history command number
    pub curhist: i64,
    /// Current line number ($LINENO)
    pub lineno: i64,
    /// Parent PID ($PPID)
    pub ppid: i64,
    /// Subshell nesting ($ZSH_SUBSHELL)
    pub zsh_subshell: i64,
    /// Terminal columns ($COLUMNS)
    pub columns: i64,
    /// Terminal lines ($LINES)
    pub lines: i64,
    /// $SHLVL
    pub shlvl: i64,
    /// Max function nesting ($FUNCNEST)
    pub funcnest: i64,
    /// $OPTIND
    pub optind: i64,
    /// $OPTARG
    pub optarg: String,
    /// TRY_BLOCK_ERROR
    pub try_errflag: i64,
    /// TRY_BLOCK_INTERRUPT
    pub try_interrupt: i64,
    /// ZLE_RPROMPT_INDENT
    pub rprompt_indent: i64,
    /// IFS value
    pub ifs: String,
    /// Underscore ($_)
    pub underscore: String,
    /// Positional parameters ($1, $2, ...)
    pub pparams: Vec<String>,
    /// $0
    pub argzero: String,
    /// Positional zero for POSIX
    pub posixzero: String,
    /// $pipestatus
    pub pipestats: Vec<i32>,
    /// Prompt strings
    pub prompt: String,
    pub prompt2: String,
    pub prompt3: String,
    pub prompt4: String,
    pub rprompt: String,
    pub rprompt2: String,
    pub sprompt: String,
    /// NULLCMD / READNULLCMD
    pub nullcmd: String,
    pub readnullcmd: String,
    /// POSTEDIT
    pub postedit: String,
    /// WORDCHARS
    pub wordchars: String,
    /// KEYBOARD_HACK
    pub keyboard_hack_char: u8,
    /// HOME
    pub home: String,
    /// TERM
    pub term: String,
    /// TERMINFO
    pub terminfo: String,
    /// TERMINFO_DIRS
    pub terminfo_dirs: String,
    /// Tied parameter bindings
    pub tied: HashMap<String, TiedData>,
    /// HISTSIZE
    pub histsize: i64,
    /// SAVEHIST
    pub savehist: i64,
    /// Options state for KSH_ARRAYS etc.
    pub ksh_arrays: bool,
    /// Options state for POSIX_ARGZERO
    pub posix_argzero: bool,
    /// Eval context stack
    pub zsh_eval_context: Vec<String>,
    /// RANDOM seed
    random_seed: u32,
}

impl Default for ParamTable {
    fn default() -> Self {
        Self::new()
    }
}

impl ParamTable {
    pub fn new() -> Self {
        let shtimer_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let pid = std::process::id() as i64;
        let shlvl = env::var("SHLVL")
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0)
            + 1;

        let home = env::var("HOME").unwrap_or_default();
        let term = env::var("TERM").unwrap_or_default();
        let ifs = " \t\n\0".to_string();

        let mut table = ParamTable {
            params: HashMap::new(),
            local_level: 0,
            shtimer_secs,
            shtimer_instant: Instant::now(),
            seconds_is_float: false,
            histchars: [b'!', b'^', b'#'],
            lastval: 0,
            mypid: pid,
            lastpid: 0,
            curhist: 0,
            lineno: 1,
            ppid: 0,
            zsh_subshell: 0,
            columns: 80,
            lines: 24,
            shlvl,
            funcnest: -1,
            optind: 1,
            optarg: String::new(),
            try_errflag: 0,
            try_interrupt: 0,
            rprompt_indent: 1,
            ifs,
            underscore: String::new(),
            pparams: Vec::new(),
            argzero: String::new(),
            posixzero: String::new(),
            pipestats: vec![0],
            prompt: "%m%# ".to_string(),
            prompt2: "%_> ".to_string(),
            prompt3: "?# ".to_string(),
            prompt4: "+%N:%i> ".to_string(),
            rprompt: String::new(),
            rprompt2: String::new(),
            sprompt: "zsh: correct '%R' to '%r' [nyae]? ".to_string(),
            nullcmd: "cat".to_string(),
            readnullcmd: "more".to_string(),
            postedit: String::new(),
            wordchars: "*?_-.[]~=/&;!#$%^(){}<>".to_string(),
            keyboard_hack_char: 0,
            home: home.clone(),
            term,
            terminfo: String::new(),
            terminfo_dirs: String::new(),
            tied: HashMap::new(),
            histsize: 30,
            savehist: 0,
            ksh_arrays: false,
            posix_argzero: false,
            zsh_eval_context: Vec::new(),
            random_seed: std::process::id(),
        };

        #[cfg(unix)]
        {
            table.ppid = unsafe { libc::getppid() } as i64;
        }

        // Try to get terminal size
        #[cfg(unix)]
        {
            let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
            if unsafe { libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) } == 0 {
                if ws.ws_col > 0 {
                    table.columns = ws.ws_col as i64;
                }
                if ws.ws_row > 0 {
                    table.lines = ws.ws_row as i64;
                }
            }
        }

        // Initialize special parameters
        table.init_special_params();

        // Setup tied parameters
        table.init_tied_params();

        // Import environment
        table.import_environment();

        // Set standard non-special parameters
        table.init_standard_params();

        table
    }

    fn init_special_params(&mut self) {
        // All special params get the SPECIAL flag
        for def in SPECIAL_PARAMS {
            let pm_flags = def.pm_type | def.pm_flags | flags::SPECIAL;
            let value = self.get_special_initial_value(def.name, def.pm_type);
            let param = Param {
                name: def.name.to_string(),
                value,
                flags: pm_flags,
                base: 10,
                width: 0,
                level: 0,
                ename: def.tied_name.map(|s| s.to_string()),
                old: None,
            };
            self.params.insert(def.name.to_string(), param);
        }
    }

    fn get_special_initial_value(&self, name: &str, pm_type: u32) -> ParamValue {
        match name {
            "$" => ParamValue::Integer(self.mypid),
            "?" | "status" => ParamValue::Integer(self.lastval),
            "!" => ParamValue::Integer(self.lastpid),
            "#" | "ARGC" => ParamValue::Integer(self.pparams.len() as i64),
            "PPID" => ParamValue::Integer(self.ppid),
            "LINENO" => ParamValue::Integer(self.lineno),
            "HISTCMD" => ParamValue::Integer(self.curhist),
            "ZSH_SUBSHELL" => ParamValue::Integer(self.zsh_subshell),
            "COLUMNS" => ParamValue::Integer(self.columns),
            "LINES" => ParamValue::Integer(self.lines),
            "SHLVL" => ParamValue::Integer(self.shlvl),
            "FUNCNEST" => ParamValue::Integer(self.funcnest),
            "OPTIND" => ParamValue::Integer(self.optind),
            "TRY_BLOCK_ERROR" => ParamValue::Integer(self.try_errflag),
            "TRY_BLOCK_INTERRUPT" => ParamValue::Integer(self.try_interrupt),
            "ZLE_RPROMPT_INDENT" => ParamValue::Integer(self.rprompt_indent),
            "RANDOM" => ParamValue::Integer(0),
            "SECONDS" => ParamValue::Integer(0),
            "HISTSIZE" => ParamValue::Integer(self.histsize),
            "SAVEHIST" => ParamValue::Integer(self.savehist),
            "ERRNO" => ParamValue::Integer(0),
            "TTYIDLE" => ParamValue::Integer(-1),
            "UID" => {
                #[cfg(unix)]
                {
                    ParamValue::Integer(unsafe { libc::getuid() } as i64)
                }
                #[cfg(not(unix))]
                {
                    ParamValue::Integer(0)
                }
            }
            "EUID" => {
                #[cfg(unix)]
                {
                    ParamValue::Integer(unsafe { libc::geteuid() } as i64)
                }
                #[cfg(not(unix))]
                {
                    ParamValue::Integer(0)
                }
            }
            "GID" => {
                #[cfg(unix)]
                {
                    ParamValue::Integer(unsafe { libc::getgid() } as i64)
                }
                #[cfg(not(unix))]
                {
                    ParamValue::Integer(0)
                }
            }
            "EGID" => {
                #[cfg(unix)]
                {
                    ParamValue::Integer(unsafe { libc::getegid() } as i64)
                }
                #[cfg(not(unix))]
                {
                    ParamValue::Integer(0)
                }
            }
            "USERNAME" => {
                let name = env::var("USER")
                    .or_else(|_| env::var("LOGNAME"))
                    .unwrap_or_else(|_| "unknown".to_string());
                ParamValue::Scalar(name)
            }
            "-" => ParamValue::Scalar(String::new()), // dash: current option flags
            "histchars" | "HISTCHARS" => {
                let s = String::from_utf8_lossy(&self.histchars).to_string();
                ParamValue::Scalar(s)
            }
            "HOME" => ParamValue::Scalar(self.home.clone()),
            "TERM" => ParamValue::Scalar(self.term.clone()),
            "TERMINFO" => ParamValue::Scalar(self.terminfo.clone()),
            "TERMINFO_DIRS" => ParamValue::Scalar(self.terminfo_dirs.clone()),
            "WORDCHARS" => ParamValue::Scalar(self.wordchars.clone()),
            "IFS" => ParamValue::Scalar(self.ifs.clone()),
            "_" => ParamValue::Scalar(self.underscore.clone()),
            "KEYBOARD_HACK" => ParamValue::Scalar(String::new()),
            "0" => ParamValue::Scalar(self.argzero.clone()),
            "OPTARG" => ParamValue::Scalar(self.optarg.clone()),
            "NULLCMD" => ParamValue::Scalar(self.nullcmd.clone()),
            "READNULLCMD" => ParamValue::Scalar(self.readnullcmd.clone()),
            "POSTEDIT" => ParamValue::Scalar(self.postedit.clone()),
            "PS1" | "prompt" | "PROMPT" => ParamValue::Scalar(self.prompt.clone()),
            "PS2" | "PROMPT2" => ParamValue::Scalar(self.prompt2.clone()),
            "PS3" | "PROMPT3" => ParamValue::Scalar(self.prompt3.clone()),
            "PS4" | "PROMPT4" => ParamValue::Scalar(self.prompt4.clone()),
            "RPS1" | "RPROMPT" => ParamValue::Scalar(self.rprompt.clone()),
            "RPS2" | "RPROMPT2" => ParamValue::Scalar(self.rprompt2.clone()),
            "SPROMPT" => ParamValue::Scalar(self.sprompt.clone()),
            "*" | "@" | "argv" => ParamValue::Array(self.pparams.clone()),
            "pipestatus" => {
                ParamValue::Array(self.pipestats.iter().map(|s| s.to_string()).collect())
            }
            // Tied colon-separated paths
            "CDPATH" | "FIGNORE" | "FPATH" | "MAILPATH" | "PATH" | "PSVAR" | "ZSH_EVAL_CONTEXT"
            | "MODULE_PATH" | "MANPATH" => {
                let env_val = env::var(name).unwrap_or_default();
                ParamValue::Scalar(env_val)
            }
            // Locale
            "LANG" | "LC_ALL" | "LC_COLLATE" | "LC_CTYPE" | "LC_MESSAGES" | "LC_NUMERIC"
            | "LC_TIME" => {
                let env_val = env::var(name).unwrap_or_default();
                ParamValue::Scalar(env_val)
            }
            _ => {
                if pm_type == flags::INTEGER {
                    ParamValue::Integer(0)
                } else if pm_type == flags::ARRAY {
                    ParamValue::Array(Vec::new())
                } else {
                    ParamValue::Scalar(String::new())
                }
            }
        }
    }

    fn init_tied_params(&mut self) {
        // Set up tied parameter pairs (scalar PATH <-> array path)
        let pairs: &[(&str, &str)] = &[
            ("CDPATH", "cdpath"),
            ("FIGNORE", "fignore"),
            ("FPATH", "fpath"),
            ("MAILPATH", "mailpath"),
            ("PATH", "path"),
            ("PSVAR", "psvar"),
            ("ZSH_EVAL_CONTEXT", "zsh_eval_context"),
            ("MODULE_PATH", "module_path"),
            ("MANPATH", "manpath"),
        ];

        for (scalar, array) in pairs {
            let val = env::var(scalar).unwrap_or_default();
            let arr: Vec<String> = val
                .split(':')
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect();

            // Create the array side
            let arr_flags = flags::ARRAY | flags::SPECIAL | flags::TIED;
            let arr_param = Param {
                name: array.to_string(),
                value: ParamValue::Array(arr),
                flags: arr_flags,
                base: 10,
                width: 0,
                level: 0,
                ename: Some(scalar.to_string()),
                old: None,
            };
            self.params.insert(array.to_string(), arr_param);

            // Mark the scalar side as tied
            if let Some(p) = self.params.get_mut(*scalar) {
                p.flags |= flags::TIED;
                p.ename = Some(array.to_string());
            }

            self.tied.insert(
                scalar.to_string(),
                TiedData {
                    join_char: ':',
                    scalar_name: scalar.to_string(),
                    array_name: array.to_string(),
                },
            );
        }
    }

    fn import_environment(&mut self) {
        for (key, value) in env::vars() {
            if !self.params.contains_key(&key) && isident(&key) {
                let mut param = Param::new_scalar(&key, &value);
                param.flags |= flags::EXPORT;
                self.params.insert(key, param);
            }
        }
    }

    fn init_standard_params(&mut self) {
        // HOST
        let hostname = {
            #[cfg(unix)]
            {
                let mut buf = [0u8; 256];
                let ptr = buf.as_mut_ptr() as *mut libc::c_char;
                if unsafe { libc::gethostname(ptr, 256) } == 0 {
                    let cstr = unsafe { std::ffi::CStr::from_ptr(ptr) };
                    cstr.to_string_lossy().to_string()
                } else {
                    "unknown".to_string()
                }
            }
            #[cfg(not(unix))]
            {
                "unknown".to_string()
            }
        };
        self.set_scalar_internal("HOST", &hostname, 0);

        // LOGNAME
        let logname = env::var("LOGNAME")
            .or_else(|_| env::var("USER"))
            .unwrap_or_else(|_| "unknown".to_string());
        self.set_scalar_internal("LOGNAME", &logname, 0);

        // MACHTYPE, OSTYPE, VENDOR
        self.set_scalar_internal("MACHTYPE", std::env::consts::ARCH, 0);
        self.set_scalar_internal("OSTYPE", std::env::consts::OS, 0);
        self.set_scalar_internal("VENDOR", "unknown", 0);

        // TTY
        #[cfg(unix)]
        {
            let tty = unsafe {
                let ptr = libc::ttyname(0);
                if ptr.is_null() {
                    String::new()
                } else {
                    std::ffi::CStr::from_ptr(ptr).to_string_lossy().to_string()
                }
            };
            self.set_scalar_internal("TTY", &tty, 0);
        }

        // ZSH_VERSION / ZSH_PATCHLEVEL
        self.set_scalar_internal("ZSH_VERSION", "5.9", 0);
        self.set_scalar_internal("ZSH_PATCHLEVEL", "zshrs", 0);

        // Defaults
        self.set_integer_internal("MAILCHECK", 60, 0);
        self.set_integer_internal("KEYTIMEOUT", 40, 0);
        self.set_integer_internal("LISTMAX", 100, 0);
        self.set_scalar_internal("TMPPREFIX", "/tmp/zsh", 0);
        self.set_scalar_internal("TIMEFMT", "%J  %U user %S system %P cpu %*E total", 0);

        // Signals array
        #[cfg(unix)]
        {
            let sigs = vec![
                "EXIT", "HUP", "INT", "QUIT", "ILL", "TRAP", "ABRT", "EMT", "FPE", "KILL", "BUS",
                "SEGV", "SYS", "PIPE", "ALRM", "TERM", "URG", "STOP", "TSTP", "CONT", "CHLD",
                "TTIN", "TTOU", "IO", "XCPU", "XFSZ", "VTALRM", "PROF", "WINCH", "INFO", "USR1",
                "USR2",
            ];
            let sig_arr: Vec<String> = sigs.iter().map(|s| format!("SIG{}", s)).collect();
            self.set_array_internal("signals", sig_arr, flags::READONLY);
        }
    }

    fn set_scalar_internal(&mut self, name: &str, value: &str, extra_flags: u32) {
        if !self.params.contains_key(name) {
            let mut param = Param::new_scalar(name, value);
            param.flags |= extra_flags;
            self.params.insert(name.to_string(), param);
        }
    }

    fn set_integer_internal(&mut self, name: &str, value: i64, extra_flags: u32) {
        if !self.params.contains_key(name) {
            let mut param = Param::new_integer(name, value);
            param.flags |= extra_flags;
            self.params.insert(name.to_string(), param);
        }
    }

    fn set_array_internal(&mut self, name: &str, value: Vec<String>, extra_flags: u32) {
        if !self.params.contains_key(name) {
            let mut param = Param::new_array(name, value);
            param.flags |= extra_flags;
            self.params.insert(name.to_string(), param);
        }
    }

    // -----------------------------------------------------------------------
    // Special parameter dynamic getters
    // -----------------------------------------------------------------------

    /// Get a special parameter value dynamically.
    /// Returns None if not special (caller should use stored value).
    fn get_special_value(&self, name: &str) -> Option<ParamValue> {
        match name {
            "$" => Some(ParamValue::Integer(self.mypid)),
            "?" | "status" => Some(ParamValue::Integer(self.lastval)),
            "!" => Some(ParamValue::Integer(self.lastpid)),
            "#" | "ARGC" => Some(ParamValue::Integer(self.pparams.len() as i64)),
            "PPID" => Some(ParamValue::Integer(self.ppid)),
            "LINENO" => Some(ParamValue::Integer(self.lineno)),
            "HISTCMD" => Some(ParamValue::Integer(self.curhist)),
            "ZSH_SUBSHELL" => Some(ParamValue::Integer(self.zsh_subshell)),
            "COLUMNS" => Some(ParamValue::Integer(self.columns)),
            "LINES" => Some(ParamValue::Integer(self.lines)),
            "SHLVL" => Some(ParamValue::Integer(self.shlvl)),
            "FUNCNEST" => Some(ParamValue::Integer(self.funcnest)),
            "OPTIND" => Some(ParamValue::Integer(self.optind)),
            "TRY_BLOCK_ERROR" => Some(ParamValue::Integer(self.try_errflag)),
            "TRY_BLOCK_INTERRUPT" => Some(ParamValue::Integer(self.try_interrupt)),
            "ZLE_RPROMPT_INDENT" => Some(ParamValue::Integer(self.rprompt_indent)),
            "HISTSIZE" => Some(ParamValue::Integer(self.histsize)),
            "SAVEHIST" => Some(ParamValue::Integer(self.savehist)),
            "RANDOM" => Some(ParamValue::Integer(self.get_random())),
            "SECONDS" => {
                if self.seconds_is_float {
                    Some(ParamValue::Float(self.get_seconds_float()))
                } else {
                    Some(ParamValue::Integer(self.get_seconds_int()))
                }
            }
            "ERRNO" => {
                #[cfg(unix)]
                {
                    Some(ParamValue::Integer(
                        std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as i64,
                    ))
                }
                #[cfg(not(unix))]
                {
                    Some(ParamValue::Integer(0))
                }
            }
            "TTYIDLE" => Some(ParamValue::Integer(self.get_tty_idle())),
            "UID" => {
                #[cfg(unix)]
                {
                    Some(ParamValue::Integer(unsafe { libc::getuid() } as i64))
                }
                #[cfg(not(unix))]
                {
                    Some(ParamValue::Integer(0))
                }
            }
            "EUID" => {
                #[cfg(unix)]
                {
                    Some(ParamValue::Integer(unsafe { libc::geteuid() } as i64))
                }
                #[cfg(not(unix))]
                {
                    Some(ParamValue::Integer(0))
                }
            }
            "GID" => {
                #[cfg(unix)]
                {
                    Some(ParamValue::Integer(unsafe { libc::getgid() } as i64))
                }
                #[cfg(not(unix))]
                {
                    Some(ParamValue::Integer(0))
                }
            }
            "EGID" => {
                #[cfg(unix)]
                {
                    Some(ParamValue::Integer(unsafe { libc::getegid() } as i64))
                }
                #[cfg(not(unix))]
                {
                    Some(ParamValue::Integer(0))
                }
            }
            "USERNAME" => {
                let name = env::var("USER")
                    .or_else(|_| env::var("LOGNAME"))
                    .unwrap_or_else(|_| "unknown".to_string());
                Some(ParamValue::Scalar(name))
            }
            "-" => {
                // Return current option string
                Some(ParamValue::Scalar(String::new()))
            }
            "histchars" | "HISTCHARS" => {
                let s = String::from_utf8_lossy(&self.histchars).to_string();
                Some(ParamValue::Scalar(s))
            }
            "IFS" => Some(ParamValue::Scalar(self.ifs.clone())),
            "_" => Some(ParamValue::Scalar(self.underscore.clone())),
            "KEYBOARD_HACK" => {
                let s = if self.keyboard_hack_char != 0 {
                    String::from(self.keyboard_hack_char as char)
                } else {
                    String::new()
                };
                Some(ParamValue::Scalar(s))
            }
            "HOME" => Some(ParamValue::Scalar(self.home.clone())),
            "WORDCHARS" => Some(ParamValue::Scalar(self.wordchars.clone())),
            "TERM" => Some(ParamValue::Scalar(self.term.clone())),
            "TERMINFO" => Some(ParamValue::Scalar(self.terminfo.clone())),
            "TERMINFO_DIRS" => Some(ParamValue::Scalar(self.terminfo_dirs.clone())),
            "0" => {
                if self.posix_argzero {
                    Some(ParamValue::Scalar(self.posixzero.clone()))
                } else {
                    Some(ParamValue::Scalar(self.argzero.clone()))
                }
            }
            "OPTARG" => Some(ParamValue::Scalar(self.optarg.clone())),
            "NULLCMD" => Some(ParamValue::Scalar(self.nullcmd.clone())),
            "READNULLCMD" => Some(ParamValue::Scalar(self.readnullcmd.clone())),
            "POSTEDIT" => Some(ParamValue::Scalar(self.postedit.clone())),
            "PS1" | "prompt" | "PROMPT" => Some(ParamValue::Scalar(self.prompt.clone())),
            "PS2" | "PROMPT2" => Some(ParamValue::Scalar(self.prompt2.clone())),
            "PS3" | "PROMPT3" => Some(ParamValue::Scalar(self.prompt3.clone())),
            "PS4" | "PROMPT4" => Some(ParamValue::Scalar(self.prompt4.clone())),
            "RPS1" | "RPROMPT" => Some(ParamValue::Scalar(self.rprompt.clone())),
            "RPS2" | "RPROMPT2" => Some(ParamValue::Scalar(self.rprompt2.clone())),
            "SPROMPT" => Some(ParamValue::Scalar(self.sprompt.clone())),
            "*" | "@" | "argv" => Some(ParamValue::Array(self.pparams.clone())),
            "pipestatus" => Some(ParamValue::Array(
                self.pipestats.iter().map(|s| s.to_string()).collect(),
            )),
            _ => None,
        }
    }

    /// Handle special parameter set side-effects
    fn handle_special_set(&mut self, name: &str, value: &ParamValue) {
        match name {
            "RANDOM" => {
                if let ParamValue::Integer(v) = value {
                    self.random_seed = *v as u32;
                    // Re-seed
                }
            }
            "SECONDS" => match value {
                ParamValue::Integer(x) => {
                    let now = Instant::now();
                    self.shtimer_instant = now - std::time::Duration::from_secs(*x as u64);
                    self.seconds_is_float = false;
                }
                ParamValue::Float(x) => {
                    let now = Instant::now();
                    self.shtimer_instant = now - std::time::Duration::from_secs_f64(*x);
                    self.seconds_is_float = true;
                }
                _ => {}
            },
            "HISTSIZE" => {
                if let ParamValue::Integer(v) = value {
                    self.histsize = (*v).max(1);
                }
            }
            "SAVEHIST" => {
                if let ParamValue::Integer(v) = value {
                    self.savehist = (*v).max(0);
                }
            }
            "COLUMNS" => {
                if let ParamValue::Integer(v) = value {
                    self.columns = *v;
                }
            }
            "LINES" => {
                if let ParamValue::Integer(v) = value {
                    self.lines = *v;
                }
            }
            "SHLVL" => {
                if let ParamValue::Integer(v) = value {
                    self.shlvl = *v;
                }
            }
            "FUNCNEST" => {
                if let ParamValue::Integer(v) = value {
                    self.funcnest = *v;
                }
            }
            "OPTIND" => {
                if let ParamValue::Integer(v) = value {
                    self.optind = *v;
                }
            }
            "TRY_BLOCK_ERROR" => {
                if let ParamValue::Integer(v) = value {
                    self.try_errflag = *v;
                }
            }
            "TRY_BLOCK_INTERRUPT" => {
                if let ParamValue::Integer(v) = value {
                    self.try_interrupt = *v;
                }
            }
            "ZLE_RPROMPT_INDENT" => {
                if let ParamValue::Integer(v) = value {
                    self.rprompt_indent = *v;
                }
            }
            "IFS" => {
                self.ifs = value.as_string();
            }
            "HOME" => {
                self.home = value.as_string();
            }
            "TERM" => {
                self.term = value.as_string();
            }
            "TERMINFO" => {
                self.terminfo = value.as_string();
            }
            "TERMINFO_DIRS" => {
                self.terminfo_dirs = value.as_string();
            }
            "WORDCHARS" => {
                self.wordchars = value.as_string();
            }
            "KEYBOARD_HACK" => {
                let s = value.as_string();
                self.keyboard_hack_char = s.as_bytes().first().copied().unwrap_or(0);
            }
            "histchars" | "HISTCHARS" => {
                let s = value.as_string();
                let bytes = s.as_bytes();
                self.histchars[0] = bytes.first().copied().unwrap_or(b'!');
                self.histchars[1] = bytes.get(1).copied().unwrap_or(b'^');
                self.histchars[2] = bytes.get(2).copied().unwrap_or(b'#');
            }
            "0" => {
                if !self.posix_argzero {
                    self.argzero = value.as_string();
                }
            }
            "OPTARG" => {
                self.optarg = value.as_string();
            }
            "NULLCMD" => {
                self.nullcmd = value.as_string();
            }
            "READNULLCMD" => {
                self.readnullcmd = value.as_string();
            }
            "POSTEDIT" => {
                self.postedit = value.as_string();
            }
            "PS1" | "prompt" | "PROMPT" => {
                self.prompt = value.as_string();
            }
            "PS2" | "PROMPT2" => {
                self.prompt2 = value.as_string();
            }
            "PS3" | "PROMPT3" => {
                self.prompt3 = value.as_string();
            }
            "PS4" | "PROMPT4" => {
                self.prompt4 = value.as_string();
            }
            "RPS1" | "RPROMPT" => {
                self.rprompt = value.as_string();
            }
            "RPS2" | "RPROMPT2" => {
                self.rprompt2 = value.as_string();
            }
            "SPROMPT" => {
                self.sprompt = value.as_string();
            }
            "pipestatus" => {
                if let ParamValue::Array(arr) = value {
                    self.pipestats = arr.iter().map(|s| s.parse::<i32>().unwrap_or(0)).collect();
                }
            }
            #[cfg(unix)]
            "UID" => {
                if let ParamValue::Integer(v) = value {
                    unsafe {
                        libc::setuid(*v as libc::uid_t);
                    }
                }
            }
            #[cfg(unix)]
            "EUID" => {
                if let ParamValue::Integer(v) = value {
                    unsafe {
                        libc::seteuid(*v as libc::uid_t);
                    }
                }
            }
            #[cfg(unix)]
            "GID" => {
                if let ParamValue::Integer(v) = value {
                    unsafe {
                        libc::setgid(*v as libc::gid_t);
                    }
                }
            }
            #[cfg(unix)]
            "EGID" => {
                if let ParamValue::Integer(v) = value {
                    unsafe {
                        libc::setegid(*v as libc::gid_t);
                    }
                }
            }
            _ => {}
        }

        // Handle tied parameter sync
        if let Some(tied) = self.tied.get(name).cloned() {
            if name == tied.scalar_name {
                // Scalar changed -> update array
                let arr: Vec<String> = value
                    .as_string()
                    .split(tied.join_char)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect();
                if let Some(p) = self.params.get_mut(&tied.array_name) {
                    p.value = ParamValue::Array(arr);
                }
            } else if name == tied.array_name {
                // Array changed -> update scalar
                let s = value.as_array().join(&tied.join_char.to_string());
                if let Some(p) = self.params.get_mut(&tied.scalar_name) {
                    p.value = ParamValue::Scalar(s.clone());
                }
                // Update environment
                if let Some(p) = self.params.get(&tied.scalar_name) {
                    if p.is_exported() {
                        env::set_var(&tied.scalar_name, &s);
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Special value getters
    // -----------------------------------------------------------------------

    pub fn get_random(&self) -> i64 {
        // Simple LCG PRNG matching zsh's rand() & 0x7fff
        static COUNTER: AtomicI64 = AtomicI64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let seed = self.random_seed as i64;
        // Linear congruential generator
        let val = (seed
            .wrapping_mul(1103515245)
            .wrapping_add(12345)
            .wrapping_add(n))
            & 0x7fffffff;
        (val >> 16) & 0x7fff
    }

    pub fn get_seconds_int(&self) -> i64 {
        self.shtimer_instant.elapsed().as_secs() as i64
    }

    pub fn get_seconds_float(&self) -> f64 {
        self.shtimer_instant.elapsed().as_secs_f64()
    }

    /// Get the SECONDS value
    pub fn get_seconds(&self) -> f64 {
        self.get_seconds_float()
    }

    pub fn set_seconds_type(&mut self, is_float: bool) {
        self.seconds_is_float = is_float;
    }

    fn get_tty_idle(&self) -> i64 {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let fd = std::io::stdin().as_raw_fd();
            let mut stat: libc::stat = unsafe { std::mem::zeroed() };
            if unsafe { libc::fstat(fd, &mut stat) } == 0 {
                let now = unsafe { libc::time(std::ptr::null_mut()) };
                return now as i64 - stat.st_atime as i64;
            }
        }
        -1
    }

    // -----------------------------------------------------------------------
    // Core get/set/unset operations
    // -----------------------------------------------------------------------

    /// Get a parameter value, resolving specials and namerefs
    pub fn get(&self, name: &str) -> Option<&ParamValue> {
        // Check for nameref resolution
        let resolved = self.resolve_nameref_name(name);
        let lookup = resolved.as_deref().unwrap_or(name);

        self.params.get(lookup).map(|p| &p.value)
    }

    /// Get a parameter value, including dynamic specials
    pub fn get_value(&self, name: &str) -> Option<ParamValue> {
        let resolved = self.resolve_nameref_name(name);
        let lookup = resolved.as_deref().unwrap_or(name);

        // Check dynamic specials first
        if let Some(p) = self.params.get(lookup) {
            if p.is_special() {
                if let Some(val) = self.get_special_value(lookup) {
                    return Some(val);
                }
            }
            if !p.is_unset() {
                return Some(p.value.clone());
            }
        }
        None
    }

    /// Get the full parameter struct
    pub fn get_param(&self, name: &str) -> Option<&Param> {
        self.params.get(name)
    }

    /// Get mutable parameter
    pub fn get_param_mut(&mut self, name: &str) -> Option<&mut Param> {
        self.params.get_mut(name)
    }

    /// Resolve nameref chain, returning the final target name
    fn resolve_nameref_name(&self, name: &str) -> Option<String> {
        let param = self.params.get(name)?;
        if !param.is_nameref() || param.is_unset() {
            return None;
        }
        let target = param.value.as_string();
        if target.is_empty() || target == name {
            return None;
        }
        // Follow chain, with loop detection
        let mut visited = HashSet::new();
        visited.insert(name.to_string());
        let mut current = target;
        loop {
            if visited.contains(&current) {
                return None; // Loop detected
            }
            visited.insert(current.clone());
            if let Some(p) = self.params.get(&current) {
                if p.is_nameref() && !p.is_unset() {
                    let next = p.value.as_string();
                    if next.is_empty() {
                        return Some(current);
                    }
                    current = next;
                } else {
                    return Some(current);
                }
            } else {
                return Some(current);
            }
        }
    }

    /// Set a scalar parameter
    pub fn set_scalar(&mut self, name: &str, value: &str) -> bool {
        let resolved = self
            .resolve_nameref_name(name)
            .unwrap_or_else(|| name.to_string());
        let name = &resolved;

        if let Some(param) = self.params.get_mut(name) {
            if param.is_readonly() {
                return false;
            }
            let value = if (param.flags & flags::LOWER) != 0 {
                value.to_lowercase()
            } else if (param.flags & flags::UPPER) != 0 {
                value.to_uppercase()
            } else {
                value.to_string()
            };
            let pv = ParamValue::Scalar(value);
            param.value = pv.clone();
            param.flags &= !flags::UNSET;

            if param.is_exported() {
                env::set_var(name, param.value.as_string());
            }

            self.handle_special_set(name, &pv);
            true
        } else {
            let param = Param::new_scalar(name, value);
            let pv = param.value.clone();
            self.params.insert(name.to_string(), param);
            self.handle_special_set(name, &pv);
            true
        }
    }

    /// Set an integer parameter
    pub fn set_integer(&mut self, name: &str, value: i64) -> bool {
        let resolved = self
            .resolve_nameref_name(name)
            .unwrap_or_else(|| name.to_string());
        let name = &resolved;

        if let Some(param) = self.params.get_mut(name) {
            if param.is_readonly() {
                return false;
            }
            let pv = ParamValue::Integer(value);
            param.value = pv.clone();
            param.flags &= !flags::UNSET;
            if param.is_exported() {
                env::set_var(name, value.to_string());
            }
            self.handle_special_set(name, &pv);
            true
        } else {
            let param = Param::new_integer(name, value);
            let pv = param.value.clone();
            self.params.insert(name.to_string(), param);
            self.handle_special_set(name, &pv);
            true
        }
    }

    /// Set a float parameter
    pub fn set_float(&mut self, name: &str, value: f64) -> bool {
        let resolved = self
            .resolve_nameref_name(name)
            .unwrap_or_else(|| name.to_string());
        let name = &resolved;

        if let Some(param) = self.params.get_mut(name) {
            if param.is_readonly() {
                return false;
            }
            let pv = ParamValue::Float(value);
            param.value = pv.clone();
            param.flags &= !flags::UNSET;
            self.handle_special_set(name, &pv);
            true
        } else {
            let param = Param::new_float(name, value);
            let pv = param.value.clone();
            self.params.insert(name.to_string(), param);
            self.handle_special_set(name, &pv);
            true
        }
    }

    /// Set an array parameter
    pub fn set_array(&mut self, name: &str, value: Vec<String>) -> bool {
        let resolved = self
            .resolve_nameref_name(name)
            .unwrap_or_else(|| name.to_string());
        let name = &resolved;

        if let Some(param) = self.params.get_mut(name) {
            if param.is_readonly() {
                return false;
            }
            let value = if param.is_unique() {
                uniq_array(value)
            } else {
                value
            };
            let pv = ParamValue::Array(value);
            param.value = pv.clone();
            param.flags &= !flags::UNSET;
            self.handle_special_set(name, &pv);
            true
        } else {
            let param = Param::new_array(name, value);
            let pv = param.value.clone();
            self.params.insert(name.to_string(), param);
            self.handle_special_set(name, &pv);
            true
        }
    }

    /// Set an associative array parameter
    pub fn set_assoc(&mut self, name: &str, value: HashMap<String, String>) -> bool {
        let resolved = self
            .resolve_nameref_name(name)
            .unwrap_or_else(|| name.to_string());
        let name = &resolved;

        if let Some(param) = self.params.get_mut(name) {
            if param.is_readonly() {
                return false;
            }
            param.value = ParamValue::Assoc(value);
            param.flags &= !flags::UNSET;
            true
        } else {
            let param = Param::new_assoc(name, value);
            self.params.insert(name.to_string(), param);
            true
        }
    }

    /// Set a numeric value (MNumber)
    pub fn set_numeric(&mut self, name: &str, val: MNumber) -> bool {
        match val {
            MNumber::Integer(i) => self.set_integer(name, i),
            MNumber::Float(f) => self.set_float(name, f),
        }
    }

    /// Augmented assignment (+=)
    pub fn augment_scalar(&mut self, name: &str, value: &str) -> bool {
        if let Some(current) = self.get_value(name) {
            let new_val = format!("{}{}", current.as_string(), value);
            self.set_scalar(name, &new_val)
        } else {
            self.set_scalar(name, value)
        }
    }

    /// Augmented assignment for arrays (+=)
    pub fn augment_array(&mut self, name: &str, value: Vec<String>) -> bool {
        if let Some(current) = self.get_value(name) {
            let mut arr = current.as_array();
            arr.extend(value);
            self.set_array(name, arr)
        } else {
            self.set_array(name, value)
        }
    }

    /// Augmented assignment for integers (+=)
    pub fn augment_integer(&mut self, name: &str, value: i64) -> bool {
        let current = self.get_value(name).map(|v| v.as_integer()).unwrap_or(0);
        self.set_integer(name, current + value)
    }

    /// Unset a parameter
    pub fn unset(&mut self, name: &str) -> bool {
        if let Some(param) = self.params.get(name) {
            if param.is_readonly() {
                return false;
            }
        }

        // Handle tied parameter cleanup
        if let Some(tied) = self.tied.get(name).cloned() {
            if name == tied.scalar_name {
                if let Some(p) = self.params.get_mut(&tied.array_name) {
                    p.flags |= flags::UNSET;
                    p.value = ParamValue::Array(Vec::new());
                }
            } else if name == tied.array_name {
                if let Some(p) = self.params.get_mut(&tied.scalar_name) {
                    p.flags |= flags::UNSET;
                    p.value = ParamValue::Scalar(String::new());
                }
            }
        }

        // For special params, mark unset but don't remove
        if let Some(param) = self.params.get(name) {
            if param.is_special() {
                if let Some(p) = self.params.get_mut(name) {
                    p.flags |= flags::UNSET;
                }
                return true;
            }
        }

        // Check for local scope: keep struct but mark unset
        if let Some(param) = self.params.get(name) {
            if param.level > 0 && param.level <= self.local_level {
                if let Some(p) = self.params.get_mut(name) {
                    p.flags |= flags::UNSET;
                }
                return true;
            }
        }

        env::remove_var(name);

        // If there's an old param, restore it
        let old = self.params.get(name).and_then(|p| p.old.clone());
        if let Some(old_param) = old {
            self.params.insert(name.to_string(), *old_param);
            // Re-export if needed
            if let Some(p) = self.params.get(name) {
                if p.is_exported() {
                    env::set_var(name, p.value.as_string());
                }
            }
        } else {
            self.params.remove(name);
        }
        true
    }

    /// Export a parameter
    pub fn export(&mut self, name: &str) -> bool {
        if let Some(param) = self.params.get_mut(name) {
            param.flags |= flags::EXPORT;
            env::set_var(name, param.value.as_string());
            true
        } else {
            false
        }
    }

    /// Unexport a parameter
    pub fn unexport(&mut self, name: &str) {
        if let Some(param) = self.params.get_mut(name) {
            param.flags &= !flags::EXPORT;
            env::remove_var(name);
        }
    }

    /// Mark parameter as readonly
    pub fn set_readonly(&mut self, name: &str) -> bool {
        if let Some(param) = self.params.get_mut(name) {
            param.flags |= flags::READONLY;
            true
        } else {
            false
        }
    }

    // -----------------------------------------------------------------------
    // Scope management (from startparamscope/endparamscope)
    // -----------------------------------------------------------------------

    /// Start a new local scope
    pub fn push_scope(&mut self) {
        self.local_level += 1;
    }

    /// End a local scope, restoring parameters
    pub fn pop_scope(&mut self) {
        let level = self.local_level;
        let names_to_check: Vec<String> = self.params.keys().cloned().collect();

        for name in names_to_check {
            let should_remove = {
                if let Some(param) = self.params.get(&name) {
                    param.level > level - 1
                } else {
                    false
                }
            };

            if should_remove {
                let is_special = self
                    .params
                    .get(&name)
                    .map(|p| p.is_special())
                    .unwrap_or(false);

                if is_special {
                    // Restore special parameter from old
                    let old = self.params.get(&name).and_then(|p| p.old.clone());
                    if let Some(old_param) = old {
                        let old_value = old_param.value.clone();
                        if let Some(p) = self.params.get_mut(&name) {
                            p.flags = old_param.flags;
                            p.level = old_param.level;
                            p.base = old_param.base;
                            p.width = old_param.width;
                            p.old = old_param.old;
                            if (old_param.flags & flags::NORESTORE) == 0 {
                                p.value = old_value.clone();
                                self.handle_special_set(&name, &old_value);
                            }
                        }
                    }
                } else {
                    // Remove local and restore old
                    let old = self.params.get(&name).and_then(|p| p.old.clone());
                    if let Some(old_param) = old {
                        self.params.insert(name.clone(), *old_param);
                        if let Some(p) = self.params.get(&name) {
                            if p.is_exported() {
                                env::set_var(&name, p.value.as_string());
                            }
                        }
                    } else {
                        self.params.remove(&name);
                    }
                }
            }
        }

        self.local_level -= 1;
    }

    /// Create a local variable (from typeset/local builtin)
    pub fn make_local(&mut self, name: &str) {
        if let Some(param) = self.params.get(name) {
            if param.level == self.local_level {
                // Already at this level
                return;
            }
            // Save old and create new at current level
            let old = Box::new(param.clone());
            let mut new_param = Param {
                name: name.to_string(),
                value: ParamValue::Unset,
                flags: flags::SCALAR | flags::LOCAL | flags::UNSET,
                base: 10,
                width: 0,
                level: self.local_level,
                ename: None,
                old: Some(old),
            };

            // For special params, copy the special flag
            if param.is_special() {
                new_param.flags |= flags::SPECIAL;
                new_param.value = param.value.clone();
                new_param.flags &= !flags::UNSET;
            }

            self.params.insert(name.to_string(), new_param);
        } else {
            // Create new local
            let param = Param {
                name: name.to_string(),
                value: ParamValue::Unset,
                flags: flags::SCALAR | flags::LOCAL | flags::UNSET,
                base: 10,
                width: 0,
                level: self.local_level,
                ename: None,
                old: None,
            };
            self.params.insert(name.to_string(), param);
        }
    }

    /// Create a local variable with a specific type
    pub fn make_local_typed(&mut self, name: &str, pm_flags: u32) {
        self.make_local(name);
        if let Some(param) = self.params.get_mut(name) {
            // Set type, preserve LOCAL
            param.flags =
                (param.flags & (flags::LOCAL | flags::SPECIAL | flags::EXPORT)) | pm_flags;
            // Set appropriate default value
            param.value = match flags::pm_type(pm_flags) {
                flags::INTEGER => ParamValue::Integer(0),
                flags::EFLOAT | flags::FFLOAT => ParamValue::Float(0.0),
                flags::ARRAY => ParamValue::Array(Vec::new()),
                flags::HASHED => ParamValue::Assoc(HashMap::new()),
                _ => ParamValue::Scalar(String::new()),
            };
            param.flags &= !flags::UNSET;
        }
    }

    // -----------------------------------------------------------------------
    // Create parameter (from createparam in C)
    // -----------------------------------------------------------------------

    /// Create a parameter with given flags. Returns false if already exists and set.
    pub fn createparam(&mut self, name: &str, pm_flags: u32) -> bool {
        if !isident(name) {
            return false;
        }

        if let Some(existing) = self.params.get(name) {
            if existing.level == self.local_level {
                if !existing.is_unset() && !existing.is_special() {
                    // Already exists and set at this level
                    if let Some(p) = self.params.get_mut(name) {
                        p.flags &= !flags::UNSET;
                    }
                    return false;
                }
            }
        }

        let value = match flags::pm_type(pm_flags) {
            flags::INTEGER => ParamValue::Integer(0),
            flags::EFLOAT | flags::FFLOAT => ParamValue::Float(0.0),
            flags::ARRAY => ParamValue::Array(Vec::new()),
            flags::HASHED => ParamValue::Assoc(HashMap::new()),
            flags::NAMEREF => ParamValue::Scalar(String::new()),
            _ => ParamValue::Scalar(String::new()),
        };

        let old = self.params.get(name).cloned().map(Box::new);
        let param = Param {
            name: name.to_string(),
            value,
            flags: pm_flags & !flags::LOCAL,
            base: 10,
            width: 0,
            level: if (pm_flags & flags::LOCAL) != 0 {
                self.local_level
            } else {
                0
            },
            ename: None,
            old,
        };
        self.params.insert(name.to_string(), param);
        true
    }

    /// Reset parameter to new type (from resetparam in C)
    pub fn resetparam(&mut self, name: &str, new_flags: u32) -> bool {
        if let Some(param) = self.params.get(name) {
            if param.is_readonly() {
                return false;
            }
        }
        // Unset and recreate
        let exported = self
            .params
            .get(name)
            .map(|p| p.flags & flags::EXPORT)
            .unwrap_or(0);
        self.unset(name);
        self.createparam(name, new_flags | exported);
        true
    }

    // -----------------------------------------------------------------------
    // Named reference support (from resolve_nameref etc.)
    // -----------------------------------------------------------------------

    /// Create a named reference
    pub fn set_nameref(&mut self, name: &str, target: &str) -> bool {
        if !isident(name) || !valid_refname(target) {
            return false;
        }
        // Don't allow self-reference
        if name == target {
            return false;
        }

        let level = self.local_level;
        let old = self.params.get(name).cloned().map(Box::new);
        let param = Param {
            name: name.to_string(),
            value: ParamValue::Scalar(target.to_string()),
            flags: flags::NAMEREF,
            base: 0,
            width: 0,
            level,
            ename: None,
            old,
        };
        self.params.insert(name.to_string(), param);
        true
    }

    /// Resolve a nameref to its ultimate target Param
    pub fn resolve_nameref<'a>(&'a self, name: &str) -> Option<&'a Param> {
        if let Some(target) = self.resolve_nameref_name(name) {
            self.params.get(&target)
        } else {
            self.params.get(name)
        }
    }

    /// Set loop variable (for-loop nameref support)
    pub fn set_loop_var(&mut self, name: &str, value: &str) {
        if let Some(param) = self.params.get(name) {
            if param.is_nameref() {
                if param.is_readonly() {
                    return;
                }
                // Update the nameref target
                if let Some(p) = self.params.get_mut(name) {
                    p.value = ParamValue::Scalar(value.to_string());
                    p.flags &= !flags::UNSET;
                }
                return;
            }
        }
        self.set_scalar(name, value);
    }

    // -----------------------------------------------------------------------
    // Tied parameter support
    // -----------------------------------------------------------------------

    /// Tie scalar to array with separator (from typeset -T)
    pub fn tie_param(&mut self, scalar: &str, array: &str, sep: char) {
        // Get current value from scalar
        let current = self
            .get_value(scalar)
            .map(|v| v.as_string())
            .unwrap_or_default();

        let arr: Vec<String> = current
            .split(sep)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();

        // Create/update scalar
        if !self.params.contains_key(scalar) {
            let mut param = Param::new_scalar(scalar, &current);
            param.flags |= flags::TIED;
            param.ename = Some(array.to_string());
            self.params.insert(scalar.to_string(), param);
        } else if let Some(p) = self.params.get_mut(scalar) {
            p.flags |= flags::TIED;
            p.ename = Some(array.to_string());
        }

        // Create/update array
        let arr_param = Param {
            name: array.to_string(),
            value: ParamValue::Array(arr),
            flags: flags::ARRAY | flags::TIED,
            base: 10,
            width: 0,
            level: 0,
            ename: Some(scalar.to_string()),
            old: None,
        };
        self.params.insert(array.to_string(), arr_param);

        self.tied.insert(
            scalar.to_string(),
            TiedData {
                join_char: sep,
                scalar_name: scalar.to_string(),
                array_name: array.to_string(),
            },
        );
        self.tied.insert(
            array.to_string(),
            TiedData {
                join_char: sep,
                scalar_name: scalar.to_string(),
                array_name: array.to_string(),
            },
        );
    }

    /// Untie a parameter pair
    pub fn untie_param(&mut self, name: &str) {
        if let Some(tied) = self.tied.remove(name) {
            let other = if name == tied.scalar_name {
                &tied.array_name
            } else {
                &tied.scalar_name
            };
            self.tied.remove(other);

            if let Some(p) = self.params.get_mut(name) {
                p.flags &= !flags::TIED;
                p.ename = None;
            }
            if let Some(p) = self.params.get_mut(other) {
                p.flags &= !flags::TIED;
                p.ename = None;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Array/hash element access
    // -----------------------------------------------------------------------

    /// Set array element by index (1-based, zsh style)
    pub fn set_array_element(&mut self, name: &str, index: i64, value: &str) -> bool {
        if let Some(param) = self.params.get_mut(name) {
            if param.is_readonly() {
                return false;
            }
            if let ParamValue::Array(ref mut arr) = param.value {
                let len = arr.len() as i64;
                let idx = if index < 0 { len + index + 1 } else { index };
                if idx < 1 {
                    return false;
                }
                let idx = (idx - 1) as usize;
                while arr.len() <= idx {
                    arr.push(String::new());
                }
                arr[idx] = value.to_string();
                let pv = ParamValue::Array(arr.clone());
                self.handle_special_set(name, &pv);
                return true;
            }
        }
        false
    }

    /// Get array element by index (1-based, zsh style)
    pub fn get_array_element(&self, name: &str, index: i64) -> Option<String> {
        if let Some(param) = self.params.get(name) {
            if let ParamValue::Array(ref arr) = param.value {
                let len = arr.len() as i64;
                let idx = if index < 0 { len + index + 1 } else { index };
                if idx < 1 || idx > len {
                    return None;
                }
                return Some(arr[(idx - 1) as usize].clone());
            }
        }
        None
    }

    /// Set associative array element
    pub fn set_hash_element(&mut self, name: &str, key: &str, value: &str) -> bool {
        if let Some(param) = self.params.get_mut(name) {
            if param.is_readonly() {
                return false;
            }
            if let ParamValue::Assoc(ref mut hash) = param.value {
                hash.insert(key.to_string(), value.to_string());
                return true;
            }
        }
        false
    }

    /// Get associative array element
    pub fn get_hash_element(&self, name: &str, key: &str) -> Option<String> {
        if let Some(param) = self.params.get(name) {
            if let ParamValue::Assoc(ref hash) = param.value {
                return hash.get(key).cloned();
            }
        }
        None
    }

    /// Delete associative array element
    pub fn unset_hash_element(&mut self, name: &str, key: &str) -> bool {
        if let Some(param) = self.params.get_mut(name) {
            if param.is_readonly() {
                return false;
            }
            if let ParamValue::Assoc(ref mut hash) = param.value {
                return hash.remove(key).is_some();
            }
        }
        false
    }

    /// Get all keys from associative array
    pub fn get_hash_keys(&self, name: &str) -> Vec<String> {
        if let Some(param) = self.params.get(name) {
            if let ParamValue::Assoc(ref hash) = param.value {
                return hash.keys().cloned().collect();
            }
        }
        Vec::new()
    }

    /// Get all values from associative array
    pub fn get_hash_values(&self, name: &str) -> Vec<String> {
        if let Some(param) = self.params.get(name) {
            if let ParamValue::Assoc(ref hash) = param.value {
                return hash.values().cloned().collect();
            }
        }
        Vec::new()
    }

    // -----------------------------------------------------------------------
    // Array slice operations (from getarrvalue/setarrvalue)
    // -----------------------------------------------------------------------

    /// Get array slice with subscript handling
    pub fn get_array_slice(&self, name: &str, start: i64, end: i64) -> Vec<String> {
        if let Some(param) = self.params.get(name) {
            if let ParamValue::Array(ref arr) = param.value {
                return getarrvalue(arr, start, end);
            }
        }
        Vec::new()
    }

    /// Set array slice with subscript handling
    pub fn set_array_slice(&mut self, name: &str, start: i64, end: i64, val: Vec<String>) -> bool {
        if let Some(param) = self.params.get_mut(name) {
            if param.is_readonly() {
                return false;
            }
            if let ParamValue::Array(ref mut arr) = param.value {
                setarrvalue(arr, start, end, val);
                let pv = ParamValue::Array(arr.clone());
                self.handle_special_set(name, &pv);
                return true;
            }
        }
        false
    }

    /// Get string slice
    pub fn get_str_slice(&self, name: &str, start: i64, end: i64) -> String {
        let val = self
            .get_value(name)
            .map(|v| v.as_string())
            .unwrap_or_default();
        let len = val.len() as i64;

        let start = if start < 0 {
            (len + start).max(0) as usize
        } else {
            start.max(0) as usize
        };
        let end = if end < 0 {
            (len + end + 1).max(0) as usize
        } else {
            end.min(len) as usize
        };

        if start >= val.len() || start >= end {
            return String::new();
        }
        val[start..end.min(val.len())].to_string()
    }

    /// Set string slice
    pub fn set_str_slice(&mut self, name: &str, start: i64, end: i64, val: &str) -> bool {
        let current = self
            .get_value(name)
            .map(|v| v.as_string())
            .unwrap_or_default();
        let len = current.len() as i64;

        let s = if start < 0 {
            (len + start).max(0) as usize
        } else {
            start as usize
        };
        let e = if end < 0 {
            (len + end + 1).max(0) as usize
        } else {
            end as usize
        };
        let s = s.min(current.len());
        let e = e.min(current.len());

        let mut result = String::with_capacity(s + val.len() + current.len() - e);
        result.push_str(&current[..s]);
        result.push_str(val);
        if e < current.len() {
            result.push_str(&current[e..]);
        }
        self.set_scalar(name, &result)
    }

    // -----------------------------------------------------------------------
    // Environment operations
    // -----------------------------------------------------------------------

    /// Export parameter to environment (full version from export_param)
    pub fn export_param(&mut self, name: &str) {
        if let Some(param) = self.params.get_mut(name) {
            param.flags |= flags::EXPORT;
            let val = match flags::pm_type(param.flags) {
                flags::ARRAY | flags::HASHED => return, // Can't export arrays
                flags::INTEGER => convbase(param.value.as_integer(), param.base as u32),
                flags::EFLOAT | flags::FFLOAT => {
                    format_float(param.value.as_float(), param.base, param.flags)
                }
                _ => param.value.as_string(),
            };
            env::set_var(name, &val);
        }
    }

    /// Fix environment after array change (from arrfixenv)
    pub fn arr_fix_env(&mut self, name: &str) {
        if let Some(tied) = self.tied.get(name).cloned() {
            if name == tied.array_name {
                let arr = self
                    .params
                    .get(name)
                    .map(|p| p.value.as_array())
                    .unwrap_or_default();
                let joined = arr.join(&tied.join_char.to_string());
                if let Some(p) = self.params.get(&tied.scalar_name) {
                    if p.is_exported() {
                        env::set_var(&tied.scalar_name, &joined);
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Scanning / iteration
    // -----------------------------------------------------------------------

    /// Iterate over all parameters
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Param)> {
        self.params.iter()
    }

    /// Check if a parameter exists (and is set)
    pub fn contains(&self, name: &str) -> bool {
        self.params
            .get(name)
            .map(|p| !p.is_unset())
            .unwrap_or(false)
    }

    /// Get parameter count
    pub fn len(&self) -> usize {
        self.params.values().filter(|p| !p.is_unset()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Scan parameters matching pattern with optional flag filter
    pub fn scan_match<F>(&self, pattern: &str, flag_filter: u32, mut callback: F)
    where
        F: FnMut(&str, &Param),
    {
        for (name, param) in &self.params {
            if param.is_unset() {
                continue;
            }
            if flag_filter != 0 && (param.flags & flag_filter) == 0 {
                continue;
            }
            if pattern.is_empty() || glob_match(pattern, name) {
                callback(name, param);
            }
        }
    }

    /// Get all parameter names matching pattern
    pub fn paramnames(&self, pattern: Option<&str>) -> Vec<String> {
        let mut names: Vec<String> = self
            .params
            .iter()
            .filter(|(_, p)| !p.is_unset())
            .filter(|(name, _)| pattern.map_or(true, |p| glob_match(p, name)))
            .map(|(name, _)| name.clone())
            .collect();
        names.sort();
        names
    }

    // -----------------------------------------------------------------------
    // Parameter printing (from printparamnode)
    // -----------------------------------------------------------------------

    /// Format a parameter for display (typeset -p output)
    pub fn format_param(&self, name: &str, pf: u32) -> Option<String> {
        let param = self.params.get(name)?;
        if param.is_unset()
            && (pf & print_flags::POSIX_READONLY) == 0
            && (pf & print_flags::POSIX_EXPORT) == 0
        {
            return None;
        }

        let mut out = String::new();

        if (pf & (print_flags::TYPESET | print_flags::POSIX_READONLY | print_flags::POSIX_EXPORT))
            != 0
        {
            if (pf & print_flags::POSIX_EXPORT) != 0 {
                if (param.flags & flags::EXPORT) == 0 {
                    return None;
                }
                out.push_str("export ");
            } else if (pf & print_flags::POSIX_READONLY) != 0 {
                if (param.flags & flags::READONLY) == 0 {
                    return None;
                }
                out.push_str("readonly ");
            } else if (param.flags & flags::EXPORT) != 0
                && (param.flags & (flags::ARRAY | flags::HASHED)) == 0
            {
                out.push_str("export ");
            } else if self.local_level > 0 && param.level >= self.local_level {
                out.push_str("typeset ");
            } else {
                out.push_str("typeset ");
            }
        }

        // Print type flags
        if (pf & (print_flags::TYPE | print_flags::TYPESET)) != 0 {
            let mut flag_chars = String::new();
            for pmt in PM_TYPES {
                if pmt.test_level {
                    if param.level > 0 {
                        // local
                    }
                    continue;
                }
                if pmt.bin_flag != 0 && (param.flags & pmt.bin_flag) != 0 {
                    if (pf & print_flags::TYPESET) != 0 && pmt.type_flag != '\0' {
                        flag_chars.push(pmt.type_flag);
                    } else if (pf & print_flags::TYPE) != 0 {
                        out.push_str(pmt.string);
                        out.push(' ');
                    }
                }
            }
            if !flag_chars.is_empty() {
                out.push('-');
                out.push_str(&flag_chars);
                out.push(' ');
            }
        }

        // Print name and value
        out.push_str(&param.name);

        if (pf & print_flags::NAMEONLY) == 0 && (param.flags & flags::HIDEVAL) == 0 {
            out.push('=');
            match &param.value {
                ParamValue::Scalar(s) => {
                    out.push_str(&shell_quote(s));
                }
                ParamValue::Integer(i) => {
                    out.push_str(&convbase(*i, param.base as u32));
                }
                ParamValue::Float(f) => {
                    out.push_str(&format_float(*f, param.base, param.flags));
                }
                ParamValue::Array(arr) => {
                    out.push('(');
                    for (i, elem) in arr.iter().enumerate() {
                        if i > 0 {
                            out.push(' ');
                        }
                        out.push_str(&shell_quote(elem));
                    }
                    out.push(')');
                }
                ParamValue::Assoc(hash) => {
                    out.push('(');
                    let mut pairs: Vec<_> = hash.iter().collect();
                    pairs.sort_by_key(|(k, _)| (*k).clone());
                    for (i, (k, v)) in pairs.iter().enumerate() {
                        if i > 0 {
                            out.push(' ');
                        }
                        out.push('[');
                        out.push_str(&shell_quote(k));
                        out.push_str("]=");
                        out.push_str(&shell_quote(v));
                    }
                    out.push(')');
                }
                ParamValue::Unset => {}
            }
        }

        Some(out)
    }

    /// Get parameter type string (from getparamtype)
    pub fn getparamtype(&self, name: &str) -> &'static str {
        if let Some(param) = self.params.get(name) {
            match flags::pm_type(param.flags) {
                flags::HASHED => "association",
                flags::ARRAY => "array",
                flags::INTEGER => "integer",
                flags::EFLOAT | flags::FFLOAT => "float",
                flags::NAMEREF => "nameref",
                _ => "scalar",
            }
        } else {
            ""
        }
    }

    /// Check if parameter is set (from issetvar)
    pub fn issetvar(&self, name: &str) -> bool {
        self.params
            .get(name)
            .map(|p| !p.is_unset())
            .unwrap_or(false)
    }

    /// Get array length (from arrlen)
    pub fn arrlen(&self, name: &str) -> usize {
        if let Some(param) = self.params.get(name) {
            match &param.value {
                ParamValue::Array(arr) => arr.len(),
                ParamValue::Assoc(hash) => hash.len(),
                ParamValue::Scalar(s) if s.is_empty() => 0,
                ParamValue::Scalar(_) => 1,
                ParamValue::Unset => 0,
                _ => 1,
            }
        } else {
            0
        }
    }

    /// Check if parameter is an array
    pub fn isarray(&self, name: &str) -> bool {
        self.params.get(name).map(|p| p.is_array()).unwrap_or(false)
    }

    /// Check if parameter is a hash
    pub fn ishash(&self, name: &str) -> bool {
        self.params.get(name).map(|p| p.is_assoc()).unwrap_or(false)
    }

    /// Copy a parameter value
    pub fn copyparam(&self, name: &str) -> Option<ParamValue> {
        self.params.get(name).map(|p| p.value.clone())
    }
}

// ---------------------------------------------------------------------------
// Free functions matching the C API
// ---------------------------------------------------------------------------

/// Get integer parameter value (from params.c getiparam)
pub fn getiparam(table: &ParamTable, name: &str) -> i64 {
    table.get_value(name).map(|v| v.as_integer()).unwrap_or(0)
}

/// Get scalar (string) parameter (from params.c getsparam)
pub fn getsparam(table: &ParamTable, name: &str) -> Option<String> {
    table.get_value(name).map(|v| v.as_string())
}

/// Get scalar with default
pub fn getsparam_u(table: &ParamTable, name: &str, default: &str) -> String {
    getsparam(table, name).unwrap_or_else(|| default.to_string())
}

/// Get array parameter (from params.c getaparam)
pub fn getaparam(table: &ParamTable, name: &str) -> Option<Vec<String>> {
    match table.get_value(name)? {
        ParamValue::Array(arr) => Some(arr),
        _ => None,
    }
}

/// Get hash parameter values as array (from params.c gethparam)
pub fn gethparam(table: &ParamTable, name: &str) -> Option<Vec<String>> {
    match table.get_value(name)? {
        ParamValue::Assoc(h) => Some(h.values().cloned().collect()),
        _ => None,
    }
}

/// Get hash parameter keys as array (from params.c gethkparam)
pub fn gethkparam(table: &ParamTable, name: &str) -> Option<Vec<String>> {
    match table.get_value(name)? {
        ParamValue::Assoc(h) => Some(h.keys().cloned().collect()),
        _ => None,
    }
}

/// Get numeric parameter (from params.c getnparam)
pub fn getnparam(table: &ParamTable, name: &str) -> MNumber {
    match table.get_value(name) {
        Some(ParamValue::Integer(i)) => MNumber::Integer(i),
        Some(ParamValue::Float(f)) => MNumber::Float(f),
        Some(ParamValue::Scalar(s)) => {
            if let Ok(i) = s.parse::<i64>() {
                MNumber::Integer(i)
            } else if let Ok(f) = s.parse::<f64>() {
                MNumber::Float(f)
            } else {
                MNumber::default()
            }
        }
        _ => MNumber::default(),
    }
}

/// Assign string parameter (from params.c assignsparam)
pub fn assignsparam(table: &mut ParamTable, name: &str, val: &str) -> bool {
    table.set_scalar(name, val)
}

/// Assign integer parameter (from params.c assigniparam)
pub fn assigniparam(table: &mut ParamTable, name: &str, val: i64) -> bool {
    table.set_integer(name, val)
}

/// Assign array parameter (from params.c assignaparam)
pub fn assignaparam(table: &mut ParamTable, name: &str, val: Vec<String>) -> bool {
    table.set_array(name, val)
}

/// Assign float parameter
pub fn assignfparam(table: &mut ParamTable, name: &str, val: f64) -> bool {
    table.set_float(name, val)
}

/// Assign hash parameter (from params.c sethparam)
pub fn assignhparam(table: &mut ParamTable, name: &str, val: HashMap<String, String>) -> bool {
    table.set_assoc(name, val)
}

/// Unset parameter (from params.c unsetparam)
pub fn unsetparam(table: &mut ParamTable, name: &str) -> bool {
    table.unset(name)
}

/// Check if parameter is set
pub fn isset_param(table: &ParamTable, name: &str) -> bool {
    table.contains(name)
}

/// Get parameter type flags
pub fn paramtype(table: &ParamTable, name: &str) -> u32 {
    table.params.get(name).map(|p| p.flags).unwrap_or(0)
}

/// Check if parameter is exported
pub fn isexported(table: &ParamTable, name: &str) -> bool {
    table
        .params
        .get(name)
        .map(|p| p.is_exported())
        .unwrap_or(false)
}

/// Check if parameter is readonly
pub fn isreadonly(table: &ParamTable, name: &str) -> bool {
    table
        .params
        .get(name)
        .map(|p| p.is_readonly())
        .unwrap_or(false)
}

/// Export parameter to environment
pub fn export_param(table: &mut ParamTable, name: &str) {
    table.export_param(name);
}

/// Unexport parameter
pub fn unexport_param(table: &mut ParamTable, name: &str) {
    table.unexport(name);
}

/// Start a parameter scope
pub fn startparamscope(table: &mut ParamTable) {
    table.push_scope();
}

/// End a parameter scope
pub fn endparamscope(table: &mut ParamTable) {
    table.pop_scope();
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

/// Check if string is valid identifier (from params.c isident)
pub fn isident(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut chars = s.chars().peekable();

    // Handle namespace prefix (e.g. "ns.var")
    if chars.peek() == Some(&'.') {
        chars.next();
        if chars.peek().map_or(true, |c| c.is_ascii_digit()) {
            return false;
        }
    }

    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };

    if first.is_ascii_digit() {
        // All-digit names are valid (positional params)
        return chars.all(|c| c.is_ascii_digit());
    }

    if !first.is_alphabetic() && first != '_' {
        return false;
    }

    for c in chars {
        if c == '[' {
            // Subscript is OK at end
            return true;
        }
        if !c.is_alphanumeric() && c != '_' && c != '.' {
            return false;
        }
    }
    true
}

/// Validate nameref target name (from valid_refname)
pub fn valid_refname(val: &str) -> bool {
    if val.is_empty() {
        return false;
    }
    let first = val.chars().next().unwrap();
    if first.is_ascii_digit() {
        // All digits OK for positional params
        let rest = &val[1..];
        if let Some(bracket_pos) = rest.find('[') {
            return rest[..bracket_pos].chars().all(|c| c.is_ascii_digit());
        }
        return rest.chars().all(|c| c.is_ascii_digit());
    }
    if first == '!' || first == '?' || first == '$' || first == '-' {
        return val.len() == 1 || val.as_bytes().get(1) == Some(&b'[');
    }
    if !first.is_alphabetic() && first != '_' {
        return false;
    }
    for c in val[1..].chars() {
        if c == '[' {
            return true; // Subscript is fine
        }
        if !c.is_alphanumeric() && c != '_' && c != '.' {
            return false;
        }
    }
    true
}

/// Colon-separated path to array
pub fn colonarr_to_array(s: &str) -> Vec<String> {
    s.split(':')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// Array to colon-separated path
pub fn array_to_colonarr(arr: &[String]) -> String {
    arr.join(":")
}

/// Remove duplicate elements from array while preserving order
pub fn uniq_array(arr: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    arr.into_iter().filter(|s| seen.insert(s.clone())).collect()
}

/// Parse a subscript expression like "[1]", "[1,5]", "[@]", "[*]"
pub fn parse_subscript(subscript: &str, ksh_arrays: bool) -> Option<SubscriptIndex> {
    let s = subscript.trim();

    if s == "@" || s == "*" {
        return Some(SubscriptIndex::all());
    }

    if let Some(comma_pos) = s.find(',') {
        let start_str = s[..comma_pos].trim();
        let end_str = s[comma_pos + 1..].trim();
        let start = parse_index_value(start_str, ksh_arrays)?;
        let end = parse_index_value(end_str, ksh_arrays)?;
        return Some(SubscriptIndex::range(start, end));
    }

    let idx = parse_index_value(s, ksh_arrays)?;
    Some(SubscriptIndex::single(idx))
}

fn parse_index_value(s: &str, _ksh_arrays: bool) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    s.parse::<i64>().ok()
}

/// Parse simple subscript - extract index from [n] or [m,n] syntax
pub fn parse_simple_subscript(s: &str) -> Option<(i64, i64)> {
    let s = s.trim();
    if !s.starts_with('[') || !s.ends_with(']') {
        return None;
    }
    let inner = &s[1..s.len() - 1];
    if let Some(comma) = inner.find(',') {
        let start = inner[..comma].trim().parse::<i64>().ok()?;
        let end = inner[comma + 1..].trim().parse::<i64>().ok()?;
        Some((start, end))
    } else {
        let idx = inner.trim().parse::<i64>().ok()?;
        Some((idx, idx))
    }
}

/// Get array element with subscript handling (from params.c getarrvalue)
pub fn getarrvalue(arr: &[String], start: i64, end: i64) -> Vec<String> {
    let len = arr.len() as i64;
    if len == 0 {
        return Vec::new();
    }
    let start = if start < 0 { len + start + 1 } else { start };
    let end = if end < 0 { len + end + 1 } else { end };
    let start = (start.max(1) - 1) as usize;
    let end = end.min(len) as usize;
    if start >= end || start >= arr.len() {
        return Vec::new();
    }
    arr[start..end].to_vec()
}

/// Set array element with subscript handling (from params.c setarrvalue)
pub fn setarrvalue(arr: &mut Vec<String>, start: i64, end: i64, val: Vec<String>) {
    let len = arr.len() as i64;
    let start = if start < 0 {
        (len + start + 1).max(0)
    } else {
        start
    };
    let end = if end < 0 { (len + end + 1).max(0) } else { end };
    let start = (start.max(1) - 1) as usize;
    let end = end.max(0) as usize;

    while arr.len() < start {
        arr.push(String::new());
    }

    let end = end.min(arr.len());
    if start <= end {
        arr.splice(start..end, val);
    } else {
        for (i, v) in val.into_iter().enumerate() {
            if start + i < arr.len() {
                arr[start + i] = v;
            } else {
                arr.push(v);
            }
        }
    }
}

/// Get single array element by index (handles ksh_arrays)
pub fn get_array_element(arr: &[String], idx: i64, ksh_arrays: bool) -> Option<String> {
    let len = arr.len() as i64;
    let actual_idx = if idx < 0 {
        let adj = len + idx;
        if adj < 0 {
            return None;
        }
        adj as usize
    } else if ksh_arrays {
        idx as usize
    } else {
        if idx > 0 {
            (idx - 1) as usize
        } else {
            return None;
        }
    };
    arr.get(actual_idx).cloned()
}

/// Get array slice based on subscript index
pub fn get_array_slice(arr: &[String], idx: &SubscriptIndex, ksh_arrays: bool) -> Vec<String> {
    if idx.is_all {
        return arr.to_vec();
    }
    let len = arr.len() as i64;
    let start = if idx.start < 0 {
        (len + idx.start).max(0) as usize
    } else if ksh_arrays {
        idx.start as usize
    } else {
        if idx.start > 0 {
            (idx.start - 1) as usize
        } else {
            0
        }
    };
    let end = if idx.end < 0 {
        ((len + idx.end + 1).max(0) as usize).min(arr.len())
    } else if ksh_arrays {
        (idx.end as usize).min(arr.len())
    } else {
        (idx.end as usize).min(arr.len())
    };
    if start >= arr.len() || start >= end {
        return Vec::new();
    }
    arr[start..end].to_vec()
}

/// Simple glob match for parameter scanning
fn glob_match(pattern: &str, name: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if pattern.ends_with('*') && !pattern[..pattern.len() - 1].contains('*') {
        return name.starts_with(&pattern[..pattern.len() - 1]);
    }
    if pattern.starts_with('*') && !pattern[1..].contains('*') {
        return name.ends_with(&pattern[1..]);
    }
    // Simple two-star case: *foo*
    if pattern.starts_with('*') && pattern.ends_with('*') && pattern.len() > 2 {
        let inner = &pattern[1..pattern.len() - 1];
        if !inner.contains('*') {
            return name.contains(inner);
        }
    }
    pattern == name
}

/// Shell-quote a string for display
fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    // Check if quoting is needed
    if s.chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '/' || c == '.' || c == '-' || c == ':')
    {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

// ---------------------------------------------------------------------------
// Integer/Float conversion (from convbase/convfloat)
// ---------------------------------------------------------------------------

/// Convert integer to string with base (from params.c convbase)
pub fn convbase(val: i64, base: u32) -> String {
    if base == 0 || base == 10 {
        return val.to_string();
    }

    let negative = val < 0;
    let mut v = if negative { (-val) as u64 } else { val as u64 };

    if v == 0 {
        return match base {
            16 => "0x0".to_string(),
            8 => "00".to_string(),
            _ => format!("{}#0", base),
        };
    }

    let mut digits = Vec::new();
    while v > 0 {
        let dig = (v % base as u64) as u8;
        digits.push(if dig < 10 {
            b'0' + dig
        } else {
            b'A' + dig - 10
        });
        v /= base as u64;
    }
    digits.reverse();

    let prefix = match base {
        16 => "0x",
        8 => "0",
        10 => "",
        _ => "",
    };

    let base_prefix = if base != 10 && base != 16 && base != 8 {
        format!("{}#", base)
    } else {
        prefix.to_string()
    };

    let sign = if negative { "-" } else { "" };
    format!(
        "{}{}{}",
        sign,
        base_prefix,
        String::from_utf8_lossy(&digits)
    )
}

/// Convert integer to string with underscores for readability
pub fn convbase_underscore(val: i64, base: u32, underscore: i32) -> String {
    let s = convbase(val, base);
    if underscore <= 0 {
        return s;
    }

    // Find the digits portion
    let (prefix, digits) = if s.starts_with('-') {
        let rest = &s[1..];
        let digit_start = rest
            .find(|c: char| c.is_ascii_digit() || c.is_ascii_uppercase())
            .unwrap_or(0);
        (&s[..1 + digit_start], &rest[digit_start..])
    } else {
        let digit_start = s
            .find(|c: char| c.is_ascii_digit() || c.is_ascii_uppercase())
            .unwrap_or(0);
        (&s[..digit_start], &s[digit_start..])
    };

    if digits.len() <= underscore as usize {
        return s;
    }

    let u = underscore as usize;
    let mut result = prefix.to_string();
    let chars: Vec<char> = digits.chars().collect();
    let first_group = chars.len() % u;
    if first_group > 0 {
        result.extend(&chars[..first_group]);
        if first_group < chars.len() {
            result.push('_');
        }
    }
    for (i, chunk) in chars[first_group..].chunks(u).enumerate() {
        if i > 0 {
            result.push('_');
        }
        result.extend(chunk);
    }
    result
}

/// Format a float value for output (from params.c convfloat)
pub fn format_float(dval: f64, digits: i32, pm_flags: u32) -> String {
    if dval.is_infinite() {
        return if dval < 0.0 {
            "-Inf".to_string()
        } else {
            "Inf".to_string()
        };
    }
    if dval.is_nan() {
        return "NaN".to_string();
    }

    let digits = if digits <= 0 { 10 } else { digits as usize };

    if (pm_flags & flags::EFLOAT) != 0 {
        format!("{:.*e}", digits.saturating_sub(1), dval)
    } else if (pm_flags & flags::FFLOAT) != 0 {
        format!("{:.*}", digits, dval)
    } else {
        // General format
        let s = format!("{:.*}", 17, dval);
        // Ensure there's a decimal point
        if !s.contains('.') && !s.contains('e') {
            format!("{}.", s)
        } else {
            s
        }
    }
}

/// Format float with underscores
pub fn convfloat_underscore(dval: f64, underscore: i32) -> String {
    let s = format_float(dval, 0, 0);
    if underscore <= 0 {
        return s;
    }

    let u = underscore as usize;
    let (sign, rest) = if s.starts_with('-') {
        ("-", &s[1..])
    } else {
        ("", s.as_str())
    };

    let (int_part, frac_exp) = if let Some(dot_pos) = rest.find('.') {
        (&rest[..dot_pos], &rest[dot_pos..])
    } else {
        (rest, "")
    };

    // Add underscores to integer part
    let int_chars: Vec<char> = int_part.chars().collect();
    let mut result = sign.to_string();
    let first_group = int_chars.len() % u;
    if first_group > 0 {
        result.extend(&int_chars[..first_group]);
        if first_group < int_chars.len() {
            result.push('_');
        }
    }
    for (i, chunk) in int_chars[first_group..].chunks(u).enumerate() {
        if i > 0 {
            result.push('_');
        }
        result.extend(chunk);
    }

    // Add underscores to fractional part
    if frac_exp.starts_with('.') {
        result.push('.');
        let frac = &frac_exp[1..];
        let (frac_digits, exp) = if let Some(e_pos) = frac.find('e') {
            (&frac[..e_pos], &frac[e_pos..])
        } else {
            (frac, "")
        };

        let frac_chars: Vec<char> = frac_digits.chars().collect();
        for (i, chunk) in frac_chars.chunks(u).enumerate() {
            if i > 0 {
                result.push('_');
            }
            result.extend(chunk);
        }
        result.push_str(exp);
    } else {
        result.push_str(frac_exp);
    }

    result
}

/// Integer parameter with base formatting (from params.c intgetfn)
pub fn intgetfn(table: &ParamTable, name: &str, base: u32) -> String {
    let val = getiparam(table, name);
    convbase(val, base)
}

/// String parameter with modifiers (from params.c strgetfn)
pub fn strgetfn(table: &ParamTable, name: &str, lower: bool, upper: bool) -> Option<String> {
    let val = getsparam(table, name)?;
    Some(if lower {
        val.to_lowercase()
    } else if upper {
        val.to_uppercase()
    } else {
        val
    })
}

// ---------------------------------------------------------------------------
// Subscript flag parsing (from getarg subscription flags)
// ---------------------------------------------------------------------------

/// Parse subscription flags from (flags) prefix
pub fn parse_subscription_flags(s: &str) -> (SubscriptFlags, &str) {
    let mut flags = SubscriptFlags::default();
    flags.num = 1;

    if !s.starts_with('(') {
        return (flags, s);
    }

    let mut chars = s[1..].char_indices();
    let mut end_pos = 0;

    while let Some((pos, c)) = chars.next() {
        match c {
            ')' => {
                end_pos = pos + 2; // +1 for '(' offset, +1 for ')'
                break;
            }
            'r' => {
                flags.reverse = true;
                flags.down = false;
                flags.index = false;
                flags.key_match = false;
            }
            'R' => {
                flags.reverse = true;
                flags.down = true;
                flags.index = false;
                flags.key_match = false;
            }
            'k' => {
                flags.key_match = true;
                flags.reverse = true;
                flags.down = false;
                flags.index = false;
            }
            'K' => {
                flags.key_match = true;
                flags.reverse = true;
                flags.down = true;
                flags.index = false;
            }
            'i' => {
                flags.reverse = true;
                flags.index = true;
                flags.down = false;
                flags.key_match = false;
            }
            'I' => {
                flags.reverse = true;
                flags.index = true;
                flags.down = true;
                flags.key_match = false;
            }
            'w' => {
                flags.word = true;
            }
            'f' => {
                flags.word = true;
                flags.separator = Some("\n".to_string());
            }
            'e' => {
                flags.quote_arg = true;
            }
            _ => {}
        }
    }

    if end_pos > 0 && end_pos <= s.len() {
        (flags, &s[end_pos..])
    } else {
        (flags, s)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_param_value_conversions() {
        let scalar = ParamValue::Scalar("42".to_string());
        assert_eq!(scalar.as_integer(), 42);
        assert_eq!(scalar.as_float(), 42.0);
        assert_eq!(scalar.as_string(), "42");
    }

    #[test]
    fn test_param_table_set_get() {
        let mut table = ParamTable::new();
        table.set_scalar("FOO", "bar");
        assert_eq!(table.get_value("FOO").unwrap().as_string(), "bar");
    }

    #[test]
    fn test_param_readonly() {
        let mut table = ParamTable::new();
        table.set_scalar("TEST", "value");
        table.set_readonly("TEST");
        assert!(!table.set_scalar("TEST", "new_value"));
        assert_eq!(table.get_value("TEST").unwrap().as_string(), "value");
    }

    #[test]
    fn test_param_array() {
        let mut table = ParamTable::new();
        table.set_array("arr", vec!["a".into(), "b".into(), "c".into()]);
        assert_eq!(
            table.get_value("arr").unwrap().as_array(),
            vec!["a", "b", "c"]
        );
    }

    #[test]
    fn test_param_assoc() {
        let mut table = ParamTable::new();
        let mut hash = HashMap::new();
        hash.insert("key".to_string(), "value".to_string());
        table.set_assoc("hash", hash);
        if let ParamValue::Assoc(h) = table.get_value("hash").unwrap() {
            assert_eq!(h.get("key"), Some(&"value".to_string()));
        } else {
            panic!("Expected associative array");
        }
    }

    #[test]
    fn test_colonarr_conversion() {
        let arr = colonarr_to_array("/bin:/usr/bin:/usr/local/bin");
        assert_eq!(arr, vec!["/bin", "/usr/bin", "/usr/local/bin"]);
        let path = array_to_colonarr(&arr);
        assert_eq!(path, "/bin:/usr/bin:/usr/local/bin");
    }

    #[test]
    fn test_local_scope() {
        let mut table = ParamTable::new();
        table.set_scalar("GLOBAL", "value");

        table.push_scope();
        table.make_local("LOCAL_VAR");
        table.set_scalar("LOCAL_VAR", "local_value");
        assert!(table.contains("LOCAL_VAR"));

        table.pop_scope();
        assert!(!table.contains("LOCAL_VAR"));
        assert!(table.contains("GLOBAL"));
    }

    #[test]
    fn test_special_params() {
        let table = ParamTable::new();
        // $$ should be the PID
        let pid = table.get_value("$").unwrap().as_integer();
        assert!(pid > 0);

        // SHLVL should be at least 1
        let shlvl = table.get_value("SHLVL").unwrap().as_integer();
        assert!(shlvl >= 1);
    }

    #[test]
    fn test_isident() {
        assert!(isident("foo"));
        assert!(isident("_bar"));
        assert!(isident("FOO_BAR"));
        assert!(isident("x123"));
        assert!(isident("123")); // positional params
        assert!(!isident(""));
        assert!(!isident("foo bar"));
    }

    #[test]
    fn test_nameref() {
        let mut table = ParamTable::new();
        table.set_scalar("target", "hello");
        table.set_nameref("ref", "target");

        // Getting through nameref should resolve
        let val = table.get_value("ref").unwrap();
        assert_eq!(val.as_string(), "hello");
    }

    #[test]
    fn test_tied_params() {
        let mut table = ParamTable::new();
        table.tie_param("MY_PATH", "my_path", ':');
        table.set_scalar("MY_PATH", "/bin:/usr/bin");

        // Array should be synced
        let arr = table.get_value("my_path").unwrap().as_array();
        assert_eq!(arr, vec!["/bin", "/usr/bin"]);
    }

    #[test]
    fn test_unique_array() {
        let arr = vec!["a".into(), "b".into(), "a".into(), "c".into(), "b".into()];
        let result = uniq_array(arr);
        assert_eq!(result, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_convbase() {
        assert_eq!(convbase(255, 16), "0xFF");
        assert_eq!(convbase(10, 10), "10");
        assert_eq!(convbase(-5, 10), "-5");
        assert_eq!(convbase(7, 8), "07");
        assert_eq!(convbase(5, 2), "2#101");
    }

    #[test]
    fn test_format_float() {
        let s = format_float(3.14, 2, flags::FFLOAT);
        assert!(s.starts_with("3.14"));

        assert_eq!(format_float(f64::INFINITY, 0, 0), "Inf");
        assert_eq!(format_float(f64::NEG_INFINITY, 0, 0), "-Inf");
        assert_eq!(format_float(f64::NAN, 0, 0), "NaN");
    }

    #[test]
    fn test_augment_scalar() {
        let mut table = ParamTable::new();
        table.set_scalar("foo", "hello");
        table.augment_scalar("foo", " world");
        assert_eq!(table.get_value("foo").unwrap().as_string(), "hello world");
    }

    #[test]
    fn test_augment_integer() {
        let mut table = ParamTable::new();
        table.set_integer("count", 10);
        table.augment_integer("count", 5);
        assert_eq!(table.get_value("count").unwrap().as_integer(), 15);
    }

    #[test]
    fn test_augment_array() {
        let mut table = ParamTable::new();
        table.set_array("arr", vec!["a".into(), "b".into()]);
        table.augment_array("arr", vec!["c".into(), "d".into()]);
        assert_eq!(
            table.get_value("arr").unwrap().as_array(),
            vec!["a", "b", "c", "d"]
        );
    }

    #[test]
    fn test_array_element_access() {
        let mut table = ParamTable::new();
        table.set_array("arr", vec!["a".into(), "b".into(), "c".into()]);

        assert_eq!(table.get_array_element("arr", 1), Some("a".to_string()));
        assert_eq!(table.get_array_element("arr", -1), Some("c".to_string()));
        assert_eq!(table.get_array_element("arr", 4), None);

        table.set_array_element("arr", 2, "B");
        assert_eq!(table.get_array_element("arr", 2), Some("B".to_string()));
    }

    #[test]
    fn test_hash_element_access() {
        let mut table = ParamTable::new();
        let mut hash = HashMap::new();
        hash.insert("k1".to_string(), "v1".to_string());
        table.set_assoc("h", hash);

        assert_eq!(table.get_hash_element("h", "k1"), Some("v1".to_string()));
        table.set_hash_element("h", "k2", "v2");
        assert_eq!(table.get_hash_element("h", "k2"), Some("v2".to_string()));

        table.unset_hash_element("h", "k1");
        assert_eq!(table.get_hash_element("h", "k1"), None);
    }

    #[test]
    fn test_scope_special_restore() {
        let mut table = ParamTable::new();

        let initial_shlvl = table.shlvl;

        table.push_scope();
        table.make_local("SHLVL");
        table.set_integer("SHLVL", 99);
        assert_eq!(table.get_value("SHLVL").unwrap().as_integer(), 99);

        table.pop_scope();
        assert_eq!(
            table.get_value("SHLVL").unwrap().as_integer(),
            initial_shlvl
        );
    }

    #[test]
    fn test_export_unexport() {
        let mut table = ParamTable::new();
        table.set_scalar("MY_VAR", "test_val");
        table.export("MY_VAR");
        assert_eq!(env::var("MY_VAR").ok(), Some("test_val".to_string()));

        table.unexport("MY_VAR");
        assert!(env::var("MY_VAR").is_err());
    }

    #[test]
    fn test_parse_subscript() {
        let idx = parse_subscript("@", false).unwrap();
        assert!(idx.is_all);

        let idx = parse_subscript("3", false).unwrap();
        assert_eq!(idx.start, 3);

        let idx = parse_subscript("2,5", false).unwrap();
        assert_eq!(idx.start, 2);
        assert_eq!(idx.end, 5);
    }

    #[test]
    fn test_getarrvalue() {
        let arr = vec!["a".into(), "b".into(), "c".into(), "d".into()];
        assert_eq!(getarrvalue(&arr, 2, 3), vec!["b", "c"]);
        assert_eq!(getarrvalue(&arr, -2, -1), vec!["c", "d"]);
        assert_eq!(getarrvalue(&arr, 1, 4), vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn test_setarrvalue() {
        let mut arr = vec!["a".into(), "b".into(), "c".into(), "d".into()];
        setarrvalue(&mut arr, 2, 3, vec!["X".into(), "Y".into()]);
        assert_eq!(arr, vec!["a", "X", "Y", "d"]);
    }

    #[test]
    fn test_valid_refname() {
        assert!(valid_refname("foo"));
        assert!(valid_refname("_bar"));
        assert!(valid_refname("1"));
        assert!(valid_refname("!"));
        assert!(valid_refname("arr[1]"));
        assert!(!valid_refname(""));
        assert!(!valid_refname("foo bar"));
    }

    #[test]
    fn test_glob_match() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("foo*", "foobar"));
        assert!(!glob_match("foo*", "barfoo"));
        assert!(glob_match("*bar", "foobar"));
        assert!(glob_match("exact", "exact"));
        assert!(!glob_match("exact", "other"));
    }

    #[test]
    fn test_format_param() {
        let mut table = ParamTable::new();
        table.set_scalar("MY_VAR", "hello world");
        let out = table.format_param("MY_VAR", print_flags::TYPESET).unwrap();
        assert!(out.contains("MY_VAR"));
        assert!(out.contains("hello world"));
    }

    #[test]
    fn test_seconds() {
        let table = ParamTable::new();
        let secs = table.get_seconds_int();
        assert!(secs >= 0);

        let fsecs = table.get_seconds_float();
        assert!(fsecs >= 0.0);
    }

    #[test]
    fn test_pipestatus() {
        let mut table = ParamTable::new();
        table.pipestats = vec![0, 1, 2];
        let val = table.get_value("pipestatus").unwrap();
        assert_eq!(val.as_array(), vec!["0", "1", "2"]);
    }

    #[test]
    fn test_str_slice() {
        let mut table = ParamTable::new();
        table.set_scalar("s", "hello world");

        let slice = table.get_str_slice("s", 0, 5);
        assert_eq!(slice, "hello");

        table.set_str_slice("s", 0, 5, "goodbye");
        assert_eq!(table.get_value("s").unwrap().as_string(), "goodbye world");
    }

    #[test]
    fn test_createparam() {
        let mut table = ParamTable::new();
        assert!(table.createparam("newvar", flags::SCALAR));
        assert!(table.contains("newvar"));

        assert!(table.createparam("intvar", flags::INTEGER));
        assert_eq!(table.get_value("intvar").unwrap().as_integer(), 0);
    }

    #[test]
    fn test_mnumber() {
        let i = MNumber::Integer(42);
        assert_eq!(i.as_integer(), 42);
        assert_eq!(i.as_float(), 42.0);
        assert!(!i.is_float());

        let f = MNumber::Float(3.14);
        assert_eq!(f.as_integer(), 3);
        assert!((f.as_float() - 3.14).abs() < 1e-10);
        assert!(f.is_float());
    }

    #[test]
    fn test_uniq_array_empty() {
        let empty: Vec<String> = Vec::new();
        assert!(uniq_array(empty).is_empty());
    }

    #[test]
    fn test_convbase_underscore() {
        let s = convbase_underscore(1234567, 10, 3);
        assert_eq!(s, "1_234_567");
    }

    #[test]
    fn test_subscription_flags() {
        let (flags, rest) = parse_subscription_flags("(r)3");
        assert!(flags.reverse);
        assert!(!flags.down);
        assert_eq!(rest, "3");

        let (flags, _) = parse_subscription_flags("(I)foo");
        assert!(flags.reverse);
        assert!(flags.down);
        assert!(flags.index);
    }
}
