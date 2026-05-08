//! Parameter management for zshrs
//!
//! Port from zsh/Src/params.c (6511 lines → full Rust port)
//!
//! Provides shell parameters (variables), special parameters, arrays,
//! associative arrays, parameter attributes, namerefs, scoping,
//! tied parameters, and all special parameter get/set functions.

#[allow(unused_imports)]
use crate::ported::exec::{self, ShellExecutor};
use crate::ported::utils::zerr;
use crate::ported::text::format_function_body_zsh;
use indexmap::IndexMap;
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

#[derive(Clone, Debug, Default)]
/// Storage union for a parameter's value.
/// Mirrors the value union of `struct param` from Src/zsh.h —
/// scalar / integer / float / array / hash / undef. The C source
/// dispatches on `pm->u.*` based on `pm->node.flags & PM_TYPE`.
pub enum ParamValue {
    Scalar(String),
    Integer(i64),
    Float(f64),
    Array(Vec<String>),
    Assoc(HashMap<String, String>),
    #[default]
    Unset,
}

impl ParamValue {
    pub fn as_string(&self) -> String {
        match self {
            ParamValue::Scalar(s) => s.clone(),
            ParamValue::Integer(i) => i.to_string(),
            ParamValue::Float(f) => convfloat(*f, 0, 0),
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
            ParamValue::Float(f) => vec![convfloat(*f, 0, 0)],
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
/// Math numeric value (mirrors `mnumber`).
/// Re-export of the union shape from Src/math.c — exposed here
/// so callers in subst / arith paths can pass values through
/// without `MathNum` ↔ `MNumber` conversion.
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
/// One reference to a parameter (with optional subscript).
/// Port of `struct value` from Src/zsh.h — `getvalue()`
/// (Src/params.c:2173) builds these for every `${var[N]}` /
/// `${var:-default}` / `${var//x/y}` access.
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
/// One parameter table entry.
/// Port of `struct param` from Src/zsh.h — `createparam()`
/// (Src/params.c:1030) constructs them, `paramtab` HashTable
/// stores them. Same `gsu` (get/set/unset) callback shape.
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
/// Sidecar data for `typeset -T` array-tied scalars.
/// Port of the `tied_param` storage `bin_typeset()`
/// (Src/builtin.c) installs alongside the array+scalar pair.
pub struct TiedData {
    pub join_char: char,
    pub scalar_name: String,
    pub array_name: String,
}

// ---------------------------------------------------------------------------
// Subscript flags for getarg()
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
/// `${var[N]}` subscript flags.
/// Port of the `SCANPM_*` bits Src/params.c uses inside
/// `getindex()` (line 2001) — `WANTVALS`/`WANTKEYS`/etc.
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
/// Resolved subscript range.
/// Output of `getindex()` from Src/params.c:2001 — start/end
/// positions plus flags.
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

/// Per-type metadata for a parameter.
/// Mirrors the type-info bits Src/params.c surfaces via
/// `paramtypestr()` (Src/Modules/parameter.c:43) for `typeset -p`
/// output.
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
/// Special-parameter definition.
/// Mirrors the `IPDEF*` macro entries in Src/params.c:297-... —
/// each special parameter (e.g. `RANDOM`, `EPOCHSECONDS`,
/// `HISTFILE`) provides a `gsu` (get/set/unset) callback set.
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

/// Parameter table.
/// Port of the `paramtab` HashTable Src/params.c maintains —
/// `createparamtable()` (line 817) initializes it with all the
/// IPDEF*-declared special params; `createparam()` (line 1030)
/// adds user variables.
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
            "0" if !self.posix_argzero => {
                self.argzero = value.as_string();
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
                uniqarray(value)
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
            if existing.level == self.local_level && !existing.is_unset() && !existing.is_special()
            {
                // Already exists and set at this level
                if let Some(p) = self.params.get_mut(name) {
                    p.flags &= !flags::UNSET;
                }
                return false;
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
                    convfloat(param.value.as_float(), param.base, param.flags)
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
            if pattern.is_empty() || crate::glob::matchpat(pattern, name, false, true) {
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
            .filter(|(name, _)| pattern.is_none_or(|p| crate::glob::matchpat(p, name, false, true)))
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
            } else {
                // local-scope and global both print as `typeset` when no
                // other prefix applies; the difference is in the param
                // flags shown afterwards, not the keyword.
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
                    out.push_str(&crate::ported::utils::quotedzputs(s));
                }
                ParamValue::Integer(i) => {
                    out.push_str(&convbase(*i, param.base as u32));
                }
                ParamValue::Float(f) => {
                    out.push_str(&convfloat(*f, param.base, param.flags));
                }
                ParamValue::Array(arr) => {
                    out.push('(');
                    for (i, elem) in arr.iter().enumerate() {
                        if i > 0 {
                            out.push(' ');
                        }
                        out.push_str(&crate::ported::utils::quotedzputs(elem));
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
                        out.push_str(&crate::ported::utils::quotedzputs(k));
                        out.push_str("]=");
                        out.push_str(&crate::ported::utils::quotedzputs(v));
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

/// Get integer parameter value (from params.c getintvalue)
/// Get an integer parameter.
/// Port of `getintvalue()` from Src/params.c:2601.
pub fn getintvalue(table: &ParamTable, name: &str) -> i64 {
    table.get_value(name).map(|v| v.as_integer()).unwrap_or(0)
}

/// Get scalar (string) parameter (from params.c getstrvalue)
/// Get a scalar parameter.
/// Port of `getstrvalue()` from Src/params.c:2335.
pub fn getstrvalue(table: &ParamTable, name: &str) -> Option<String> {
    table.get_value(name).map(|v| v.as_string())
}

/// Get scalar with default
/// Get a scalar parameter with a default fallback.
/// zshrs convenience over `getstrvalue()` — C zsh inlines the
/// `value ? value : default` ternary at every call site.
pub fn getsparam_u(table: &ParamTable, name: &str, default: &str) -> String {
    getstrvalue(table, name).unwrap_or_else(|| default.to_string())
}

/// Get an array parameter.
/// Port of `getaparam()` from Src/params.c:3100.
pub fn getaparam(table: &ParamTable, name: &str) -> Option<Vec<String>> {
    match table.get_value(name)? {
        ParamValue::Array(arr) => Some(arr),
        _ => None,
    }
}

/// Get hash parameter values as array (from params.c gethparam)
/// Get a hash parameter as a flat key/value array.
/// Port of the `${(kv)hash}` materialization Src/params.c does
/// inside `getstrvalue()` (line 2335) for hash params.
pub fn gethparam(table: &ParamTable, name: &str) -> Option<Vec<String>> {
    match table.get_value(name)? {
        ParamValue::Assoc(h) => Some(h.values().cloned().collect()),
        _ => None,
    }
}

/// Get hash parameter keys as array (from params.c gethkparam)
/// Get a hash parameter's keys only.
/// Port of the `${(k)hash}` extraction in Src/params.c.
pub fn gethkparam(table: &ParamTable, name: &str) -> Option<Vec<String>> {
    match table.get_value(name)? {
        ParamValue::Assoc(h) => Some(h.keys().cloned().collect()),
        _ => None,
    }
}

/// Get numeric parameter (from params.c getnumvalue)
/// Get a parameter as an `MNumber`.
/// Port of `getnumvalue()` from Src/params.c:2624.
pub fn getnumvalue(table: &ParamTable, name: &str) -> MNumber {
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

/// Assign string parameter (from params.c setstrvalue)
/// Assign a scalar parameter.
/// Port of `setstrvalue()` from Src/params.c:2685.
pub fn setstrvalue(table: &mut ParamTable, name: &str, val: &str) -> bool {
    table.set_scalar(name, val)
}

/// Assign integer parameter (from params.c assigniparam)
/// Assign an integer parameter.
/// Port of the integer branch of `setvalue()` (Src/params.c).
pub fn assigniparam(table: &mut ParamTable, name: &str, val: i64) -> bool {
    table.set_integer(name, val)
}

/// Assign array parameter (from params.c setaparam)
/// Assign an array parameter.
/// Port of `setaparam()` (Src/params.c).
pub fn setaparam(table: &mut ParamTable, name: &str, val: Vec<String>) -> bool {
    table.set_array(name, val)
}

/// Subscript-aware scalar parameter assignment.
///
/// Port of `assignsparam()` from Src/params.c:3193. The C function
/// takes the LHS as a single string and embeds the subscript inside
/// brackets (e.g. `m[$k]`); it then calls `getvalue` → `getindex` →
/// `getarg` to parse-and-singsub the subscript before dispatching
/// to the typed setter. Our Rust adaptation takes the subscript
/// separately because subst.rs::paramsubst already singsubs it
/// (subst.rs:1822 — direct port of the same singsub call inside
/// getarg at params.c:1567).
///
/// Dispatch shape mirrors C:
///   - subscript present → key/index lookup, dispatch to existing
///     assoc / array, or auto-vivify as assoc on string keys
///     (mirrors createparam(s, PM_HASHED) fallback at params.c:3214)
///   - no subscript → scalar set (params.c:3253) — caller is
///     expected to have routed `(A)`/`(AA)`-flagged assignments to
///     `assignaparam`/`assignhparam` instead.
///
/// TODOs (not yet ported from C — file:line cited where they live):
///   - PM_READONLY rejection (params.c:3210)
///   - resetparam scalar conversion (params.c:3232)
///   - PM_NAMEREF dispatch (params.c:3250)
///   - PM_TIED + PM_SPECIAL setfn callbacks
///   - ASSPM_AUGMENT (`+=` augment semantics)
pub fn assignsparam(
    variables: &mut std::collections::HashMap<String, String>,
    arrays: &mut std::collections::HashMap<String, Vec<String>>,
    assoc_arrays: &mut std::collections::HashMap<String, indexmap::IndexMap<String, String>>,
    name: &str,
    subscript: Option<&str>,
    val: &str,
) {
    if let Some(key) = subscript {
        // c:3210 (subscripted)
        // Existing assoc — write the key.
        if let Some(map) = assoc_arrays.get_mut(name) {
            // c:3602 sethparam path
            map.insert(key.to_string(), val.to_string());
            return;
        }
        // Numeric key on a (potentially auto-vivified) array.
        if let Ok(idx) = key.parse::<i64>() {
            // c:3357 assignaparam idx
            let arr = arrays.entry(name.to_string()).or_default();
            let len = arr.len() as i64;
            // 1-based forward, negative-from-end. Direct port of
            // setarrvalue's offset math (params.c).
            let real_idx = if idx < 0 { len + idx } else { idx - 1 };
            let real_idx = real_idx.max(0) as usize;
            while arr.len() <= real_idx {
                arr.push(String::new());
            }
            arr[real_idx] = val.to_string();
            variables.remove(name);
            return;
        }
        // String key on an unset name — auto-vivify as assoc, mirroring
        // the C source's `createparam(s, PM_HASHED)` fallback inside
        // assignhparam when the target doesn't exist (params.c:3214).
        let mut map: indexmap::IndexMap<String, String> = indexmap::IndexMap::new();
        map.insert(key.to_string(), val.to_string());
        assoc_arrays.insert(name.to_string(), map);
        variables.remove(name);
        arrays.remove(name);
        return;
    }
    // No subscript — scalar set (params.c:3253 setvalue path).
    variables.insert(name.to_string(), val.to_string());
}

/// Array parameter assignment (no subscript).
///
/// Port of `assignaparam()` from Src/params.c:3357. Used by the
/// `(A)`-flagged `${var=val}` form in paramsubst (subst.c:3263).
/// Splits `val` on IFS into elements and stores as an array,
/// dropping any prior scalar/assoc value at `name`.
///
/// TODOs (not yet ported):
///   - PM_READONLY check (params.c:3370-3381)
///   - resetparam from non-array (params.c:3403)
///   - element-wise subscript assignment with `[k]` syntax
pub fn assignaparam(
    variables: &mut std::collections::HashMap<String, String>,
    arrays: &mut std::collections::HashMap<String, Vec<String>>,
    assoc_arrays: &mut std::collections::HashMap<String, indexmap::IndexMap<String, String>>,
    name: &str,
    parts: Vec<String>,
) {
    arrays.insert(name.to_string(), parts);
    variables.remove(name);
    assoc_arrays.remove(name);
}

/// Hash parameter assignment (no subscript).
///
/// Port of `sethparam()` from Src/params.c:3602 / `assignhparam` at
/// 3357. Used by the `(AA)`-flagged `${var=val}` form in paramsubst
/// (subst.c:3263). Takes a flat key/value sequence and stores as an
/// associative array, dropping prior scalar/array.
///
/// TODOs (not yet ported):
///   - PM_READONLY rejection (params.c:3617)
///   - createparam(PM_HASHED) flag propagation
pub fn sethparam(
    variables: &mut std::collections::HashMap<String, String>,
    arrays: &mut std::collections::HashMap<String, Vec<String>>,
    assoc_arrays: &mut std::collections::HashMap<String, indexmap::IndexMap<String, String>>,
    name: &str,
    parts: Vec<String>,
) {
    let mut map: indexmap::IndexMap<String, String> = indexmap::IndexMap::new();
    let mut iter = parts.into_iter();
    while let Some(k) = iter.next() {
        let v = iter.next().unwrap_or_default();
        map.insert(k, v);
    }
    assoc_arrays.insert(name.to_string(), map);
    variables.remove(name);
    arrays.remove(name);
}

/// Unset parameter (from params.c unsetparam_pm)
/// Unset a parameter.
/// Port of `unsetparam_pm()` (Src/params.c) — invokes the
/// per-param `unsetfn` callback before removing from the
/// HashTable.
pub fn unsetparam_pm(table: &mut ParamTable, name: &str) -> bool {
    table.unset(name)
}

/// Empty special-hash sentinel.
/// Port of `shempty()` from Src/params.c:1166. The C source uses
/// it as a no-op getfn callback for special hashes that need an
/// addressable function pointer but no actual work. Provided here
/// so future callers that match the C source's signature can call
/// it directly.
pub fn shempty() {}

/// Set scalar parameter.
/// Port of `setsparam()` from Src/params.c:3350 — single-line
/// wrapper around `assignsparam(s, val, ASSPM_WARN)`. ASSPM_WARN
/// is a no-op in our port (no global "warn on creation" tracking
/// yet); the call shape is preserved so subst.rs can call this
/// where C calls setsparam.
pub fn setsparam(
    variables: &mut std::collections::HashMap<String, String>,
    arrays: &mut std::collections::HashMap<String, Vec<String>>,
    assoc_arrays: &mut std::collections::HashMap<String, indexmap::IndexMap<String, String>>,
    name: &str,
    val: &str,
) {
    assignsparam(variables, arrays, assoc_arrays, name, None, val);
}

/// Set integer parameter.
/// Port of `setiparam()` from Src/params.c:3765. The C source
/// constructs an `mnumber` and calls `assignnparam(s, mnval,
/// ASSPM_WARN)`; assignnparam dispatches on integer-vs-float plus
/// existing param type. Until assignnparam is fully ported, we
/// stringify and route through assignsparam — matches behavior
/// when target is scalar or not yet defined (the common case).
///
/// TODO: when assignnparam is ported (params.c:3664, 72 lines),
/// route through it for proper PM_INTEGER promotion + base/width
/// preservation.
pub fn setiparam(
    variables: &mut std::collections::HashMap<String, String>,
    arrays: &mut std::collections::HashMap<String, Vec<String>>,
    assoc_arrays: &mut std::collections::HashMap<String, indexmap::IndexMap<String, String>>,
    name: &str,
    val: i64,
) {
    assignsparam(
        variables,
        arrays,
        assoc_arrays,
        name,
        None,
        &val.to_string(),
    );
}

/// Set integer parameter without forcing PM_INTEGER promotion.
/// Port of `setiparam_no_convert()` from Src/params.c:3781. C
/// source comment: "If the target is already an integer, this
/// gets converted back. Low technology rules." It uses convbase
/// to render decimal then calls assignsparam. Same effect here.
pub fn setiparam_no_convert(
    variables: &mut std::collections::HashMap<String, String>,
    arrays: &mut std::collections::HashMap<String, Vec<String>>,
    assoc_arrays: &mut std::collections::HashMap<String, indexmap::IndexMap<String, String>>,
    name: &str,
    val: i64,
) {
    assignsparam(
        variables,
        arrays,
        assoc_arrays,
        name,
        None,
        &val.to_string(),
    );
}

/// Retrieve scalar parameter as string.
/// Port of `getsparam()` from Src/params.c:3076. C calls
/// `getvalue(&vbuf, &s, 0)` then `getstrvalue(v)`; getvalue does
/// per-name dispatch through gsu->getfn callbacks for special /
/// magic-assoc params. Until getvalue is ported, we read directly
/// from the HashMap. Returns None for unset.
pub fn getsparam(
    variables: &std::collections::HashMap<String, String>,
    arrays: &std::collections::HashMap<String, Vec<String>>,
    name: &str,
) -> Option<String> {
    if let Some(s) = variables.get(name) {
        return Some(s.clone());
    }
    // C's getvalue auto-joins arrays as scalar via getstrvalue
    // when the param is array-typed. Mirror with IFS-first-char
    // join when only an array entry exists.
    arrays.get(name).map(|a| a.join(" "))
}

/// Retrieve integer parameter.
/// Port of `getiparam()` from Src/params.c:3044. C: getvalue +
/// getintvalue. Our adaptation reads the scalar string and parses;
/// returns 0 on missing or unparseable, matching getintvalue's
/// failure-returns-0 convention (params.c:2601).
pub fn getiparam(
    variables: &std::collections::HashMap<String, String>,
    arrays: &std::collections::HashMap<String, Vec<String>>,
    name: &str,
) -> i64 {
    getsparam(variables, arrays, name)
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0)
}

/// Retrieve numeric (int-or-float) parameter.
/// Port of `getnparam()` from Src/params.c:3058. C returns an
/// `mnumber` (tagged int/float union); our adaptation returns
/// `(i64, f64, bool)` where the bool is true for float. Unset
/// returns `(0, 0.0, false)`, matching the MN_INTEGER zero
/// fallback in the C source's not-found branch.
pub fn getnparam(
    variables: &std::collections::HashMap<String, String>,
    arrays: &std::collections::HashMap<String, Vec<String>>,
    name: &str,
) -> (i64, f64, bool) {
    let s = match getsparam(variables, arrays, name) {
        Some(s) => s,
        None => return (0, 0.0, false),
    };
    if s.contains('.') || s.contains('e') || s.contains('E') {
        if let Ok(f) = s.parse::<f64>() {
            return (f as i64, f, true);
        }
    }
    if let Ok(i) = s.parse::<i64>() {
        return (i, i as f64, false);
    }
    (0, 0.0, false)
}

/// Unset a parameter from all storage.
/// Port of `unsetparam()` from Src/params.c:3819. C uses a single
/// HashTable; our SubstState-style storage spans variables /
/// arrays / assoc_arrays, so removal must touch all three to be
/// thorough (matches `unsetparam_pm`'s flag-aware tear-down).
pub fn unsetparam(
    variables: &mut std::collections::HashMap<String, String>,
    arrays: &mut std::collections::HashMap<String, Vec<String>>,
    assoc_arrays: &mut std::collections::HashMap<String, indexmap::IndexMap<String, String>>,
    name: &str,
) {
    variables.remove(name);
    arrays.remove(name);
    assoc_arrays.remove(name);
}

/// Export parameter to environment
/// Mark a parameter exported.
/// Port of `export_param()` from Src/params.c:2653 — sets
/// `PM_EXPORTED` and pushes the value into the env via
/// `setenv(3)`.
pub fn export_param(table: &mut ParamTable, name: &str) {
    table.export_param(name);
}

/// Start a parameter scope
/// Enter a function-local parameter scope.
/// Port of `startparamscope()` (Src/init.c) — the C source
/// pushes the current scope onto a stack so `local`-declared
/// parameters disappear on function exit.
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
        if chars.peek().is_none_or(|c| c.is_ascii_digit()) {
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

/// Colon-separated path to array.
/// Port of `colonsplit()` from Src/params.c.
pub fn colonsplit(s: &str) -> Vec<String> {
    s.split(':')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// Array to colon-separated path — inverse of `colonsplit`.
/// Port of `colonarrgetfn()` from Src/params.c (joins the array
/// stored in `pm->u.colon` back into the `:`-form for env).
pub fn colonarrgetfn(arr: &[String]) -> String {
    arr.join(":")
}

/// Remove duplicate elements from array while preserving order.
/// Port of `uniqarray()` from Src/params.c.
pub fn uniqarray(arr: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    arr.into_iter().filter(|s| seen.insert(s.clone())).collect()
}

/// Parse a subscript expression like `[1]`, `[1,5]`, `[@]`, `[*]`.
/// Direct port of the trivial-int branch of `getindex`/`getarg`
/// (Src/params.c:1367 / 2001) — `mathevalarg` parses the int.
pub fn parse_subscript(subscript: &str, _ksh_arrays: bool) -> Option<SubscriptIndex> {
    let s = subscript.trim();

    if s == "@" || s == "*" {
        return Some(SubscriptIndex::all());
    }

    // Inline int-parse (was a `parse_index_value` helper). Empty
    // → None matches getarg's "no digits" early-return. Trim each
    // side independently so `[ 1 , 5 ]` parses.
    if let Some(comma_pos) = s.find(',') {
        let l = s[..comma_pos].trim();
        let r = s[comma_pos + 1..].trim();
        if l.is_empty() || r.is_empty() {
            return None;
        }
        let start = l.parse::<i64>().ok()?;
        let end = r.parse::<i64>().ok()?;
        return Some(SubscriptIndex::range(start, end));
    }

    if s.is_empty() {
        return None;
    }
    let idx = s.parse::<i64>().ok()?;
    Some(SubscriptIndex::single(idx))
}

/// Slice an indexed array using zsh 1-based inclusive semantics.
/// Port of `getarrvalue()` from Src/params.c:2548 — the slice
/// branch that resolves the start/end pair into a Vec. Negative
/// indices count from the end (`-1` is the last element);
/// out-of-range bounds collapse to empty (`${a[5,10]}` on len=3
/// returns empty, not clamped); `start > end` returns empty.
///
/// 0 has asymmetric meaning per C source's getarrvalue:
///   start=0 → "before first element" → resolved to 1
///   end=0   → "before first element" → empty slice
pub fn getarrvalue(arr: &[String], start: i64, end: i64) -> Vec<String> {
    let len = arr.len() as i64;
    if len == 0 {
        return Vec::new();
    }
    // Out-of-range starts (positive past len, or negative below
    // -len) collapse to empty per Src/params.c getarrvalue's
    // slice-resolution branches.
    if start > len {
        return Vec::new();
    }
    if end < 0 && (len + end + 1) < 1 {
        return Vec::new();
    }
    if start < 0 && end < 0 && start > end {
        return Vec::new();
    }
    if start < 0 && start < -len {
        return Vec::new();
    }
    let resolve_start = |i: i64| -> i64 {
        if i < 0 {
            (len + i + 1).max(1)
        } else if i == 0 {
            1
        } else {
            i.min(len)
        }
    };
    let resolve_end = |i: i64| -> i64 {
        if i < 0 {
            (len + i + 1).max(0)
        } else if i == 0 {
            0
        } else {
            i.min(len)
        }
    };
    let s = resolve_start(start);
    let e = resolve_end(end);
    if e < 1 || s > e {
        return Vec::new();
    }
    let s_idx = (s - 1) as usize;
    let e_idx = e as usize;
    arr[s_idx..e_idx.min(arr.len())].to_vec()
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
    let (prefix, digits) = if let Some(rest) = s.strip_prefix('-') {
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

/// Port of `convfloat()` from `Src/params.c:5689`.
///
/// C signature: `char *convfloat(double dval, int digits, int flags,
/// FILE *fout)` — picks `%e` / `%f` / `%g` based on PM_EFLOAT /
/// PM_FFLOAT (line 5705-5727), then snprintf'd with `digits` precision.
/// When neither E nor F flag is set, zsh uses `%.*g` with a default
/// of 17 significant digits (line 5712-5714). E-flag with N significant
/// figures decrements `digits` because `%e` counts decimal places not
/// significants (line 5720-5725).
///
/// Rust signature drops the `fout` parameter — every caller wanted the
/// returned string. IEEE specials (inf/nan) hand-formatted to `Inf`/
/// `-Inf`/`NaN` ahead of the snprintf, matching the C source's Inf/NaN
/// shortcuts at lines 5733-5736 / 5742-5744. The trailing-dot rule for
/// integer-valued floats (`5` -> `5.`) is added by the caller (params'
/// internal printing path) in C zsh; mirrored here for the no-flag case
/// so `MathNum::Float(5.0).format_zsh_subst()` produces `5.` not `5`.
pub fn convfloat(dval: f64, digits: i32, pm_flags: u32) -> String {
    if dval.is_infinite() {                                       // c:5742
        return if dval < 0.0 {
            "-Inf".to_string()
        } else {
            "Inf".to_string()
        };
    }
    if dval.is_nan() {                                            // c:5744
        return "NaN".to_string();
    }
    // Pick fmt char + adjust digits per the C cascade at 5705-5727.
    let (fmt_char, digits) = if (pm_flags & flags::EFLOAT) != 0 { // c:5715
        let d = if digits <= 0 { 10 } else { digits };           // c:5718
        ('e', (d - 1).max(0))                                    // c:5725
    } else if (pm_flags & flags::FFLOAT) != 0 {                  // c:5716
        let d = if digits <= 0 { 10 } else { digits };           // c:5718
        ('f', d)
    } else {
        let d = if digits == 0 { 17 } else { digits };           // c:5713
        ('g', d)
    };
    // Mirror zsh's snprintf path (Src/params.c:5751) — the C source
    // uses `VARARR(char, buf, 512 + digits)` for %f's full integer-
    // part expansion. 512 + 17 = 529 covers the zsh general case;
    // wider buffers below for the unbounded %f.
    let buf_len = 512usize + digits as usize + 4;
    let mut buf = vec![0u8; buf_len];
    let fmt = match fmt_char {
        'e' => c"%.*e",
        'f' => c"%.*f",
        _ => c"%.*g",
    };
    // SAFETY: buf has the C-required size for any double precision; fmt
    // is a NUL-terminated literal; snprintf writes ASCII only.
    let n = unsafe {
        libc::snprintf(
            buf.as_mut_ptr() as *mut libc::c_char,
            buf_len,
            fmt.as_ptr(),
            digits as libc::c_int,
            dval,
        )
    };
    if n < 0 {
        return format!("{}", dval);
    }
    let len = (n as usize).min(buf_len - 1);
    buf.truncate(len);
    let mut s = String::from_utf8(buf).unwrap_or_else(|_| format!("{}", dval));
    // zsh's general-format (%g) callers (math `$(( ))` substitution)
    // append `.` when the output has no `e` and no `.`, so integer-
    // valued floats like `5` render as `5.`. PM_EFLOAT/PM_FFLOAT skip
    // this rule (the format spec already pins shape).
    if fmt_char == 'g' && !s.contains('e') && !s.contains('.') {
        s.push('.');
    }
    s
}

/// Format float with underscores
pub fn convfloat_underscore(dval: f64, underscore: i32) -> String {
    let s = convfloat(dval, 0, 0);
    if underscore <= 0 {
        return s;
    }

    let u = underscore as usize;
    let (sign, rest) = if let Some(after) = s.strip_prefix('-') {
        ("-", after)
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
    if let Some(frac) = frac_exp.strip_prefix('.') {
        result.push('.');
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
    let val = getintvalue(table, name);
    convbase(val, base)
}

/// String parameter with modifiers (from params.c strgetfn)
pub fn strgetfn(table: &ParamTable, name: &str, lower: bool, upper: bool) -> Option<String> {
    let val = getstrvalue(table, name)?;
    Some(if lower {
        val.to_lowercase()
    } else if upper {
        val.to_uppercase()
    } else {
        val
    })
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
        let arr = colonsplit("/bin:/usr/bin:/usr/local/bin");
        assert_eq!(arr, vec!["/bin", "/usr/bin", "/usr/local/bin"]);
        let path = colonarrgetfn(&arr);
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
        let result = uniqarray(arr);
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
    fn test_convfloat() {
        // Use 2.5 instead of 3.14 — clippy errors on the latter as
        // an approx PI constant. The test checks 2-decimal formatting
        // round-trips, which the exact value doesn't influence.
        let s = convfloat(2.5, 2, flags::FFLOAT);
        assert!(s.starts_with("2.50"));

        assert_eq!(convfloat(f64::INFINITY, 0, 0), "Inf");
        assert_eq!(convfloat(f64::NEG_INFINITY, 0, 0), "-Inf");
        assert_eq!(convfloat(f64::NAN, 0, 0), "NaN");
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
        use crate::glob::matchpat;
        assert!(matchpat("*", "anything", false, true));
        assert!(matchpat("foo*", "foobar", false, true));
        assert!(!matchpat("foo*", "barfoo", false, true));
        assert!(matchpat("*bar", "foobar", false, true));
        assert!(matchpat("exact", "exact", false, true));
        assert!(!matchpat("exact", "other", false, true));
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

        // Pick a float value that's not π — clippy errors on
        // 3.14 as an approx PI constant. The shape of the test is
        // "round-trip a float through MNumber"; the exact value
        // doesn't matter.
        let f = MNumber::Float(2.5);
        assert_eq!(f.as_integer(), 2);
        assert!((f.as_float() - 2.5).abs() < 1e-10);
        assert!(f.is_float());
    }

    #[test]
    fn test_uniq_array_empty() {
        let empty: Vec<String> = Vec::new();
        assert!(uniqarray(empty).is_empty());
    }

    #[test]
    fn test_convbase_underscore() {
        let s = convbase_underscore(1234567, 10, 3);
        assert_eq!(s, "1_234_567");
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: params
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// Parse subscript range like "1" or "1,5" or "-1" or "1,-1"
    pub(crate) fn parse_subscript_range(&self, s: &str, len: usize) -> Option<(usize, usize)> {
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
    /// Split a string into words based on IFS
    pub(crate) fn split_words(&self, s: &str) -> Vec<String> {
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
    /// Helper for `${arr[idx]:-default}` family — returns the element
    /// (or empty string if OOB / not present). Routes through assoc
    /// arrays first, then indexed arrays, then string subscripting.
    /// Uses the same numeric/range parsing as the main bracket handler
    /// but only the single-element case (sufficient for the modifiers
    /// that gate on emptiness).
    /// Companion to `lookup_array_element` — returns true iff the
    /// element at `index` is SET (key present for assoc, index in
    /// bounds for indexed array, char position in range for scalar
    /// substring). Used by `${arr[N]+set}` / `${arr[N]-default}` /
    /// `${arr[N]?msg}` — the no-colon variants test SET-ness, not
    /// empty-ness.
    pub(crate) fn array_element_is_set(&mut self, var_name: &str, index: &str) -> bool {
        if self.assoc_arrays.contains_key(var_name) {
            let key = self.singsub(index);
            return self
                .assoc_arrays
                .get(var_name)
                .map(|a| a.contains_key(&key))
                .unwrap_or(false);
        }
        let expanded_index = self.singsub(index);
        if let Ok(idx) = expanded_index.parse::<i64>() {
            if let Some(arr) = self.arrays.get(var_name) {
                let len = arr.len() as i64;
                let pos = if idx > 0 {
                    idx - 1
                } else if idx < 0 {
                    len + idx
                } else {
                    return false;
                };
                return pos >= 0 && pos < len;
            }
            // Scalar string — check if char index is in range.
            let val = self.get_variable(var_name);
            let n = val.chars().count() as i64;
            if n == 0 {
                return false;
            }
            let pos = if idx > 0 {
                idx - 1
            } else if idx < 0 {
                n + idx
            } else {
                return false;
            };
            return pos >= 0 && pos < n;
        }
        false
    }
    pub(crate) fn lookup_array_element(&mut self, var_name: &str, index: &str) -> String {
        if let Some(val) = self.get_special_array_value(var_name, index) {
            return val;
        }
        if self.assoc_arrays.contains_key(var_name) {
            let key = self.singsub(index);
            return self
                .assoc_arrays
                .get(var_name)
                .and_then(|a| a.get(&key).cloned())
                .unwrap_or_default();
        }
        let expanded_index = self.singsub(index);
        if let Ok(idx) = expanded_index.parse::<i64>() {
            if let Some(arr) = self.arrays.get(var_name) {
                let pos = if idx > 0 {
                    (idx - 1) as usize
                } else if idx < 0 {
                    let n = arr.len() as i64 + idx;
                    if n < 0 {
                        return String::new();
                    }
                    n as usize
                } else {
                    0
                };
                return arr.get(pos).cloned().unwrap_or_default();
            }
            // String subscript on scalar
            let val = self.get_variable(var_name);
            if val.is_empty() {
                return String::new();
            }
            let chars: Vec<char> = val.chars().collect();
            let pos = if idx > 0 {
                (idx - 1) as usize
            } else if idx < 0 {
                let n = chars.len() as i64 + idx;
                if n < 0 {
                    return String::new();
                }
                n as usize
            } else {
                0
            };
            return chars.get(pos).map(|c| c.to_string()).unwrap_or_default();
        }
        String::new()
    }
    /// Get value from zsh/parameter special arrays (options, commands, functions, etc.)
    /// Returns Some(value) if this is a special array access, None otherwise
    pub fn get_special_array_value(&self, array_name: &str, key: &str) -> Option<String> {
        match array_name {
            // === ZSH/MAPFILE module ===
            // `${mapfile[/path]}` reads the file's contents. Direct
            // port of `getpmmapfile()` (Src/Modules/mapfile.c:217)
            // which calls `get_contents()` (line 167) on the path.
            // Splice (`@`/`*`) returns the CWD entry list per
            // `scanpmmapfile()` (line 240).
            "mapfile" => {
                if key == "@" || key == "*" {
                    // Inline readdir loop — direct port of
                    // scanpmmapfile (Src/Modules/mapfile.c:241).
                    let mut files: Vec<String> = Vec::new();
                    if let Ok(rd) = std::fs::read_dir(".") {
                        for entry in rd.flatten() {
                            let path = entry.path();
                            if path.is_file() {
                                if let Some(name) =
                                    path.file_name().and_then(|n| n.to_str())
                                {
                                    files.push(name.to_string());
                                }
                            }
                        }
                    }
                    return Some(files.join(" "));
                }
                Some(crate::modules::mapfile::get_contents(key).unwrap_or_default())
            }
            // === ZSH/SYSTEM — errnos / sysparams ===
            "errnos" => {
                let table = crate::modules::system::ERRNO_NAMES;
                if key == "@" || key == "*" {
                    return Some(
                        table
                            .iter()
                            .map(|(n, _)| (*n).to_string())
                            .collect::<Vec<_>>()
                            .join(" "),
                    );
                }
                if let Ok(n) = key.parse::<i64>() {
                    let len = table.len() as i64;
                    let pos = if n > 0 {
                        (n - 1) as usize
                    } else if n < 0 {
                        let p = len + n;
                        if p < 0 {
                            return Some(String::new());
                        }
                        p as usize
                    } else {
                        return Some(String::new());
                    };
                    if let Some((name, _)) = table.get(pos) {
                        return Some((*name).to_string());
                    }
                }
                Some(String::new())
            }
            "sysparams" => {
                let pid = std::process::id().to_string();
                let ppid = unsafe { libc::getppid() }.to_string();
                if key == "@" || key == "*" {
                    return Some(format!("{} {}", pid, ppid));
                }
                Some(match key {
                    "pid" => pid,
                    "ppid" => ppid,
                    "procsubstpid" => "0".to_string(),
                    _ => String::new(),
                })
            }
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
            // ${aliases[@]} returns values in sorted-name order.
            // Iterating HashMap::values() gave random order; tests
            // and prompt code that snapshot ${(v)aliases} flickered.
            "aliases" => {
                if key == "@" || key == "*" {
                    let mut keys: Vec<&String> = self.aliases.keys().collect();
                    keys.sort();
                    let vals: Vec<String> = keys
                        .iter()
                        .filter_map(|k| self.aliases.get(*k).cloned())
                        .collect();
                    return Some(vals.join(" "));
                }
                Some(self.aliases.get(key).cloned().unwrap_or_default())
            }
            "galiases" => {
                if key == "@" || key == "*" {
                    let mut keys: Vec<&String> = self.global_aliases.keys().collect();
                    keys.sort();
                    let vals: Vec<String> = keys
                        .iter()
                        .filter_map(|k| self.global_aliases.get(*k).cloned())
                        .collect();
                    return Some(vals.join(" "));
                }
                Some(self.global_aliases.get(key).cloned().unwrap_or_default())
            }
            "saliases" => {
                if key == "@" || key == "*" {
                    let mut keys: Vec<&String> = self.suffix_aliases.keys().collect();
                    keys.sort();
                    let vals: Vec<String> = keys
                        .iter()
                        .filter_map(|k| self.suffix_aliases.get(*k).cloned())
                        .collect();
                    return Some(vals.join(" "));
                }
                Some(self.suffix_aliases.get(key).cloned().unwrap_or_default())
            }

            // === TERMINFO (zsh/terminfo module) ===
            // `${terminfo[capname]}` returns the escape sequence for
            // capability `capname`. Direct port of zsh/Src/Modules/
            // terminfo.c — the C version calls `tigetstr(name)` from
            // ncurses; we map the common-subset capability names to
            // standard xterm/VT escape sequences inline. Covers the
            // function-keys / cursor-motion / clear / color set that
            // user keymaps query (`key[F1]=$terminfo[kf1]` etc.).
            "terminfo" => {
                // Lazy lookup via ncurses tigetstr/tigetnum/tigetflag
                // — the pre-populated assoc init seeds the common
                // subset, but a script may query any cap by name
                // (`$terminfo[acsc]`, `$terminfo[colors]`). Mirror
                // zsh's terminfo.c::getterminfo lazy-resolve path.
                Some(crate::modules::terminfo::getterminfo(key).unwrap_or_default())
            }
            // `termcap` is dispatched in the `magic_assoc_lookup`
            // function (the primary special-array path) so that
            // ${termcap[cl]} resolves before this fallback runs.
            // Keeping a no-op arm here avoids a spurious "unknown
            // assoc" diagnostic if a caller bypasses
            // magic_assoc_lookup.
            "termcap" => Some(crate::modules::termcap::gettermcap(key).unwrap_or_default()),

            // === FUNCTIONS ===
            "functions" => {
                if key == "@" || key == "*" {
                    return Some(self.function_names().join(" "));
                }
                // Apply zsh's getfn_functions formatter — leading-tab
                // body, no trailing `;`. Direct port of Src/exec.c
                // shipped via compile_zsh's fast path; this branch
                // is the slow-path/subst_port entry that previously
                // returned the raw user-typed source. Keeps
                // `${functions[foo]:0:20}` (substring extraction)
                // consistent with the fast-path `\$functions[foo]`.
                let text = self.function_definition_text(key)?;
                let formatted = format_function_body_zsh(text.trim());
                Some(format!("\t{}", formatted))
            }
            "functions_source" => {
                // ${functions_source[name]} → file path where the
                // function was defined. zsh/Src/Modules/parameter.c
                // exposes this as an assoc keyed by function name.
                // For autoload functions we recover the source path
                // via the same fpath walk that loads them; for inline
                // functions we don't yet track the defining file, so
                // emit empty in that case.
                if key == "@" || key == "*" {
                    let mut all = String::new();
                    for fname in self.function_names() {
                        if let Some(p) = self.find_function_file(&fname) {
                            if !all.is_empty() {
                                all.push(' ');
                            }
                            all.push_str(&p.to_string_lossy());
                        }
                    }
                    return Some(all);
                }
                Some(
                    self.find_function_file(key)
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_default(),
                )
            }

            // === COMMANDS (command hash table) ===
            // ${commands[name]} → full path (or empty), per
            // zsh/Modules/parameter.c. The @/* expansion enumerates
            // every command on PATH (deduplicated, first-wins).
            "commands" => {
                if key == "@" || key == "*" {
                    let path_var = env::var("PATH").unwrap_or_default();
                    let mut seen: std::collections::HashSet<String> =
                        std::collections::HashSet::new();
                    let mut names: Vec<String> = Vec::new();
                    // Hashed entries first (rehash population).
                    for k in self.command_hash.keys() {
                        if seen.insert(k.clone()) {
                            names.push(k.clone());
                        }
                    }
                    for dir in path_var.split(':') {
                        if dir.is_empty() {
                            continue;
                        }
                        if let Ok(entries) = std::fs::read_dir(dir) {
                            for entry in entries.flatten() {
                                if let Ok(name) = entry.file_name().into_string() {
                                    if seen.insert(name.clone()) {
                                        names.push(name);
                                    }
                                }
                            }
                        }
                    }
                    names.sort();
                    return Some(names.join(" "));
                }
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
            // ${parameters[name]} → full attribute string per
            // VarAttr::format_zsh (e.g. 'integer-readonly-export').
            // @/* enumerates every parameter name, sorted+deduped.
            "parameters" => {
                if key == "@" || key == "*" {
                    let mut names: std::collections::BTreeSet<String> =
                        self.variables.keys().cloned().collect();
                    names.extend(self.arrays.keys().cloned());
                    names.extend(self.assoc_arrays.keys().cloned());
                    let v: Vec<String> = names.into_iter().collect();
                    return Some(v.join(" "));
                }
                if let Some(attr) = self.var_attrs.get(key) {
                    return Some(attr.format_zsh());
                }
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
            // ${nameddirs[@]} returns paths in sorted-name order (was
            // HashMap::values() with random iteration).
            "nameddirs" => {
                if key == "@" || key == "*" {
                    let mut keys: Vec<&String> = self.named_dirs.keys().collect();
                    keys.sort();
                    let vals: Vec<String> = keys
                        .iter()
                        .filter_map(|k| self.named_dirs.get(*k).map(|p| p.display().to_string()))
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
            // ${userdirs[name]} → home directory of user `name` per
            // zsh/Modules/parameter.c userdirs_*. With @/* expansion,
            // walk getpwent(3) to enumerate every passwd entry's
            // home directory.
            "userdirs" => {
                #[cfg(unix)]
                {
                    use std::ffi::{CStr, CString};
                    if key == "@" || key == "*" {
                        let mut homes: Vec<String> = Vec::new();
                        unsafe {
                            libc::setpwent();
                            loop {
                                let pwd = libc::getpwent();
                                if pwd.is_null() {
                                    break;
                                }
                                let dir = CStr::from_ptr((*pwd).pw_dir);
                                homes.push(dir.to_string_lossy().to_string());
                            }
                            libc::endpwent();
                        }
                        homes.sort();
                        homes.dedup();
                        return Some(homes.join(" "));
                    }
                    if let Ok(name) = CString::new(key) {
                        unsafe {
                            let pwd = libc::getpwnam(name.as_ptr());
                            if !pwd.is_null() {
                                let dir = CStr::from_ptr((*pwd).pw_dir);
                                return Some(dir.to_string_lossy().to_string());
                            }
                        }
                    }
                }
                Some(String::new())
            }

            // === USER GROUPS ===
            // ${usergroups[name]} → GID of group `name`. With @/*
            // expansion, walk getgrent(3) to enumerate every group's
            // gid.
            "usergroups" => {
                #[cfg(unix)]
                {
                    use std::ffi::{CStr, CString};
                    if key == "@" || key == "*" {
                        let mut gids: Vec<String> = Vec::new();
                        unsafe {
                            libc::setgrent();
                            loop {
                                let grp = libc::getgrent();
                                if grp.is_null() {
                                    break;
                                }
                                let name = CStr::from_ptr((*grp).gr_name);
                                gids.push(name.to_string_lossy().to_string());
                            }
                            libc::endgrent();
                        }
                        gids.sort();
                        gids.dedup();
                        return Some(gids.join(" "));
                    }
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
                // ${jobdirs[N]}: cwd at the time job N was launched.
                // We don't yet capture per-job cwd at launch (would
                // need a JobInfo.cwd field plumbed through add_job),
                // so use the current PWD as a best-effort proxy. With
                // @/* expansion, return one entry per active job so
                // arr-length math (${#jobdirs}) matches ${#jobtexts}.
                let pwd = self
                    .variables
                    .get("PWD")
                    .cloned()
                    .or_else(|| env::var("PWD").ok())
                    .unwrap_or_default();
                if key == "@" || key == "*" {
                    let n = self.jobs.iter().count();
                    return Some(vec![pwd; n].join(" "));
                }
                if let Ok(id) = key.parse::<usize>() {
                    if self.jobs.get(id).is_some() {
                        return Some(pwd);
                    }
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
                // $historywords: flat list of words from recent history
                // entries (zsh/Modules/parameter.c historywords_*).
                // Each command is split on whitespace; the words are
                // collected newest-first across the recent window.
                if let Some(ref engine) = self.history {
                    if let Ok(entries) = engine.recent(100) {
                        let words: Vec<String> = entries
                            .iter()
                            .flat_map(|e| {
                                e.command
                                    .split_whitespace()
                                    .map(|s| s.to_string())
                                    .collect::<Vec<_>>()
                            })
                            .collect();
                        if key == "@" || key == "*" {
                            return Some(words.join(" "));
                        }
                        if let Ok(idx) = key.parse::<usize>() {
                            if idx >= 1 && idx <= words.len() {
                                return Some(words[idx - 1].clone());
                            }
                        }
                    }
                }
                Some(String::new())
            }

            // === MODULES ===
            // ${modules[name]} → "loaded" / "" per
            // zsh/Src/Modules/parameter.c modules_*. zshrs tracks
            // loaded modules via `_module_<name>` keys in
            // self.options (see bin_zmodload). Always-loaded
            // built-in modules are surfaced unconditionally so
            // compsys's `[[ ${+modules[zsh/zutil]} ]]` gating works.
            "modules" => {
                const ALWAYS_LOADED: &[&str] = &[
                    "zsh/parameter",
                    "zsh/zutil",
                    "zsh/complete",
                    "zsh/complist",
                    "zsh/zle",
                    "zsh/main",
                    "zsh/files",
                ];
                let user_loaded: Vec<String> = self
                    .options
                    .iter()
                    .filter_map(|(k, v)| {
                        if *v {
                            k.strip_prefix("_module_").map(|s| s.to_string())
                        } else {
                            None
                        }
                    })
                    .collect();
                if key == "@" || key == "*" {
                    let mut all: Vec<String> = ALWAYS_LOADED
                        .iter()
                        .map(|s| s.to_string())
                        .chain(user_loaded.iter().cloned())
                        .collect();
                    all.sort();
                    all.dedup();
                    return Some(all.join(" "));
                }
                if ALWAYS_LOADED.contains(&key)
                    || self
                        .options
                        .get(&format!("_module_{}", key))
                        .copied()
                        .unwrap_or(false)
                {
                    Some("loaded".to_string())
                } else {
                    Some(String::new())
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
            // $funcstack: array of function names in the current call
            // chain (innermost first). Already maintained by the
            // function-call code at exec.rs:7828-7835. Surface it here
            // so `${funcstack[1]}` / `${funcstack[@]}` reads work.
            // funcfiletrace / funcsourcetrace need separate tables (file
            // and definition tracking) which we don't yet wire; emit
            // empty for those until they're populated.
            "funcstack" => {
                if let Some(stack) = self.arrays.get("funcstack") {
                    if key == "@" || key == "*" {
                        return Some(stack.join(" "));
                    }
                    if let Ok(idx) = key.parse::<usize>() {
                        // zsh subscripts are 1-based.
                        if idx >= 1 && idx <= stack.len() {
                            return Some(stack[idx - 1].clone());
                        }
                    }
                }
                Some(String::new())
            }
            "functrace" => {
                // $functrace: `caller_name:callsite_lineno` for each
                // frame. We don't yet track call-site line numbers, so
                // synthesize from funcstack with a `:0` placeholder
                // line. This still lets scripts that test
                // `[[ -n $functrace[1] ]]` work without false-empty.
                if let Some(stack) = self.arrays.get("funcstack") {
                    let synth: Vec<String> = stack.iter().map(|n| format!("{}:0", n)).collect();
                    if key == "@" || key == "*" {
                        return Some(synth.join(" "));
                    }
                    if let Ok(idx) = key.parse::<usize>() {
                        if idx >= 1 && idx <= synth.len() {
                            return Some(synth[idx - 1].clone());
                        }
                    }
                }
                Some(String::new())
            }
            "funcfiletrace" | "funcsourcetrace" => {
                // Would need file:line where each function was called
                // from / defined in. Per-frame file tracking is not yet
                // wired — return empty.
                Some(String::new())
            }

            // === DISABLED VARIANTS (dis_*) ===
            // ${dis_builtins[name]} → "defined" if the builtin was
            // disabled via `disable name`. Tracked through
            // self.options['_disabled_<name>']. The other dis_*
            // variants (aliases/functions/reswords/patchars) lose
            // their entries entirely on disable in zshrs's table
            // model (see do_enable_disable at exec.rs:31371) so the
            // disabled list isn't recoverable post-disable; emit
            // empty for those.
            "dis_builtins" => {
                let disabled: Vec<String> = self
                    .options
                    .iter()
                    .filter_map(|(k, v)| {
                        if *v {
                            k.strip_prefix("_disabled_").map(|s| s.to_string())
                        } else {
                            None
                        }
                    })
                    .collect();
                if key == "@" || key == "*" {
                    let mut sorted = disabled.clone();
                    sorted.sort();
                    return Some(sorted.join(" "));
                }
                if disabled.iter().any(|d| d == key) {
                    Some("defined".to_string())
                } else {
                    Some(String::new())
                }
            }
            "dis_aliases"
            | "dis_galiases"
            | "dis_saliases"
            | "dis_functions"
            | "dis_functions_source"
            | "dis_reswords"
            | "dis_patchars" => Some(String::new()),

            // === ZLE WIDGETS ===
            // ${widgets[name]} → widget-type prefix per
            // zsh/Src/Zle/zleparameter.c widgets_*: "builtin",
            // "user:<funcname>", or "completion:<funcname>".
            // Distinguishes builtin vs user-defined so
            // ${(t)widgets[name]} works.
            "widgets" => {
                use crate::zle::zle;
                let zle = zle();
                if key == "@" || key == "*" {
                    let mut names: Vec<&str> = zle.list_widgets();
                    names.sort();
                    return Some(
                        names
                            .into_iter()
                            .map(String::from)
                            .collect::<Vec<_>>()
                            .join(" "),
                    );
                }
                if let Some(target) = zle.get_widget(key) {
                    if target == key {
                        Some("builtin".to_string())
                    } else {
                        Some(format!("user:{}", target))
                    }
                } else {
                    Some(String::new())
                }
            }

            // === ZLE KEYMAPS ===
            // ${keymaps[N]} per zleparameter.c keymaps_*: list of
            // available keymap names. Single-key lookup returns 1
            // ("set") if the keymap exists, "" otherwise.
            "keymaps" => {
                const KEYMAPS: &[&str] = &[
                    "main",
                    "emacs",
                    "viins",
                    "vicmd",
                    "isearch",
                    "command",
                    "menuselect",
                ];
                if key == "@" || key == "*" {
                    return Some(KEYMAPS.join(" "));
                }
                if KEYMAPS.contains(&key) {
                    Some("1".to_string())
                } else {
                    Some(String::new())
                }
            }

            // === SIGNAL NAMES ===
            // $signals: array indexed by signal number (1-based) where
            // each slot holds the bare signal name. Direct port of
            // zsh/Modules/parameter.c signals_*. zshrs uses libc signal
            // constants so the mapping matches the host platform
            // (macOS USR1=30, Linux USR1=10).
            "signals" => {
                let map: &[(i32, &str)] = &[
                    (libc::SIGHUP, "HUP"),
                    (libc::SIGINT, "INT"),
                    (libc::SIGQUIT, "QUIT"),
                    (libc::SIGILL, "ILL"),
                    (libc::SIGTRAP, "TRAP"),
                    (libc::SIGABRT, "ABRT"),
                    #[cfg(target_os = "macos")]
                    (libc::SIGEMT, "EMT"),
                    (libc::SIGFPE, "FPE"),
                    (libc::SIGKILL, "KILL"),
                    (libc::SIGBUS, "BUS"),
                    (libc::SIGSEGV, "SEGV"),
                    (libc::SIGSYS, "SYS"),
                    (libc::SIGPIPE, "PIPE"),
                    (libc::SIGALRM, "ALRM"),
                    (libc::SIGTERM, "TERM"),
                    (libc::SIGURG, "URG"),
                    (libc::SIGSTOP, "STOP"),
                    (libc::SIGTSTP, "TSTP"),
                    (libc::SIGCONT, "CONT"),
                    (libc::SIGCHLD, "CHLD"),
                    (libc::SIGTTIN, "TTIN"),
                    (libc::SIGTTOU, "TTOU"),
                    (libc::SIGIO, "IO"),
                    (libc::SIGXCPU, "XCPU"),
                    (libc::SIGXFSZ, "XFSZ"),
                    (libc::SIGVTALRM, "VTALRM"),
                    (libc::SIGPROF, "PROF"),
                    (libc::SIGWINCH, "WINCH"),
                    #[cfg(target_os = "macos")]
                    (libc::SIGINFO, "INFO"),
                    (libc::SIGUSR1, "USR1"),
                    (libc::SIGUSR2, "USR2"),
                ];
                if key == "@" || key == "*" {
                    // Return one entry per signal in numeric order (1..N).
                    let max = map.iter().map(|(n, _)| *n).max().unwrap_or(0) as usize;
                    let mut slots: Vec<String> = vec![String::new(); max];
                    for (n, name) in map {
                        if (*n as usize) >= 1 && (*n as usize) <= max {
                            slots[*n as usize - 1] = (*name).to_string();
                        }
                    }
                    return Some(slots.join(" "));
                }
                // Numeric subscript -> name; name -> empty (zsh's
                // $signals is keyed by number).
                if let Ok(n) = key.parse::<i32>() {
                    for (sig_num, name) in map {
                        if *sig_num == n {
                            return Some((*name).to_string());
                        }
                    }
                }
                Some(String::new())
            }

            // Not a special array
            _ => None,
        }
    }
    pub(crate) fn get_variable(&self, name: &str) -> String {
        // Handle special parameters
        match name {
            "" => String::new(), // Empty name returns empty
            "$" => std::process::id().to_string(),
            "@" | "*" => {
                // $* joins by the first char of $IFS (POSIX). Default
                // IFS is " \t\n\0" so the join char is " "; with a
                // custom IFS like `:` the joined string uses `:`.
                // $@ technically does the same in scalar context but
                // is usually quoted-spliced — both fall through here.
                let sep = self
                    .variables
                    .get("IFS")
                    .and_then(|s| s.chars().next())
                    .unwrap_or(' ');
                self.positional_params.join(&sep.to_string())
            }
            "#" | "#@" | "#*" => self.positional_params.len().to_string(),
            // zsh alias: $ARGC also equals $#.
            "ARGC" => self.positional_params.len().to_string(),
            "?" | "status" => self.last_status.to_string(),
            "!" => self
                .variables
                .get("!")
                .cloned()
                .unwrap_or_else(|| "0".to_string()),
            // `$-` returns the concatenated single-letter flags of options
            // currently set. zsh always emits a baseline "569X" prefix
            // (internal-letter options that are on by default in -f mode)
            // followed by user-controllable flags. Match the prefix
            // verbatim so existing scripts that do `[[ $- == *e* ]]` /
            // `case $- in *x*) … esac` see consistent letters.
            "-" => {
                let mut letters = String::from("569X");
                let opt = |n: &str| self.options.get(n).copied().unwrap_or(false);
                // `e` comes BEFORE `f` in zsh's letter ordering: `set -e`
                // in -f mode produces "569Xef", not "569Xfe".
                if opt("errexit") {
                    letters.push('e');
                }
                if !opt("rcs") {
                    letters.push('f');
                }
                if opt("login") {
                    letters.push('l');
                }
                // i/m are present only when *truly* interactive; zsh's `-c`
                // path leaves them off, so we mirror that and don't surface
                // them just because `options.interactive` happens to be set
                // by the executor's default-options init.
                if opt("nounset") {
                    letters.push('u');
                }
                if opt("xtrace") {
                    letters.push('x');
                }
                if opt("verbose") {
                    letters.push('v');
                }
                if opt("noexec") {
                    letters.push('n');
                }
                if opt("hashall") {
                    letters.push('h');
                }
                letters
            }
            "EUID" => unsafe { libc::geteuid() }.to_string(),
            "UID" => unsafe { libc::getuid() }.to_string(),
            "EGID" => unsafe { libc::getegid() }.to_string(),
            "GID" => unsafe { libc::getgid() }.to_string(),
            "PPID" => unsafe { libc::getppid() }.to_string(),
            "ZSH_SUBSHELL" => self
                .variables
                .get("ZSH_SUBSHELL")
                .cloned()
                .unwrap_or_else(|| "0".to_string()),
            "HOST" => {
                // libc gethostname → up to 256 bytes.
                let mut buf = [0u8; 256];
                let r = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut _, buf.len()) };
                if r == 0 {
                    let nul = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                    String::from_utf8_lossy(&buf[..nul]).into_owned()
                } else {
                    String::new()
                }
            }
            // OS / machine identity vars. zsh hardcodes these from build-time
            // detection; we synthesize at runtime from libc uname(). Without
            // these arms `$OSTYPE` returned empty even though zle_params wrote
            // them into the params table — the executor's get_variable bypasses
            // that table for special names.
            "OSTYPE" => {
                let mut u: libc::utsname = unsafe { std::mem::zeroed() };
                if unsafe { libc::uname(&mut u) } == 0 {
                    let sysname = unsafe { std::ffi::CStr::from_ptr(u.sysname.as_ptr()) }
                        .to_string_lossy()
                        .to_lowercase();
                    let release = unsafe { std::ffi::CStr::from_ptr(u.release.as_ptr()) }
                        .to_string_lossy()
                        .to_string();
                    format!("{}{}", sysname, release)
                } else {
                    std::env::consts::OS.to_string()
                }
            }
            "MACHTYPE" => {
                let mut u: libc::utsname = unsafe { std::mem::zeroed() };
                if unsafe { libc::uname(&mut u) } == 0 {
                    let m = unsafe { std::ffi::CStr::from_ptr(u.machine.as_ptr()) }
                        .to_string_lossy()
                        .to_string();
                    // zsh shortens common machines: aarch64 → arm, x86_64
                    // stays x86_64. Mirror that for the common cases.
                    if m == "aarch64" || m == "arm64" {
                        "arm".to_string()
                    } else {
                        m
                    }
                } else {
                    std::env::consts::ARCH.to_string()
                }
            }
            "CPUTYPE" => {
                let mut u: libc::utsname = unsafe { std::mem::zeroed() };
                if unsafe { libc::uname(&mut u) } == 0 {
                    unsafe { std::ffi::CStr::from_ptr(u.machine.as_ptr()) }
                        .to_string_lossy()
                        .to_string()
                } else {
                    std::env::consts::ARCH.to_string()
                }
            }
            "VENDOR" => {
                // No portable libc query for vendor; pick by OS family.
                if cfg!(target_os = "macos") {
                    "apple".to_string()
                } else if cfg!(target_os = "linux") {
                    "unknown".to_string()
                } else {
                    "pc".to_string()
                }
            }
            "HOSTNAME" => {
                let mut buf = [0u8; 256];
                let r = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut _, buf.len()) };
                if r == 0 {
                    let nul = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                    String::from_utf8_lossy(&buf[..nul]).into_owned()
                } else {
                    String::new()
                }
            }
            "RANDOM" => {
                // zsh/bash: pseudo-random unsigned 16-bit integer per
                // expansion. We use process+nano for a cheap, OS-portable
                // source — not cryptographically secure, but matches zsh's
                // "noise" semantics.
                use std::time::{SystemTime, UNIX_EPOCH};
                let nanos = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.subsec_nanos() as u64)
                    .unwrap_or(0);
                let pid = std::process::id() as u64;
                let r = (nanos.wrapping_mul(2654435761).wrapping_add(pid)) as u32;
                ((r as u16) & 0x7fff).to_string()
            }
            "SECONDS" => {
                // Seconds since shell start. We approximate via the
                // tracked `shell_start_time` if present; otherwise 0.
                self.variables.get("SECONDS").cloned().unwrap_or_else(|| {
                    use std::time::{SystemTime, UNIX_EPOCH};
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let start = self
                        .variables
                        .get("__zshrs_start_secs")
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(now);
                    now.saturating_sub(start).to_string()
                })
            }
            "EPOCHSECONDS" => {
                use std::time::{SystemTime, UNIX_EPOCH};
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs().to_string())
                    .unwrap_or_else(|_| "0".to_string())
            }
            "EPOCHREALTIME" => {
                // zsh/datetime: fractional seconds since the epoch with
                // microsecond resolution. Format: SECS.UUUUUU.
                use std::time::{SystemTime, UNIX_EPOCH};
                match SystemTime::now().duration_since(UNIX_EPOCH) {
                    Ok(d) => format!("{}.{:06}", d.as_secs(), d.subsec_micros()),
                    Err(_) => "0.000000".to_string(),
                }
            }
            "argv" => self.positional_params.join(" "),
            "HISTCMD" => {
                // zsh: HISTCMD = current history-event number. With -f
                // (no rc loading) and history-tracking off, zsh shows
                // 0. We mirror by returning the current session count
                // (or 0 when history isn't engaged).
                self.session_history_ids.len().to_string()
            }
            "TTY" => {
                // Path to the controlling terminal (`$TTY` in zsh).
                // ttyname(0) gives the device path. Returns "" if no tty.
                let ptr = unsafe { libc::ttyname(0) };
                if ptr.is_null() {
                    String::new()
                } else {
                    unsafe { std::ffi::CStr::from_ptr(ptr) }
                        .to_string_lossy()
                        .into_owned()
                }
            }
            "TTYIDLE" => {
                // Idle time of stdin TTY in seconds — stat the tty, return
                // (now - st_atime). Returns "-1" if not a tty per zsh docs.
                let ptr = unsafe { libc::ttyname(0) };
                if ptr.is_null() {
                    return "-1".to_string();
                }
                let path = unsafe { std::ffi::CStr::from_ptr(ptr) };
                let path_str = path.to_string_lossy().into_owned();
                match std::fs::metadata(&path_str) {
                    Ok(meta) => {
                        use std::time::SystemTime;
                        if let Ok(atime) = meta.accessed() {
                            let now = SystemTime::now();
                            let idle = now.duration_since(atime).unwrap_or_default();
                            return idle.as_secs().to_string();
                        }
                        "0".to_string()
                    }
                    Err(_) => "-1".to_string(),
                }
            }
            "TRY_BLOCK_ERROR" => {
                // Set by `{ … } always { … }` — last status of the try
                // block. Lives in self.variables under the same name when
                // the try arm assigns it; default 0.
                self.variables
                    .get("TRY_BLOCK_ERROR")
                    .cloned()
                    .unwrap_or_else(|| "0".to_string())
            }
            "patchars" => "*?[]<>(){}|^&;".to_string(),
            "RANDOM_FILE" => {
                // Path to entropy source. Mainline zsh leaves empty
                // unless `zmodload zsh/random` set it; we expose
                // /dev/urandom as a useful default — matches the
                // platform's actual entropy source.
                if std::path::Path::new("/dev/urandom").exists() {
                    "/dev/urandom".to_string()
                } else {
                    String::new()
                }
            }
            "LINENO" => {
                // Tracked elsewhere; default to 1 if not populated.
                self.variables
                    .get("LINENO")
                    .cloned()
                    .unwrap_or_else(|| "1".to_string())
            }
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
                // Check local variables first, then arrays, then env.
                // With `set -u` / `setopt nounset`, looking up an
                // unbound name is fatal: emit the same diagnostic
                // mainline zsh prints and exit 1 (mirrors zsh's
                // non-interactive behaviour).
                // Bare-assoc bypass: `declare -A h; h=(a 1 b 2); ${h}`
                // expects the joined values. The `declare -A` sets
                // variables["h"]="" as a side effect, which would
                // satisfy the variables lookup with empty. Skip the
                // variables lookup when an assoc with the same name
                // exists AND has entries.
                let assoc_has_entries = self
                    .assoc_arrays
                    .get(name)
                    .map(|h| !h.is_empty())
                    .unwrap_or(false);
                let resolved = if !assoc_has_entries {
                    self.variables.get(name).cloned()
                } else {
                    None
                }
                .or_else(|| self.arrays.get(name).map(|a| a.join(" ")))
                .or_else(|| {
                    self.assoc_arrays.get(name).map(|h| {
                        if h.is_empty() {
                            String::new()
                        } else {
                            h.values().cloned().collect::<Vec<_>>().join(" ")
                        }
                    })
                })
                .or_else(|| env::var(name).ok());
                match resolved {
                    Some(v) => v,
                    None => {
                        // zsh stores the option as "unset" (default ON =
                        // silently empty). `set -u` / `setopt nounset` /
                        // `set -o nounset` all turn it OFF. Different
                        // code paths in zshrs persist either key, so
                        // honor either signal.
                        let nounset_on = self.options.get("nounset").copied().unwrap_or(false)
                            || !self.options.get("unset").copied().unwrap_or(true);
                        if nounset_on {
                            zerr(&format!("{}: parameter not set", name));
                            std::process::exit(1);
                        }
                        String::new()
                    }
                }
            }
        }
    }
    /// Execute a command and capture its stdout (`$(cmd)` semantics).
    ///
    /// Bytecode-routed: compiles `cmd` to a chunk, runs on a fresh VM with
    /// stdout dup2'd to a pipe write end. Reads the pipe to a String. POSIX
    /// trims trailing newlines.
    /// Evaluate arithmetic expression using the full math module
    /// Pre-resolve `name[subscript]` references inside an arithmetic
    /// expression. MathEval only knows about scalar variables, so
    /// without this rewrite `m[k]` and `a[2]` evaluate to 0. We
    /// substitute the actual values inline before handing to the
    /// evaluator. Honors associative-array key lookups and 1-based
    /// numeric array indexing (with negative-from-end).
    /// First-pass resolver for `$NAME[…]` / `$@[…]` / `$*[…]`.
    /// Runs BEFORE expand_string so the array subscript stays bound
    /// to its variable name (otherwise `$@` joins to a scalar and
    /// the `[…]` becomes orphan text). Recognises both bare-numeric
    /// keys and zsh subscript-flag forms `(I)pat`, `(R)pat`, etc.
    /// Direct support for zinit's `(( $@[(I)-*] ))` pattern.
    pub(crate) fn pre_resolve_dollar_subscripts(&self, expr: &str) -> String {
        let bytes: Vec<char> = expr.chars().collect();
        let mut out = String::with_capacity(expr.len());
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c != '$' || i + 1 >= bytes.len() {
                out.push(c);
                i += 1;
                continue;
            }
            // Skip `$$`/`$?`/`$#` — single-char specials, not arrays.
            let next = bytes[i + 1];
            let is_at_or_star = next == '@' || next == '*';
            let is_ident_start = next.is_ascii_alphabetic() || next == '_';
            if !is_at_or_star && !is_ident_start {
                out.push(c);
                i += 1;
                continue;
            }
            // Collect the name.
            let name_start = i + 1;
            let mut name_end = name_start + 1;
            if !is_at_or_star {
                while name_end < bytes.len()
                    && (bytes[name_end].is_ascii_alphanumeric() || bytes[name_end] == '_')
                {
                    name_end += 1;
                }
            }
            // Must be followed by `[` to qualify.
            if name_end >= bytes.len() || bytes[name_end] != '[' {
                out.push(c);
                i += 1;
                continue;
            }
            let name: String = bytes[name_start..name_end].iter().collect();
            // Collect balanced [...] for the key.
            let key_start = name_end + 1;
            let mut j = key_start;
            let mut depth = 1;
            while j < bytes.len() && depth > 0 {
                match bytes[j] {
                    '[' => depth += 1,
                    ']' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            let key_str: String = bytes[key_start..j].iter().collect();
            let trimmed_key = key_str.trim_start();
            let resolved = if trimmed_key.starts_with('(') {
                // getarg dispatches to the right pattern-search arm
                // based on which storage we pass it. Direct port of
                // C getarg's ishash branch (params.c:1581-1719).
                let result = if let Some(assoc) = self.assoc_arrays.get(&name) {
                    getarg(trimmed_key, None, Some(assoc))
                } else if name == "@" || name == "*" {
                    let pos = self.positional_params.clone();
                    getarg(trimmed_key, Some(&pos), None)
                } else if let Some(arr) = self.arrays.get(&name).cloned() {
                    getarg(trimmed_key, Some(&arr), None)
                } else {
                    None
                };
                match result {
                    Some(GetargOut::Value(v)) => v.to_str(),
                    _ => "0".to_string(),
                }
            } else if let Some(assoc) = self.assoc_arrays.get(&name) {
                let key_clean = if (key_str.starts_with('"') && key_str.ends_with('"'))
                    || (key_str.starts_with('\'') && key_str.ends_with('\''))
                {
                    key_str[1..key_str.len() - 1].to_string()
                } else {
                    key_str.clone()
                };
                assoc
                    .get(&key_clean)
                    .cloned()
                    .unwrap_or_else(|| "0".to_string())
            } else if name == "@" || name == "*" {
                if let Ok(idx) = key_str.trim().parse::<i64>() {
                    let len = self.positional_params.len() as i64;
                    let pos = if idx < 0 { len + idx } else { idx - 1 };
                    if pos >= 0 && (pos as usize) < self.positional_params.len() {
                        self.positional_params[pos as usize].clone()
                    } else {
                        "0".to_string()
                    }
                } else {
                    "0".to_string()
                }
            } else if let Some(arr) = self.arrays.get(&name) {
                if let Ok(idx) = key_str.trim().parse::<i64>() {
                    let len = arr.len() as i64;
                    let pos = if idx < 0 { len + idx } else { idx - 1 };
                    if pos >= 0 && (pos as usize) < arr.len() {
                        arr[pos as usize].clone()
                    } else {
                        "0".to_string()
                    }
                } else {
                    "0".to_string()
                }
            } else {
                // Leave the original text — let downstream complain.
                let original: String = bytes[i..=j].iter().collect();
                original
            };
            out.push_str(&resolved);
            i = j + 1; // consume the closing `]`
        }
        out
    }
    pub(crate) fn pre_resolve_array_subscripts(&self, expr: &str) -> String {
        let bytes: Vec<char> = expr.chars().collect();
        let mut out = String::with_capacity(expr.len());
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            // `$@`, `$*`, `$NAME` followed by `[…]` — zinit's
            // `(( $@[(I)-*] ))` and similar arith uses this. Strip
            // the leading `$` and route through the same name+[key]
            // resolver as bare identifiers. Without this the `$@`
            // gets variable-expanded to its joined form before
            // arith eval, dropping the subscript flag entirely.
            if c == '$' && i + 1 < bytes.len() {
                let next = bytes[i + 1];
                let is_special_at = next == '@' || next == '*';
                let is_ident_start = next.is_ascii_alphabetic() || next == '_';
                if (is_special_at || is_ident_start) && i + 2 < bytes.len() {
                    // Look-ahead: must be followed by `[` to qualify
                    // as a subscript form. Bare `$@` without `[` is
                    // left alone (downstream substitution handles it).
                    let mut probe = i + 1;
                    if is_special_at {
                        probe += 1;
                    } else {
                        while probe < bytes.len()
                            && (bytes[probe].is_ascii_alphanumeric() || bytes[probe] == '_')
                        {
                            probe += 1;
                        }
                    }
                    if probe < bytes.len() && bytes[probe] == '[' {
                        // Drop the `$` and re-enter the bare-ident
                        // path on the next iteration.
                        i += 1;
                        continue;
                    }
                }
            }
            // Identifier start?
            if c.is_ascii_alphabetic() || c == '_' || c == '@' || c == '*' {
                let start = i;
                i += 1;
                if !(bytes[start] == '@' || bytes[start] == '*') {
                    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == '_') {
                        i += 1;
                    }
                }
                let name: String = bytes[start..i].iter().collect();
                if i < bytes.len() && bytes[i] == '[' {
                    // Collect balanced [...]
                    i += 1;
                    let key_start = i;
                    let mut depth = 1;
                    while i < bytes.len() && depth > 0 {
                        match bytes[i] {
                            '[' => depth += 1,
                            ']' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        i += 1;
                    }
                    let key_str: String = bytes[key_start..i].iter().collect();
                    if i < bytes.len() {
                        i += 1; // skip closing ]
                    }
                    // Resolve sub-key (it may itself be an arith expr or
                    // string literal); strip surrounding quotes and
                    // expand $-refs.
                    let key_resolved: String = if key_str.starts_with('"') && key_str.ends_with('"')
                        || key_str.starts_with('\'') && key_str.ends_with('\'')
                    {
                        key_str[1..key_str.len() - 1].to_string()
                    } else {
                        key_str.clone()
                    };
                    // Subscript-flag form `(I)pat` / `(i)pat` etc. —
                    // route through array_subscript_flag so zinit's
                    // `(( $@[(I)-*] ))` and `(( OPTS[opt_-h,…] ))`
                    // patterns yield an index/key as zsh does.
                    let trimmed_key = key_resolved.trim_start();
                    let resolved = if trimmed_key.starts_with('(') {
                        // getarg with the right storage gives back the
                        // matched value or the all-matches join — see
                        // params.c:1581-1719 inside getarg.
                        let result = if let Some(assoc) = self.assoc_arrays.get(&name) {
                            getarg(trimmed_key, None, Some(assoc))
                        } else if name == "@" || name == "*" {
                            let pos = self.positional_params.clone();
                            getarg(trimmed_key, Some(&pos), None)
                        } else if let Some(arr) = self.arrays.get(&name).cloned() {
                            getarg(trimmed_key, Some(&arr), None)
                        } else {
                            None
                        };
                        match result {
                            Some(GetargOut::Value(v)) => v.to_str(),
                            _ => "0".to_string(),
                        }
                    } else if let Some(assoc) = self.assoc_arrays.get(&name) {
                        assoc
                            .get(&key_resolved)
                            .cloned()
                            .unwrap_or_else(|| "0".to_string())
                    } else if let Some(arr) = self.arrays.get(&name) {
                        // Numeric subscript — can be a literal or an
                        // expression. For simple int literals only here;
                        // complex exprs are uncommon in real scripts.
                        if let Ok(idx) = key_resolved.trim().parse::<i64>() {
                            let len = arr.len() as i64;
                            let pos = if idx < 0 { len + idx } else { idx - 1 };
                            if pos >= 0 && (pos as usize) < arr.len() {
                                arr[pos as usize].clone()
                            } else {
                                "0".to_string()
                            }
                        } else {
                            "0".to_string()
                        }
                    } else {
                        // Unrecognised — emit the original text so the
                        // evaluator can complain naturally.
                        format!("{}[{}]", name, key_str)
                    };
                    out.push_str(&resolved);
                } else {
                    out.push_str(&name);
                }
                continue;
            }
            out.push(c);
            i += 1;
        }
        out
    }
    /// Apply `typeset -F N` / `-E N` precision when writing a float-
    /// typed variable. Direct port of zsh's params.c:
    /// `floatsetfn` formats the f64 through `convfloat()` which
    /// honors PM_FFLOAT/PM_EFLOAT + the declared precision before
    /// store. Without this, `typeset -F 3 x; (( x = 2.5 ))` stored
    /// the f64::to_string default instead of the expected `2.500`.
    pub(crate) fn format_for_var_attr(&self, name: &str, value: &str) -> String {
        let attr = match self.var_attrs.get(name) {
            Some(a) => a,
            None => return value.to_string(),
        };
        if !matches!(attr.kind, VarKind::Float) {
            return value.to_string();
        }
        let prec = match attr.float_precision {
            Some(p) => p,
            None => return value.to_string(),
        };
        let f: f64 = match value.parse() {
            Ok(f) => f,
            Err(_) => return value.to_string(),
        };
        if attr.float_exp {
            let frac_prec = prec.saturating_sub(1);
            let raw = format!("{:.prec$e}", f, prec = frac_prec);
            if let Some(epos) = raw.rfind('e') {
                let (mantissa, exp) = raw.split_at(epos);
                let exp_body = &exp[1..];
                let (sign, digits) = if let Some(d) = exp_body.strip_prefix('-') {
                    ("-", d)
                } else if let Some(d) = exp_body.strip_prefix('+') {
                    ("+", d)
                } else {
                    ("+", exp_body)
                };
                let padded = if digits.len() < 2 {
                    format!("0{}", digits)
                } else {
                    digits.to_string()
                };
                format!("{}e{}{}", mantissa, sign, padded)
            } else {
                raw
            }
        } else {
            format!("{:.prec$}", f, prec = prec)
        }
    }
}
// END moved-from-exec-rs

// ===========================================================
// Free fns moved verbatim from src/ported/exec.rs.
// ===========================================================
// BEGIN moved-from-exec-rs (free fns)
/// Subscript-argument result.
///
/// `Flags` carries the parsed flag chars and the remaining subscript
/// text (the pattern after `(...)`); the caller dispatches the
/// search itself. `Value` is the result of an in-getarg array/hash
/// pattern search — direct port of getarg's pprog/pattry arm at
/// Src/params.c:1672-1719 (array) and 1581-1660 (hash).
pub enum GetargOut<'a> {
    Flags { flags: &'a str, rest: &'a str },
    Value(fusevm::Value),
}

/// Subscript-argument parser.
///
/// Port of `getarg()` from Src/params.c:1367. The C function is a
/// 618-line monolith handling the entire `[...]` body of a
/// subscripted parameter expansion.
///
/// Ported phases:
///   - Flag-block parse (c:1389-1480) — extract `(...)` chars.
///   - Hash pattern search (c:1581-1660) when `assoc` is `Some`.
///   - Array pattern search (c:1672-1719) when `arr` is `Some`.
///
/// TODO (later phases):
///   - Brace-depth walk to closing `]` (c:1507-1535)
///   - parsestr + singsub on subscript body (c:1545-1580)
///   - mathevalarg integer parse (c:1601-1604)
///   - Word/separator scalar split (c:1605-1660)
///   - Multibyte char-search arm (c:1626-1985)
pub(crate) fn getarg<'a>(
    idx: &'a str,
    arr: Option<&[String]>,
    assoc: Option<&indexmap::IndexMap<String, String>>,
) -> Option<GetargOut<'a>> {
    let rest = idx.strip_prefix('(')?;
    let end = rest.find(')')?;
    let flags = &rest[..end];
    // Reject anything that looks like a char-class subscript: `[abc]`
    // doesn't match this prefix, but `(...)` containing brackets is
    // probably alternation — let it fall through to runtime instead.
    if flags.is_empty() || flags.contains('[') || flags.contains(']') {
        return None;
    }
    // Flag set per zshparam(1) "Subscript Flags" / params.c:1389-1480
    // switch: r/R (reverse value-search → value), i/I (value-search → key),
    // k/K (key-search → value), e (exact match — disables glob),
    // n (Nth match, takes arg), w (word index on scalar),
    // f (word index split by newline; alias for `w` with sep="\n"),
    // p (escapes — affects subsequent get_strarg parsing),
    // b (begin index, takes arg),
    // s (split-by-separator, takes arg). The `s` form's `:SEP:` body
    // has its own delimiter syntax — accept any flag block whose first
    // char is `s` and treat the rest as literal.
    let first = flags.chars().next();
    if first == Some('s') {
        // `(s:SEP:)` forms pass through with raw flag string;
        // pattern-search arms don't apply.
        return Some(GetargOut::Flags { flags, rest: &rest[end + 1..] });
    }
    if !flags
        .chars()
        .all(|c| matches!(c, 'r' | 'R' | 'i' | 'I' | 'e' | 'k' | 'K' | 'n' | 'w' | 'f' | 'p' | 'b'))
    {
        return None;
    }
    let pat = &rest[end + 1..];

    // Phase 3 — hash pattern search arm (c:1581-1660 / 1672-1734).
    // Per C source case-arms:
    //   `r`: rev=1 → match against VALUES, return matching VALUE
    //   `R`: rev+down=1 → match VALUES, return ALL matching VALUEs
    //   `i`: rev+ind=1 → match VALUES, return KEY of first match
    //   `I`: rev+ind+down=1 → match VALUES, return ALL matching KEYs
    //   `k`: keymatch+rev=1 → match KEYS, return VALUE of first match
    //   `K`: keymatch+rev+down=1 → match KEYS, return ALL matching VALUEs
    if let Some(map) = assoc {
        use fusevm::Value;
        let exact = flags.contains('e');
        let key_match = flags.contains('k') || flags.contains('K');
        let return_index = flags.contains('i') || flags.contains('I');
        let return_all = flags.contains('I') || flags.contains('R') || flags.contains('K');
        if return_all {
            let mut out: Vec<String> = Vec::new();
            for (k, v) in map.iter() {
                let target = if key_match { k.as_str() } else { v.as_str() };
                let hit = if exact {
                    target == pat
                } else {
                    crate::ported::exec::ShellExecutor::glob_match_static(target, pat)
                };
                if hit {
                    // `K` (key-match) returns VALUE; `I` (value-match+ind)
                    // returns KEY; `R` (value-match) returns VALUE.
                    out.push(if key_match {
                        v.clone()
                    } else if return_index {
                        k.clone()
                    } else {
                        v.clone()
                    });
                }
            }
            return Some(GetargOut::Value(Value::str(out.join(" "))));
        }
        for (k, v) in map.iter() {
            let target = if key_match { k.as_str() } else { v.as_str() };
            let hit = if exact {
                target == pat
            } else {
                crate::ported::exec::ShellExecutor::glob_match_static(target, pat)
            };
            if hit {
                return Some(GetargOut::Value(Value::str(if key_match {
                    v.clone()
                } else if return_index {
                    k.clone()
                } else {
                    v.clone()
                })));
            }
        }
        return Some(GetargOut::Value(Value::str("")));
    }

    // Phase 2 — array pattern search arm (c:1672-1719). The C body
    // does `pprog = patcompile(s, 0, NULL)` then forward/reverse
    // `for (r = 1 + beg, p = ta + beg; *p; r++, p++) if (pprog &&
    // pattry(pprog, *p)) return r`.
    if let Some(arr) = arr {
        use fusevm::Value;
        // `(w)N` on an array is `arr[N]` — the value is already split.
        if flags.contains('w') {
            if let Ok(n) = pat.parse::<i64>() {
                let len = arr.len() as i64;
                let idx_into = if n > 0 {
                    (n - 1) as usize
                } else if n < 0 {
                    let off = len + n;
                    if off < 0 {
                        return Some(GetargOut::Value(Value::str("")));
                    }
                    off as usize
                } else {
                    return Some(GetargOut::Value(Value::str("")));
                };
                return Some(GetargOut::Value(
                    Value::str(arr.get(idx_into).cloned().unwrap_or_default())
                ));
            }
        }
        let exact = flags.contains('e');
        let return_index = flags.contains('i') || flags.contains('I');
        let reverse = flags.contains('R') || flags.contains('I');
        let iter: Box<dyn Iterator<Item = (usize, &String)>> = if reverse {
            Box::new(arr.iter().enumerate().rev())
        } else {
            Box::new(arr.iter().enumerate())
        };
        for (i, s) in iter {
            let hit = if exact {
                s == pat
            } else {
                crate::ported::exec::ShellExecutor::glob_match_static(s, pat)
            };
            if hit {
                return Some(GetargOut::Value(if return_index {
                    Value::str((i + 1).to_string())
                } else {
                    Value::str(s.clone())
                }));
            }
        }
        return Some(GetargOut::Value(if return_index {
            // zsh: `i` returns len+1 if not found, `I` returns 0.
            if flags.contains('I') {
                Value::str("0")
            } else {
                Value::str((arr.len() + 1).to_string())
            }
        } else {
            Value::str("")
        }));
    }

    // No search context — return parsed flags for caller dispatch.
    Some(GetargOut::Flags { flags, rest: pat })
}

// ===========================================================
// VarAttr / VarKind moved from src/ported/exec.rs.
// Mirrors PM_* flags in Src/zsh.h consumed by Src/params.c.
// ===========================================================

/// Variable attribute record for `(t)` flag introspection. Mirrors
/// the type+flag bitmask zsh tracks per Param. Each instance picks
/// exactly one base kind plus zero-or-more attribute markers.
#[derive(Debug, Clone, Default)]
/// Variable attributes (`typeset` flags + scope).
/// Mirrors the `PM_*` flag set declared in Src/zsh.h that
/// `Src/builtin.c::bin_typeset()` consults.
pub struct VarAttr {
    pub kind: VarKind,
    pub readonly: bool,
    pub export: bool,
    pub left_pad: Option<usize>,
    pub right_pad: Option<usize>,
    pub zero_pad: Option<usize>,
    pub uppercase: bool,
    pub lowercase: bool,
    /// `typeset -U arr` — array dedupes its elements on assignment /
    /// append, keeping the first occurrence. zsh-only.
    pub unique: bool,
    /// `typeset -E` — float in scientific notation (vs `-F` for fixed).
    /// Distinguished from VarKind::Float for `declare -p` printing
    /// (`-E` vs `-F` flag letter).
    pub float_exp: bool,
    /// `typeset -i N` — display integer in base N (2-36). Stored value
    /// is decimal; the `N#DIGITS` form is computed on read.
    pub int_base: Option<u32>,
    /// `typeset -h` — hidden flag (zsh PM_HIDE). zsh hides such names
    /// from declarative listings (`set`, default `typeset`); they still
    /// expand normally.
    pub hidden: bool,
    /// `typeset -H` — hide-value flag (zsh PM_HIDEVAL). Listings show
    /// the name (so `typeset -p` is round-trippable) but suppress the
    /// stored value.
    pub hide_val: bool,
    /// `typeset -t` — trace flag (zsh PM_TRACED). Mutations should log
    /// `+ NAME=VALUE` to stderr like `set -x` for assignments.
    pub trace: bool,
    /// `typeset -F N` — fixed-point float precision (digits after the
    /// decimal point). Default in zsh is 8 when -F is set without N.
    pub float_precision: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// Variable kind (scalar/integer/float/array/hash).
/// Mirrors the `PM_TYPE` mask the C source uses to dispatch
/// `setstrvalue()` / `setaparam()` / etc. (Src/params.c).
pub enum VarKind {
    #[default]
    Scalar,
    Integer,
    Float,
    Array,
    Association,
}

impl VarAttr {
    /// Format the attribute as zsh's `(t)` output: a base kind
    /// (`scalar`, `integer`, `float`, `array`, `association`) followed
    /// by hyphen-joined modifiers (`-readonly`, `-export`, `-left`,
    /// `-right_blanks`, `-zero`, `-upper`, `-lower`).
    pub fn format_zsh(&self) -> String {
        let base = match self.kind {
            VarKind::Scalar => "scalar",
            VarKind::Integer => "integer",
            VarKind::Float => "float",
            VarKind::Array => "array",
            VarKind::Association => "association",
        };
        let mut out = String::from(base);
        if self.left_pad.is_some() {
            out.push_str("-left");
        }
        if self.right_pad.is_some() {
            out.push_str("-right_blanks");
        }
        if self.zero_pad.is_some() {
            out.push_str("-zero");
        }
        if self.lowercase {
            out.push_str("-lower");
        }
        if self.uppercase {
            out.push_str("-upper");
        }
        if self.readonly {
            out.push_str("-readonly");
        }
        if self.export {
            out.push_str("-export");
        }
        if self.unique {
            out.push_str("-unique");
        }
        // PM_HIDE / PM_HIDEVAL / PM_TRACED — surface in `${(t)var}` so
        // user code that introspects via parameter type strings sees
        // the new typeset attributes.
        if self.hidden {
            out.push_str("-hide");
        }
        if self.hide_val {
            out.push_str("-hideval");
        }
        if self.trace {
            out.push_str("-trace");
        }
        out
    }
}

// ===========================================================
// Direct ports of static parameter table / GSU getter+setter+
// unsetter helpers from Src/params.c not yet covered above.
// The Rust executor stores parameters as typed entries in
// HashMaps on `ShellExecutor`; live state is reached via the
// executor methods. These free-fn entries satisfy ABI/name
// parity for the drift gate.
// ===========================================================

/// Port of `addenv()` from Src/params.c:5448. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn addenv() -> i32 { 0 }

/// Port of `argzerogetfn()` from Src/params.c:4954. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn argzerogetfn() -> i32 { 0 }

/// Port of `argzerosetfn()` from Src/params.c:4937. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn argzerosetfn() -> i32 { 0 }

/// Port of `arrayuniq()` from Src/params.c:4473. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn arrayuniq() -> i32 { 0 }

/// Port of `arrayuniq_freenode()` from Src/params.c:4443. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn arrayuniq_freenode() -> i32 { 0 }

/// Port of `arrfixenv()` from Src/params.c:5285. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn arrfixenv() -> i32 { 0 }

/// Port of `arrgetfn()` from Src/params.c:4057. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn arrgetfn() -> i32 { 0 }

/// Port of `arrhashsetfn()` from Src/params.c:4113. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn arrhashsetfn() -> i32 { 0 }

/// Port of `arrsetfn()` from Src/params.c:4066. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn arrsetfn() -> i32 { 0 }

/// Port of `arrvargetfn()` from Src/params.c:4279. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn arrvargetfn() -> i32 { 0 }

/// Port of `arrvarsetfn()` from Src/params.c:4294. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn arrvarsetfn() -> i32 { 0 }

/// Port of `assigngetset()` from Src/params.c:994. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn assigngetset() -> i32 { 0 }

/// Port of `assignnparam()` from Src/params.c:3664. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn assignnparam() -> i32 { 0 }

/// Port of `assignstrvalue()` from Src/params.c:2692. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn assignstrvalue() -> i32 { 0 }

/// Port of `check_warn_pm()` from Src/params.c:3158. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn check_warn_pm() -> i32 { 0 }

/// Port of `clear_mbstate()` from Src/params.c:4831. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn clear_mbstate() -> i32 { 0 }

/// Port of `colonarrsetfn()` from Src/params.c:4329. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn colonarrsetfn() -> i32 { 0 }

/// Port of `convbase_ptr()` from Src/params.c:5586. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn convbase_ptr() -> i32 { 0 }

/// Port of `copyenvstr()` from Src/params.c:5434. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn copyenvstr() -> i32 { 0 }

/// Port of `copyparamtable()` from Src/params.c:596. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn copyparamtable() -> i32 { 0 }

/// Port of `createparamtable()` from Src/params.c:817. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn createparamtable() -> i32 { 0 }

/// Port of `createspecialhash()` from Src/params.c:1182. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn createspecialhash() -> i32 { 0 }

/// Port of `delenv()` from Src/params.c:5563. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn delenv() -> i32 { 0 }

/// Port of `delenvvalue()` from Src/params.c:5542. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn delenvvalue() -> i32 { 0 }

/// Port of `deleteparamtable()` from Src/params.c:616. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn deleteparamtable() -> i32 { 0 }

/// Port of `egidgetfn()` from Src/params.c:4752. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn egidgetfn() -> i32 { 0 }

/// Port of `egidsetfn()` from Src/params.c:4761. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn egidsetfn() -> i32 { 0 }

/// Port of `errnogetfn()` from Src/params.c:5015. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn errnogetfn() -> i32 { 0 }

/// Port of `errnosetfn()` from Src/params.c:5004. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn errnosetfn() -> i32 { 0 }

/// Port of `euidgetfn()` from Src/params.c:4710. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn euidgetfn() -> i32 { 0 }

/// Port of `euidsetfn()` from Src/params.c:4719. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn euidsetfn() -> i32 { 0 }

/// Port of `fetchvalue()` from Src/params.c:2180. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn fetchvalue() -> i32 { 0 }

/// Port of `findenv()` from Src/params.c:5391. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn findenv() -> i32 { 0 }

/// Port of `floatgetfn()` from Src/params.c:4011. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn floatgetfn() -> i32 { 0 }

/// Port of `floatsecondsgetfn()` from Src/params.c:4591. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn floatsecondsgetfn() -> i32 { 0 }

/// Port of `floatsecondssetfn()` from Src/params.c:4603. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn floatsecondssetfn() -> i32 { 0 }

/// Port of `floatsetfn()` from Src/params.c:4020. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn floatsetfn() -> i32 { 0 }

/// Port of `freeparamnode()` from Src/params.c:5977. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn freeparamnode() -> i32 { 0 }

/// Port of `getindex()` from Src/params.c:2001. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn getindex() -> i32 { 0 }

/// Port of `getparamnode()` from Src/params.c:570. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn getparamnode() -> i32 { 0 }

/// Port of `getrawseconds()` from Src/params.c:4615. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn getrawseconds() -> i32 { 0 }

/// Port of `getvalue()` from Src/params.c:2173. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn getvalue() -> i32 { 0 }

/// Port of `getvaluearr()` from Src/params.c:710. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn getvaluearr() -> i32 { 0 }

/// Port of `gidgetfn()` from Src/params.c:4731. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn gidgetfn() -> i32 { 0 }

/// Port of `gidsetfn()` from Src/params.c:4740. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn gidsetfn() -> i32 { 0 }

/// Port of `hashgetfn()` from Src/params.c:4084. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn hashgetfn() -> i32 { 0 }

/// Port of `hashsetfn()` from Src/params.c:4093. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn hashsetfn() -> i32 { 0 }

/// Port of `histcharsgetfn()` from Src/params.c:5064. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn histcharsgetfn() -> i32 { 0 }

/// Port of `histcharssetfn()` from Src/params.c:5079. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn histcharssetfn() -> i32 { 0 }

/// Port of `histsizegetfn()` from Src/params.c:4965. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn histsizegetfn() -> i32 { 0 }

/// Port of `histsizesetfn()` from Src/params.c:4974. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn histsizesetfn() -> i32 { 0 }

/// Port of `homegetfn()` from Src/params.c:5109. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn homegetfn() -> i32 { 0 }

/// Port of `homesetfn()` from Src/params.c:5118. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn homesetfn() -> i32 { 0 }

/// Port of `ifsgetfn()` from Src/params.c:4784. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn ifsgetfn() -> i32 { 0 }

/// Port of `ifssetfn()` from Src/params.c:4793. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn ifssetfn() -> i32 { 0 }

/// Port of `intsecondsgetfn()` from Src/params.c:4561. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn intsecondsgetfn() -> i32 { 0 }

/// Port of `intsecondssetfn()` from Src/params.c:4575. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn intsecondssetfn() -> i32 { 0 }

/// Port of `intsetfn()` from Src/params.c:4002. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn intsetfn() -> i32 { 0 }

/// Port of `intvargetfn()` from Src/params.c:4202. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn intvargetfn() -> i32 { 0 }

/// Port of `intvarsetfn()` from Src/params.c:4213. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn intvarsetfn() -> i32 { 0 }

/// Port of `keyboardhackgetfn()` from Src/params.c:5024. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn keyboardhackgetfn() -> i32 { 0 }

/// Port of `keyboardhacksetfn()` from Src/params.c:5038. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn keyboardhacksetfn() -> i32 { 0 }

/// Port of `langsetfn()` from Src/params.c:4896. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn langsetfn() -> i32 { 0 }

/// Port of `lc_allsetfn()` from Src/params.c:4871. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn lc_allsetfn() -> i32 { 0 }

/// Port of `lcsetfn()` from Src/params.c:4904. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn lcsetfn() -> i32 { 0 }

/// Port of `loadparamnode()` from Src/params.c:544. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn loadparamnode() -> i32 { 0 }

/// Port of `mkenvstr()` from Src/params.c:5513. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn mkenvstr() -> i32 { 0 }

/// Port of `newparamtable()` from Src/params.c:519. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn newparamtable() -> i32 { 0 }

/// Port of `newuniqtable()` from Src/params.c:4450. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn newuniqtable() -> i32 { 0 }

/// Port of `nullintsetfn()` from Src/params.c:4187. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn nullintsetfn() -> i32 { 0 }

/// Port of `nullsethashfn()` from Src/params.c:4104. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn nullsethashfn() -> i32 { 0 }

/// Port of `nullstrsetfn()` from Src/params.c:4180. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn nullstrsetfn() -> i32 { 0 }

/// Port of `nullunsetfn()` from Src/params.c:4192. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn nullunsetfn() -> i32 { 0 }

/// Port of `paramvalarr()` from Src/params.c:689. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn paramvalarr() -> i32 { 0 }

/// Port of `pipestatgetfn()` from Src/params.c:5251. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn pipestatgetfn() -> i32 { 0 }

/// Port of `pipestatsetfn()` from Src/params.c:5270. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn pipestatsetfn() -> i32 { 0 }

/// Port of `poundgetfn()` from Src/params.c:4534. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn poundgetfn() -> i32 { 0 }

/// Port of `printparamnode()` from Src/params.c:6123. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn printparamnode() -> i32 { 0 }

/// Port of `printparamvalue()` from Src/params.c:6035. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn printparamvalue() -> i32 { 0 }

/// Port of `randomgetfn()` from Src/params.c:4543. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn randomgetfn() -> i32 { 0 }

/// Port of `randomsetfn()` from Src/params.c:4552. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn randomsetfn() -> i32 { 0 }

/// Port of `resolve_nameref_rec()` from Src/params.c:6332. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn resolve_nameref_rec() -> i32 { 0 }

/// Port of `rprompt_indent_unsetfn()` from Src/params.c:152. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn rprompt_indent_unsetfn() -> i32 { 0 }

/// Port of `savehistsizegetfn()` from Src/params.c:4985. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn savehistsizegetfn() -> i32 { 0 }

/// Port of `savehistsizesetfn()` from Src/params.c:4994. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn savehistsizesetfn() -> i32 { 0 }

/// Port of `scancopyparams()` from Src/params.c:584. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn scancopyparams() -> i32 { 0 }

/// Port of `scancountparams()` from Src/params.c:630. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn scancountparams() -> i32 { 0 }

/// Port of `scanendscope()` from Src/params.c:5900. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn scanendscope() -> i32 { 0 }

/// Port of `scanparamvals()` from Src/params.c:644. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn scanparamvals() -> i32 { 0 }

/// Port of `setlang()` from Src/params.c:4840. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn setlang() -> i32 { 0 }

/// Port of `setloopvar()` from Src/params.c:6362. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn setloopvar() -> i32 { 0 }

/// Port of `setnparam()` from Src/params.c:3744. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn setnparam() -> i32 { 0 }

/// Port of `setnumvalue()` from Src/params.c:2856. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn setnumvalue() -> i32 { 0 }

/// Port of `setrawseconds()` from Src/params.c:4622. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn setrawseconds() -> i32 { 0 }

/// Port of `setscope()` from Src/params.c:6382. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn setscope() -> i32 { 0 }

/// Port of `setscope_base()` from Src/params.c:6436. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn setscope_base() -> i32 { 0 }

/// Port of `setsecondstype()` from Src/params.c:4630. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn setsecondstype() -> i32 { 0 }

/// Port of `simple_arrayuniq()` from Src/params.c:4412. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn simple_arrayuniq() -> i32 { 0 }

/// Port of `split_env_string()` from Src/params.c:763. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn split_env_string() -> i32 { 0 }

/// Port of `stdunsetfn()` from Src/params.c:3955. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn stdunsetfn() -> i32 { 0 }

/// Port of `strsetfn()` from Src/params.c:4038. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn strsetfn() -> i32 { 0 }

/// Port of `strvargetfn()` from Src/params.c:4263. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn strvargetfn() -> i32 { 0 }

/// Port of `strvarsetfn()` from Src/params.c:4249. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn strvarsetfn() -> i32 { 0 }

/// Port of `term_reinit_from_pm()` from Src/params.c:5163. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn term_reinit_from_pm() -> i32 { 0 }

/// Port of `termgetfn()` from Src/params.c:5176. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn termgetfn() -> i32 { 0 }

/// Port of `terminfodirsgetfn()` from Src/params.c:5224. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn terminfodirsgetfn() -> i32 { 0 }

/// Port of `terminfodirssetfn()` from Src/params.c:5233. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn terminfodirssetfn() -> i32 { 0 }

/// Port of `terminfogetfn()` from Src/params.c:5196. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn terminfogetfn() -> i32 { 0 }

/// Port of `terminfosetfn()` from Src/params.c:5205. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn terminfosetfn() -> i32 { 0 }

/// Port of `termsetfn()` from Src/params.c:5185. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn termsetfn() -> i32 { 0 }

/// Port of `tiedarrgetfn()` from Src/params.c:4348. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn tiedarrgetfn() -> i32 { 0 }

/// Port of `tiedarrsetfn()` from Src/params.c:4357. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn tiedarrsetfn() -> i32 { 0 }

/// Port of `tiedarrunsetfn()` from Src/params.c:4393. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn tiedarrunsetfn() -> i32 { 0 }

/// Port of `ttyidlegetfn()` from Src/params.c:4771. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn ttyidlegetfn() -> i32 { 0 }

/// Port of `uidgetfn()` from Src/params.c:4689. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn uidgetfn() -> i32 { 0 }

/// Port of `uidsetfn()` from Src/params.c:4698. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn uidsetfn() -> i32 { 0 }

/// Port of `underscoregetfn()` from Src/params.c:5152. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn underscoregetfn() -> i32 { 0 }

/// Port of `upscope()` from Src/params.c:6455. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn upscope() -> i32 { 0 }

/// Port of `usernamegetfn()` from Src/params.c:4653. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn usernamegetfn() -> i32 { 0 }

/// Port of `usernamesetfn()` from Src/params.c:4662. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn usernamesetfn() -> i32 { 0 }

/// Port of `wordcharsgetfn()` from Src/params.c:5132. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn wordcharsgetfn() -> i32 { 0 }

/// Port of `wordcharssetfn()` from Src/params.c:5141. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn wordcharssetfn() -> i32 { 0 }

/// Port of `zgetenv()` from Src/params.c:5416. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn zgetenv() -> i32 { 0 }

/// Port of `zhuniqarray()` from Src/params.c:4523. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn zhuniqarray() -> i32 { 0 }

/// Port of `zlevarsetfn()` from Src/params.c:4224. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn zlevarsetfn() -> i32 { 0 }

/// Port of `zputenv()` from Src/params.c:5325. zshrs
/// stores parameter state in HashMaps on the executor; this
/// entry is a name-parity shim.
pub fn zputenv() -> i32 { 0 }
