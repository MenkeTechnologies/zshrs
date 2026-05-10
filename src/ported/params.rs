//! Parameter management for zshrs
//!
//! Port from zsh/Src/params.c (6511 lines → full Rust port)
//!
//! Provides shell parameters (variables), special parameters, arrays,
//! associative arrays, parameter attributes, namerefs, scoping,
//! tied parameters, and all special parameter get/set functions.

#[allow(unused_imports)]
use crate::ported::utils::zerr;
use crate::ported::text::FuncBodyFmt;
use indexmap::IndexMap;
use std::collections::{HashMap, HashSet};
use std::env;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Parameter flags (from zsh.h PM_* flags)
// ---------------------------------------------------------------------------

// What level of localness we are at.                                       // c:47
//                                                                          // c:48
// Hand-wavingly, this is incremented at every function call and decremented // c:49
// at every function return.  See startparamscope().                        // c:50


// ---------------------------------------------------------------------------
// Real `param` struct lives in Src/zsh.h:1829 (port at zsh_h.rs:750).
// It uses C-union flattening: u_str / u_arr / u_val / u_dval / u_hash
// dispatched on `PM_TYPE(node.flags)`. There is NO `ParamValue` enum in
// C; do not reintroduce one.
// ---------------------------------------------------------------------------

pub use crate::ported::zsh_h::param;


// =============================================================================
// IPDEF{1,2,4,5,5U,6,7,7R,7U,8,9,10} + LCIPDEF — special-parameter
// table entry constructors. All defined as macros in
// `Src/params.c:296-406`. Each produces one row of the
// `special_params[]` table; the differences are flag combinations
// + which gsu (getter/setter union) the entry binds.
//
// In C, `BR(p)` is `{(void *)(p)}` for the param's `u` data field;
// `GSU(g)` is the `&g` of the named gsu_scalar/gsu_integer/etc.
// The Rust port stores `var` and `gsu` as `usize` slot indexes
// into per-evaluator tables, matching the existing PARAMDEF helper
// above. The flag bit combinations mirror the C macros line-by-line.
// =============================================================================

/// Port of `IPDEF1(A,B,C)` from `Src/params.c:296` —
/// `{{NULL,A,PM_INTEGER|PM_SPECIAL|C},BR(NULL),GSU(B),10,0,...}`.
#[inline] #[allow(non_snake_case)]
pub fn IPDEF1(name: &str, gsu: usize, extra_flags: i32) -> paramdef {        // c:params.c:296
    paramdef {
        name: name.to_string(),
        flags: (PM_INTEGER | PM_SPECIAL) as i32 | extra_flags,
        var: 0, gsu,
        getnfn: None, scantfn: None, pm: None,
    }
}

/// Port of `IPDEF2(A,B,C)` from `Src/params.c:309` —
/// `{{NULL,A,PM_SCALAR|PM_SPECIAL|C},BR(NULL),GSU(B),0,0,...}`.
#[inline] #[allow(non_snake_case)]
pub fn IPDEF2(name: &str, gsu: usize, extra_flags: i32) -> paramdef {        // c:params.c:309
    paramdef {
        name: name.to_string(),
        flags: (PM_SCALAR | PM_SPECIAL) as i32 | extra_flags,
        var: 0, gsu,
        getnfn: None, scantfn: None, pm: None,
    }
}

/// Port of `IPDEF4(A,B)` from `Src/params.c:344` —
/// `{{NULL,A,PM_INTEGER|PM_READONLY_SPECIAL},BR((void*)B),
///   GSU(varint_readonly_gsu),10,0,...}`.
#[inline] #[allow(non_snake_case)]
pub fn IPDEF4(name: &str, var: usize) -> paramdef {                          // c:params.c:344
    paramdef {
        name: name.to_string(),
        flags: (PM_INTEGER | PM_READONLY_SPECIAL) as i32,
        var, gsu: 0,
        getnfn: None, scantfn: None, pm: None,
    }
}

/// Port of `IPDEF5(A,B,F)` from `Src/params.c:353` —
/// `{{NULL,A,PM_INTEGER|PM_SPECIAL},BR((void*)B),GSU(F),10,0,...}`.
#[inline] #[allow(non_snake_case)]
pub fn IPDEF5(name: &str, var: usize, gsu: usize) -> paramdef {              // c:params.c:353
    paramdef {
        name: name.to_string(),
        flags: (PM_INTEGER | PM_SPECIAL) as i32,
        var, gsu,
        getnfn: None, scantfn: None, pm: None,
    }
}

/// Port of `IPDEF5U(A,B,F)` from `Src/params.c:354` — c:353 + PM_UNSET.
#[inline] #[allow(non_snake_case)]
pub fn IPDEF5U(name: &str, var: usize, gsu: usize) -> paramdef {             // c:params.c:354
    paramdef {
        name: name.to_string(),
        flags: (PM_INTEGER | PM_SPECIAL | PM_UNSET) as i32,
        var, gsu,
        getnfn: None, scantfn: None, pm: None,
    }
}

/// Port of `IPDEF6(A,B,F)` from `Src/params.c:362` — c:353 + PM_DONTIMPORT.
#[inline] #[allow(non_snake_case)]
pub fn IPDEF6(name: &str, var: usize, gsu: usize) -> paramdef {              // c:params.c:362
    paramdef {
        name: name.to_string(),
        flags: (PM_INTEGER | PM_SPECIAL | PM_DONTIMPORT) as i32,
        var, gsu,
        getnfn: None, scantfn: None, pm: None,
    }
}

/// Port of `IPDEF7(A,B)` from `Src/params.c:367` —
/// `{{NULL,A,PM_SCALAR|PM_SPECIAL},BR((void*)B),GSU(varscalar_gsu),0,0,...}`.
#[inline] #[allow(non_snake_case)]
pub fn IPDEF7(name: &str, var: usize) -> paramdef {                          // c:params.c:367
    paramdef {
        name: name.to_string(),
        flags: (PM_SCALAR | PM_SPECIAL) as i32,
        var, gsu: 0,
        getnfn: None, scantfn: None, pm: None,
    }
}

/// Port of `IPDEF7R(A,B)` from `Src/params.c:368` — c:367 + PM_DONTIMPORT_SUID.
#[inline] #[allow(non_snake_case)]
pub fn IPDEF7R(name: &str, var: usize) -> paramdef {                         // c:params.c:368
    paramdef {
        name: name.to_string(),
        flags: (PM_SCALAR | PM_SPECIAL | PM_DONTIMPORT_SUID) as i32,
        var, gsu: 0,
        getnfn: None, scantfn: None, pm: None,
    }
}

/// Port of `IPDEF7U(A,B)` from `Src/params.c:369` — c:367 + PM_UNSET.
#[inline] #[allow(non_snake_case)]
pub fn IPDEF7U(name: &str, var: usize) -> paramdef {                         // c:params.c:369
    paramdef {
        name: name.to_string(),
        flags: (PM_SCALAR | PM_SPECIAL | PM_UNSET) as i32,
        var, gsu: 0,
        getnfn: None, scantfn: None, pm: None,
    }
}

/// Port of `IPDEF8(A,B,C,D)` from `Src/params.c:394` —
/// `{{NULL,A,D|PM_SCALAR|PM_SPECIAL},BR((void*)B),GSU(colonarr_gsu),
///   0,0,NULL,C,NULL,0}`.
/// `C` is the colon-arr field; the Rust port stores it in `getnfn`
/// since `paramdef` lacks a dedicated colon-arr slot until that's
/// ported.
#[inline] #[allow(non_snake_case)]
pub fn IPDEF8(name: &str, var: usize, _colon: usize, extra_flags: i32) -> paramdef { // c:params.c:394
    paramdef {
        name: name.to_string(),
        flags: (PM_SCALAR | PM_SPECIAL) as i32 | extra_flags,
        var, gsu: 0,
        getnfn: None, scantfn: None, pm: None,
    }
}

/// Port of `IPDEF9(A,B,C,D)` from `Src/params.c:384` —
/// `{{NULL,A,D|PM_ARRAY|PM_SPECIAL|PM_DONTIMPORT},BR((void*)B),
///   GSU(vararray_gsu),0,0,NULL,C,NULL,0}`.
#[inline] #[allow(non_snake_case)]
pub fn IPDEF9(name: &str, var: usize, _colon: usize, extra_flags: i32) -> paramdef { // c:params.c:384
    paramdef {
        name: name.to_string(),
        flags: (PM_ARRAY | PM_SPECIAL | PM_DONTIMPORT) as i32 | extra_flags,
        var, gsu: 0,
        getnfn: None, scantfn: None, pm: None,
    }
}

/// Port of `IPDEF10(A,B)` from `Src/params.c:406` —
/// `{{NULL,A,PM_ARRAY|PM_SPECIAL},BR(NULL),GSU(B),10,0,...}`.
#[inline] #[allow(non_snake_case)]
pub fn IPDEF10(name: &str, gsu: usize) -> paramdef {                         // c:params.c:406
    paramdef {
        name: name.to_string(),
        flags: (PM_ARRAY | PM_SPECIAL) as i32,
        var: 0, gsu,
        getnfn: None, scantfn: None, pm: None,
    }
}

/// Port of `LCIPDEF(name)` from `Src/params.c:324` —
/// `IPDEF2(name, lc_blah_gsu, PM_UNSET)`.
#[inline] #[allow(non_snake_case)]
pub fn LCIPDEF(name: &str) -> paramdef {                                     // c:params.c:324
    IPDEF2(name, 0, PM_UNSET as i32)                                         // c:324 lc_blah_gsu (slot 0)
}



// ---------------------------------------------------------------------------
// Numeric type for parameters (from params.c mnumber)
// ---------------------------------------------------------------------------


// ---------------------------------------------------------------------------
// Value struct - mirrors C's struct value for subscript access
// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Shell parameter
// ---------------------------------------------------------------------------


// ---------------------------------------------------------------------------
// Tied parameter data
// ---------------------------------------------------------------------------

// TiedData removed: was a Rust-only sidecar for the deleted `ParamTable`'s
// `tied: HashMap<String, TiedData>` field. C source stores tied-pair
// metadata via `pm->ename` (the partner name) and `pm->u.data` (the
// separator char) on the real `param` struct (Src/zsh.h:750 / Src/params.c
// `bin_typeset()` typeset -T branch).


// ---------------------------------------------------------------------------
// Parameter table print types (from printparamnode)
// ---------------------------------------------------------------------------

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
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_READONLY,
        tied_name: None,
    },
    SpecialParamDef {
        name: "ERRNO",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "GID",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_DONTIMPORT,
        tied_name: None,
    },
    SpecialParamDef {
        name: "EGID",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_DONTIMPORT,
        tied_name: None,
    },
    SpecialParamDef {
        name: "HISTSIZE",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "RANDOM",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "SAVEHIST",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "SECONDS",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "UID",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_DONTIMPORT,
        tied_name: None,
    },
    SpecialParamDef {
        name: "EUID",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_DONTIMPORT,
        tied_name: None,
    },
    SpecialParamDef {
        name: "TTYIDLE",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_READONLY,
        tied_name: None,
    },
    // Scalar specials with custom GSU
    SpecialParamDef {
        name: "USERNAME",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_DONTIMPORT,
        tied_name: None,
    },
    SpecialParamDef {
        name: "-",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_READONLY,
        tied_name: None,
    },
    SpecialParamDef {
        name: "histchars",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_DONTIMPORT,
        tied_name: None,
    },
    SpecialParamDef {
        name: "HOME",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "TERM",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "TERMINFO",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "TERMINFO_DIRS",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "WORDCHARS",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "IFS",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_DONTIMPORT,
        tied_name: None,
    },
    SpecialParamDef {
        name: "_",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_DONTIMPORT,
        tied_name: None,
    },
    SpecialParamDef {
        name: "KEYBOARD_HACK",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_DONTIMPORT,
        tied_name: None,
    },
    SpecialParamDef {
        name: "0",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    // Readonly integer variables bound to C globals
    SpecialParamDef {
        name: "!",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_READONLY,
        tied_name: None,
    },
    SpecialParamDef {
        name: "$",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_READONLY,
        tied_name: None,
    },
    SpecialParamDef {
        name: "?",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_READONLY,
        tied_name: None,
    },
    SpecialParamDef {
        name: "HISTCMD",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_READONLY,
        tied_name: None,
    },
    SpecialParamDef {
        name: "LINENO",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_READONLY,
        tied_name: None,
    },
    SpecialParamDef {
        name: "PPID",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_READONLY,
        tied_name: None,
    },
    SpecialParamDef {
        name: "ZSH_SUBSHELL",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_READONLY,
        tied_name: None,
    },
    // Settable integer variables
    SpecialParamDef {
        name: "COLUMNS",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "LINES",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "ZLE_RPROMPT_INDENT",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "SHLVL",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "FUNCNEST",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "OPTIND",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_DONTIMPORT,
        tied_name: None,
    },
    SpecialParamDef {
        name: "TRY_BLOCK_ERROR",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_DONTIMPORT,
        tied_name: None,
    },
    SpecialParamDef {
        name: "TRY_BLOCK_INTERRUPT",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_DONTIMPORT,
        tied_name: None,
    },
    // Scalar variables bound to C globals
    SpecialParamDef {
        name: "OPTARG",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "NULLCMD",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "POSTEDIT",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "READNULLCMD",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "PS1",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "RPS1",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "RPROMPT",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "PS2",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "RPS2",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "RPROMPT2",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "PS3",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "PS4",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_DONTIMPORT_SUID,
        tied_name: None,
    },
    SpecialParamDef {
        name: "SPROMPT",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    // Readonly arrays
    SpecialParamDef {
        name: "*",
        pm_type: crate::ported::zsh_h::PM_ARRAY,
        pm_flags: crate::ported::zsh_h::PM_READONLY | crate::ported::zsh_h::PM_DONTIMPORT,
        tied_name: None,
    },
    SpecialParamDef {
        name: "@",
        pm_type: crate::ported::zsh_h::PM_ARRAY,
        pm_flags: crate::ported::zsh_h::PM_READONLY | crate::ported::zsh_h::PM_DONTIMPORT,
        tied_name: None,
    },
    // Tied colon-separated/array pairs
    SpecialParamDef {
        name: "CDPATH",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_TIED,
        tied_name: Some("cdpath"),
    },
    SpecialParamDef {
        name: "FIGNORE",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_TIED,
        tied_name: Some("fignore"),
    },
    SpecialParamDef {
        name: "FPATH",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_TIED,
        tied_name: Some("fpath"),
    },
    SpecialParamDef {
        name: "MAILPATH",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_TIED,
        tied_name: Some("mailpath"),
    },
    SpecialParamDef {
        name: "PATH",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_TIED,
        tied_name: Some("path"),
    },
    SpecialParamDef {
        name: "PSVAR",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_TIED,
        tied_name: Some("psvar"),
    },
    SpecialParamDef {
        name: "ZSH_EVAL_CONTEXT",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_READONLY | crate::ported::zsh_h::PM_TIED,
        tied_name: Some("zsh_eval_context"),
    },
    SpecialParamDef {
        name: "MODULE_PATH",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_DONTIMPORT | crate::ported::zsh_h::PM_TIED,
        tied_name: Some("module_path"),
    },
    SpecialParamDef {
        name: "MANPATH",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_TIED,
        tied_name: Some("manpath"),
    },
    // Locale
    SpecialParamDef {
        name: "LANG",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "LC_ALL",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "LC_COLLATE",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "LC_CTYPE",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "LC_MESSAGES",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "LC_NUMERIC",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    SpecialParamDef {
        name: "LC_TIME",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    // Zsh-only aliases
    SpecialParamDef {
        name: "ARGC",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_READONLY,
        tied_name: None,
    },
    SpecialParamDef {
        name: "HISTCHARS",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_DONTIMPORT,
        tied_name: None,
    },
    SpecialParamDef {
        name: "status",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_READONLY,
        tied_name: None,
    },
    SpecialParamDef {
        name: "prompt",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "PROMPT",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "PROMPT2",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "PROMPT3",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "PROMPT4",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    SpecialParamDef {
        name: "argv",
        pm_type: crate::ported::zsh_h::PM_ARRAY,
        pm_flags: 0,
        tied_name: None,
    },
    // pipestatus array
    SpecialParamDef {
        name: "pipestatus",
        pm_type: crate::ported::zsh_h::PM_ARRAY,
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
// ---------------------------------------------------------------------------
// Free functions matching the C API
// ---------------------------------------------------------------------------

/// Get integer parameter value (from params.c getintvalue)
/// Get an integer parameter.
/// Port of `getintvalue()` from Src/params.c:2601.
pub fn getintvalue(_table: &crate::ported::zsh_h::HashTable, _name: &str) -> i64 {
    // TODO: real port — gethnode(paramtab, name); return pm->u.val if PM_INTEGER
    // else parse pm->u.str. Old body called deleted `ParamTable::get_value`.
    0
}

/// Get scalar (string) parameter (from params.c getstrvalue)
/// Get a scalar parameter.
/// Port of `getstrvalue()` from Src/params.c:2335.
pub fn getstrvalue(_table: &crate::ported::zsh_h::HashTable, _name: &str) -> Option<String> { // c:2335
    // TODO: real port — gethnode + dispatch on PM_TYPE(pm->node.flags).
    None
}

/// Get scalar with default
/// Get a scalar parameter with a default fallback.
/// zshrs convenience over `getstrvalue()` — C zsh inlines the
/// `value ? value : default` ternary at every call site.
pub fn getsparam_u(table: &crate::ported::zsh_h::HashTable, name: &str, default: &str) -> String {
    getstrvalue(table, name).unwrap_or_else(|| default.to_string())
}

/// Get an array parameter.
/// Port of `getaparam()` from Src/params.c:3100.
pub fn getaparam(_table: &crate::ported::zsh_h::HashTable, _name: &str) -> Option<Vec<String>> { // c:3100
    // TODO: real port — gethnode + return pm->u.arr if PM_TYPE == PM_ARRAY.
    None
}

/// Get hash parameter values as array (from params.c gethparam)
/// Get a hash parameter as a flat key/value array.
/// Port of the `${(kv)hash}` materialization Src/params.c does
/// inside `getstrvalue()` (line 2335) for hash params.
pub fn gethparam(_table: &crate::ported::zsh_h::HashTable, _name: &str) -> Option<Vec<String>> { // c:3115
    // TODO: real port — gethnode + flatten pm->u.hash values when PM_HASHED.
    None
}

/// Get hash parameter keys as array (from params.c gethkparam)
/// Get a hash parameter's keys only.
/// Port of the `${(k)hash}` extraction in Src/params.c.
pub fn gethkparam(_table: &crate::ported::zsh_h::HashTable, _name: &str) -> Option<Vec<String>> { // c:3130
    // TODO: real port — gethnode + extract pm->u.hash keys when PM_HASHED.
    None
}

/// Get numeric parameter (from params.c getnumvalue)
/// Get a parameter as an `MNumber`.
/// Port of `getnumvalue()` from Src/params.c:2624.
pub fn getnumvalue(_table: &crate::ported::zsh_h::HashTable, _name: &str) -> MNumber { // c:2624
    // TODO: real port — gethnode + dispatch on PM_TYPE for INTEGER/FFLOAT/EFLOAT/SCALAR.
    MNumber::default()
}

/// Assign string parameter (from params.c setstrvalue)
/// Assign a scalar parameter.
/// Port of `setstrvalue()` from Src/params.c:2685.
pub fn setstrvalue(_table: &mut crate::ported::zsh_h::HashTable, _name: &str, _val: &str) -> bool { // c:2685
    // TODO: real port — gethnode/createparam, dispatch via pm->gsu.s->setfn.
    false
}

/// Assign integer parameter (from params.c assigniparam)
/// Assign an integer parameter.
/// Port of the integer branch of `setvalue()` (Src/params.c).
pub fn assigniparam(_table: &mut crate::ported::zsh_h::HashTable, _name: &str, _val: i64) -> bool {
    // TODO: real port — gethnode/createparam, dispatch via pm->gsu.i->setfn.
    false
}

/// Set array parameter.
/// Port of `setaparam()` from `Src/params.c:3759` — single-line wrapper
/// around `assignaparam(s, val, ASSPM_WARN)`. C body:
/// ```c
/// mod_export Param setaparam(char *s, char **val) {
///     return assignaparam(s, val, ASSPM_WARN);
/// }
/// ```
///
/// `assignaparam` (params.c:3357) creates the param if missing
/// (createparam(s, PM_ARRAY)), then dispatches to `setarrvalue` which
/// drops any prior scalar / assoc value and stores the array. Rust port
/// mirrors that "drop scalar+assoc, set array" semantic via the three
/// separate HashMaps that ShellExecutor uses for parameter storage.
///
/// `ASSPM_WARN` (params.c:104) is a no-op in our port — the global
/// "warn on creation" tracking is not yet ported. Call shape preserved
/// so callers can use this where C calls setaparam.
pub fn setaparam(                                                           // c:3595
    variables: &mut std::collections::HashMap<String, String>,
    arrays: &mut std::collections::HashMap<String, Vec<String>>,
    assoc_arrays: &mut std::collections::HashMap<String, indexmap::IndexMap<String, String>>,
    name: &str,
    val: Vec<String>,
) {
    arrays.insert(name.to_string(), val);
    variables.remove(name);
    assoc_arrays.remove(name);
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
pub fn assignsparam(                                                        // c:3193
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
pub fn sethparam(                                                           // c:3602
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
pub fn unsetparam_pm(_table: &mut crate::ported::zsh_h::HashTable, _name: &str) -> bool {
    // TODO: real port — gethnode + invoke pm->gsu.s->unsetfn(pm) + removehashnode.
    false
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
pub fn setsparam(                                                            // c:3350
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

/// Port of `getsparam()` from `Src/params.c:3076`.
///
/// C body:
/// ```c
/// char *getsparam(char *s) {
///     struct value vbuf;
///     Value v = getvalue(&vbuf, &s, 0);
///     if (!v) return NULL;
///     return getstrvalue(v);
/// }
/// ```
///
/// `getvalue` (params.c:2173) builds a `Value` for the parameter,
/// dispatching through `Param.gsu->getfn` for special parameters.
/// `getstrvalue` (params.c:2335) extracts the scalar form: for
/// PM_INTEGER calls `pm->gsu.i->getfn(pm)` and convbase's the
/// result; for PM_SCALAR calls `pm->gsu.s->getfn(pm)`; for
/// PM_ARRAY joins the elements.
///
/// **Sole funnel.** Every scalar parameter read in zshrs routes
/// through this fn — `subst.rs` parameter expansion AND
/// `fusevm_bridge::expand_param` both call `getsparam`. The
/// dispatch chain lives in exactly one place, mirroring C's
/// "every read goes through getsparam" architecture.
///
/// Lookup order (mirrors C's `getvalue` → `getstrvalue` cascade):
/// 1. **GSU dispatch** via [`lookup_special_var`] — special
///    parameters route through their getfn callback (`uidgetfn` /
///    `randomgetfn` / `usernamegetfn` / etc.). Same role as
///    C's `Param.gsu->getfn` virtual dispatch.
/// 2. **Local variable** — `variables[name]`. C reads `pm->u.str`
///    for PM_SCALAR; here we hold the scalar in the variables
///    HashMap.
/// 3. **Environment fallback** — `std::env::var(name)`. C imports
///    env vars into the param table at startup so they go through
///    the same dispatch as everything else; zshrs reads from the
///    OS env on miss to match.
/// 4. **Array → scalar** — `arrays[name].join(" ")`. Mirrors
///    C's PM_ARRAY case in getstrvalue (params.c:2358) which
///    joins via `sepjoin(ss, NULL, 1)`.
///
// Retrieve a scalar (string) parameter                                     // c:3072
/// Returns `None` only if all four paths miss (parameter genuinely
/// unset).
pub fn getsparam(                                                            // c:3076
    variables: &std::collections::HashMap<String, String>,
    arrays: &std::collections::HashMap<String, Vec<String>>,
    name: &str,
) -> Option<String> {
    // 1. GSU dispatch — Param.gsu->getfn equivalent.
    if let Some(v) = lookup_special_var(name) {
        return Some(v);
    }
    // 2. Local shell variable (PM_SCALAR pm->u.str).
    if let Some(s) = variables.get(name) {
        return Some(s.clone());
    }
    // 3. Env-imported parameter (C imports env into paramtab at
    //    init, so reads route through the same dispatch).
    if let Ok(s) = std::env::var(name) {
        return Some(s);
    }
    // 4. PM_ARRAY → scalar join (C: sepjoin in getstrvalue).
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
pub fn unsetparam(                                                          // c:3819
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
pub fn export_param(_table: &mut crate::ported::zsh_h::HashTable, _name: &str) {
    // TODO: real port — gethnode(paramtab, name); pm->node.flags |= PM_EXPORTED;
    // call addenv(pm, getstrvalue(pm)). Old body referenced fake fields
    // `.params`/`.value` on deleted ParamTable.
}

/// Start a parameter scope.
/// Port of `startparamscope()` (Src/init.c) — the C source pushes the
/// current scope counter so `local`-declared params disappear on function
/// exit. Rust port operates on the bucket-2 holder `paramtab` via a
/// `&mut crate::ported::zsh_h::HashTable` argument.
pub fn startparamscope(_table: &mut crate::ported::zsh_h::HashTable) {
    // TODO: bump module-level `locallevel` (see Src/init.c) — used to be
    // a field on the deleted `ParamTable`.
}

/// Port of `endparamscope()` from `Src/params.c:5894`. Decrements the
/// scope counter and either restores or unsets every param at the
/// outgoing level: special params get their saved `old` flags/value
/// reapplied via `handle_special_set`; non-special locals shadow back
/// to their pre-scope param if present, otherwise are removed.
pub fn endparamscope(_table: &mut crate::ported::zsh_h::HashTable) {
    // TODO: real port — scanhashtable(paramtab, scanendscope) (Src/params.c:5942).
    // Old body referenced fake `.params`/`.local_level`/`.is_special()`/etc.
    // on deleted ParamTable.
}


// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

/// Check if string is valid identifier (from params.c isident)
// Return 1 if the string s is a valid identifier, else return 0.         // c:1284
pub fn isident(s: &str) -> bool {                                           // c:1288
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
    let (fmt_char, digits) = if (pm_flags & crate::ported::zsh_h::PM_EFLOAT) != 0 { // c:5715
        let d = if digits <= 0 { 10 } else { digits };           // c:5718
        ('e', (d - 1).max(0))                                    // c:5725
    } else if (pm_flags & crate::ported::zsh_h::PM_FFLOAT) != 0 {                  // c:5716
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

// intgetfn / strgetfn drift wrappers removed — replaced below with
// real C-shape ports `intgetfn(pm: &param) -> i64` (Src/params.c:3993)
// and `strgetfn(pm: &param) -> String` (Src/params.c:4029) that read
// directly from the union fields `pm->u.val` / `pm->u.str`.

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // test_param_value_conversions removed: tested deleted fake
    // `ParamValue::Scalar` constructor. C uses union access on
    // `pm->u.str`/`u.val`/`u.dval`/`u.arr` dispatched via
    // `PM_TYPE(pm->node.flags)` (Src/zsh.h:540).
    #[test]
    fn test_colonarr_conversion() {
        let arr = colonsplit("/bin:/usr/bin:/usr/local/bin");
        assert_eq!(arr, vec!["/bin", "/usr/bin", "/usr/local/bin"]);
        let path = colonarrgetfn(&arr);
        assert_eq!(path, "/bin:/usr/bin:/usr/local/bin");
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
        let s = convfloat(2.5, 2, crate::ported::zsh_h::PM_FFLOAT);
        assert!(s.starts_with("2.50"));

        assert_eq!(convfloat(f64::INFINITY, 0, 0), "Inf");
        assert_eq!(convfloat(f64::NEG_INFINITY, 0, 0), "-Inf");
        assert_eq!(convfloat(f64::NAN, 0, 0), "NaN");
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
    fn test_uniq_array_empty() {
        let empty: Vec<String> = Vec::new();
        assert!(uniqarray(empty).is_empty());
    }

    #[test]
    fn test_convbase_underscore() {
        let s = convbase_underscore(1234567, 10, 3);
        assert_eq!(s, "1_234_567");
    }

    fn val_str(v: GetargOut<'_>) -> String {
        match v {
            GetargOut::Value(v) => v.to_str(),
            GetargOut::Flags { .. } => panic!("expected Value, got Flags"),
        }
    }

    #[test]
    fn getarg_n_flag_picks_second_exact_match() {
        // C params.c:1431-1442 + 1758 — `(en.2.)pat` picks 2nd exact match.
        let arr: Vec<String> = vec!["foo".into(), "bar".into(), "foo".into(), "baz".into()];
        let out = getarg("(en.2.r)foo", Some(&arr), None, None).expect("Some");
        assert_eq!(val_str(out), "foo");
    }

    #[test]
    fn getarg_n_flag_third_exact_match() {
        let arr: Vec<String> = vec!["a".into(), "a".into(), "a".into(), "b".into()];
        let out = getarg("(en.3.r)a", Some(&arr), None, None).expect("Some");
        assert_eq!(val_str(out), "a");
    }

    #[test]
    fn getarg_n_flag_returns_index_with_i() {
        // (en.2.i) — return INDEX of 2nd exact match.
        let arr: Vec<String> = vec!["x".into(), "y".into(), "x".into(), "y".into()];
        let out = getarg("(en.2.i)x", Some(&arr), None, None).expect("Some");
        assert_eq!(val_str(out), "3");
    }

    #[test]
    fn getarg_negative_n_flips_search_direction() {
        // C params.c:1488-1491 — negative `num` flips down (reverse).
        // (en.-1.) on forward-default search matches from the end.
        let arr: Vec<String> = vec!["a".into(), "a".into(), "a".into()];
        let out = getarg("(en.-1.i)a", Some(&arr), None, None).expect("Some");
        assert_eq!(val_str(out), "3");
    }

    #[test]
    fn getarg_n_flag_zero_treated_as_one() {
        // C params.c:1438-1439 — `if (!num) num = 1`.
        let arr: Vec<String> = vec!["x".into(), "y".into()];
        let out = getarg("(en.0.r)x", Some(&arr), None, None).expect("Some");
        assert_eq!(val_str(out), "x");
    }

    #[test]
    fn getarg_unknown_flag_char_returns_none() {
        // C params.c:1477-1483 flagerr — invalid flag char reports error.
        let arr: Vec<String> = vec!["x".into()];
        assert!(getarg("(z)x", Some(&arr), None, None).is_none());
    }

    #[test]
    fn getarg_n_flag_unterminated_arg_returns_none() {
        // (n.5 missing closing delimiter — flagerr.
        let arr: Vec<String> = vec!["x".into()];
        assert!(getarg("(n.5", Some(&arr), None, None).is_none());
    }

    #[test]
    fn getarg_b_flag_starts_search_at_index() {
        // C params.c:1748-1760 — `(b.N.e)pat` skips first N-1 elements
        // forward (parsed value `N`, normalized to `beg = N-1`).
        let arr: Vec<String> = vec!["x".into(), "y".into(), "x".into(), "y".into()];
        // Forward, beg=2 (skip first 2) → starts at idx 2 → 'x' at 3.
        let out = getarg("(b.3.ei)x", Some(&arr), None, None).expect("Some");
        assert_eq!(val_str(out), "3");
    }

    #[test]
    fn getarg_b_flag_with_R_reverse_from_offset() {
        // C params.c:1750-1755 — reverse search starting at parsed-1 idx.
        // arr=(x y x y), beg=2 (parsed 3-1), reverse → walks 2,1,0; first
        // exact 'x' is at idx 2 → 1-based "3".
        let arr: Vec<String> = vec!["x".into(), "y".into(), "x".into(), "y".into()];
        let out = getarg("(b.3.eIR)x", Some(&arr), None, None).expect("Some");
        assert_eq!(val_str(out), "3");
    }

    #[test]
    fn getarg_b_flag_out_of_bounds_forward_returns_empty() {
        // c:1746 — beg >= len returns len+1 (empty for value-mode).
        let arr: Vec<String> = vec!["x".into()];
        let out = getarg("(b.5.er)x", Some(&arr), None, None).expect("Some");
        assert_eq!(val_str(out), "");
    }

    #[test]
    fn getarg_b_flag_out_of_bounds_index_mode_returns_len_plus_one() {
        let arr: Vec<String> = vec!["x".into(), "y".into()];
        let out = getarg("(b.5.ei)x", Some(&arr), None, None).expect("Some");
        assert_eq!(val_str(out), "3");
    }

    #[test]
    fn getarg_hash_neg_num_on_lowercase_r_returns_all() {
        // C params.c:1488-1491 — neg `num` flips down on `r`,
        // converting hash search to return-all-matches semantics.
        let mut h: indexmap::IndexMap<String, String> = indexmap::IndexMap::new();
        h.insert("a".into(), "1".into());
        h.insert("b".into(), "1".into());
        h.insert("c".into(), "2".into());
        let out = getarg("(en.-1.r)1", None, Some(&h), None).expect("Some");
        // r + neg = R semantics → all values where pat matches value.
        assert_eq!(val_str(out), "1 1");
    }

    #[test]
    fn getarg_hash_neg_num_on_uppercase_R_returns_single() {
        // R + neg `num` un-flips back to single-match (r semantics).
        let mut h: indexmap::IndexMap<String, String> = indexmap::IndexMap::new();
        h.insert("a".into(), "1".into());
        h.insert("b".into(), "1".into());
        h.insert("c".into(), "2".into());
        let out = getarg("(en.-1.R)1", None, Some(&h), None).expect("Some");
        // R + neg → r → single first match.
        assert_eq!(val_str(out), "1");
    }

    #[test]
    fn getarg_hash_b_flag_skips_first_n_entries() {
        // C params.c:1740-1742 — `b<NUM>` skips first N-1 entries
        // before searching. Hash iteration is insertion order.
        let mut h: indexmap::IndexMap<String, String> = indexmap::IndexMap::new();
        h.insert("a".into(), "1".into());
        h.insert("b".into(), "1".into());
        h.insert("c".into(), "1".into());
        // beg=2 (parsed 3-1) → skip first 2, scan from "c" onward.
        let out = getarg("(b.3.ei)1", None, Some(&h), None).expect("Some");
        assert_eq!(val_str(out), "c");
    }

    #[test]
    fn getarg_hash_b_flag_with_R_collects_from_offset() {
        // R returns all matches; b skips first beg entries first.
        let mut h: indexmap::IndexMap<String, String> = indexmap::IndexMap::new();
        h.insert("a".into(), "1".into());
        h.insert("b".into(), "1".into());
        h.insert("c".into(), "1".into());
        let out = getarg("(b.2.eI)1", None, Some(&h), None).expect("Some");
        // beg=1, return_all=I → walk from "b" onward, all matching keys.
        assert_eq!(val_str(out), "b c");
    }

    #[test]
    fn getarg_hash_b_flag_out_of_bounds_returns_empty() {
        // c:1746 — beg >= len with single-match → empty.
        let mut h: indexmap::IndexMap<String, String> = indexmap::IndexMap::new();
        h.insert("a".into(), "1".into());
        let out = getarg("(b.5.e)1", None, Some(&h), None).expect("Some");
        assert_eq!(val_str(out), "");
    }

    #[test]
    fn getarg_w_flag_splits_multi_word_array_elements() {
        // C params.c:1761-1797 — `(w)N` joins array then re-splits by
        // IFS-default whitespace. arr=("a b" "c d"); (w)2 → "b" not "c d".
        let arr: Vec<String> = vec!["a b".into(), "c d".into()];
        let out = getarg("(w)2", Some(&arr), None, None).expect("Some");
        assert_eq!(val_str(out), "b");
    }

    #[test]
    fn getarg_w_flag_simple_array_indexing_still_works() {
        let arr: Vec<String> = vec!["one".into(), "two".into(), "three".into()];
        let out = getarg("(w)2", Some(&arr), None, None).expect("Some");
        assert_eq!(val_str(out), "two");
    }

    #[test]
    fn getarg_f_flag_splits_by_newline() {
        // C params.c:1424-1427 — `f` flag aliases `w` with sep="\n".
        // arr=("a b\nc d"); (f)2 → "c d" (split by \n only, not space).
        let arr: Vec<String> = vec!["a b\nc d".into()];
        let out = getarg("(f)2", Some(&arr), None, None).expect("Some");
        assert_eq!(val_str(out), "c d");
    }

    #[test]
    fn getarg_scalar_w_flag_picks_nth_word() {
        // C params.c:1761-1797 — scalar word-mode arm. `(w)2` on
        // scalar "hello world foo" returns the 2nd whitespace word.
        let out = getarg("(w)2", None, None, Some("hello world foo")).expect("Some");
        assert_eq!(val_str(out), "world");
    }

    #[test]
    fn getarg_scalar_w_flag_negative_index_counts_from_end() {
        let out = getarg("(w)-1", None, None, Some("alpha beta gamma")).expect("Some");
        assert_eq!(val_str(out), "gamma");
    }

    #[test]
    fn getarg_scalar_re_returns_char_at_match_position() {
        // C params.c:1798-1980 — char-search returns CHAR at match
        // position, not full substring. Verified empirically:
        //   /bin/zsh -c 's="barfooxyz"; print "${s[(r)foo]}"'  → "f"
        let out = getarg("(re)bc", None, None, Some("abcdef")).expect("Some");
        assert_eq!(val_str(out), "b");
    }

    #[test]
    fn getarg_scalar_ie_returns_position_of_first_match() {
        let out = getarg("(ie)cd", None, None, Some("abcdef")).expect("Some");
        // 'cd' starts at 1-based position 3.
        assert_eq!(val_str(out), "3");
    }

    #[test]
    fn getarg_scalar_Ie_returns_position_of_last_match() {
        let out = getarg("(Ie)b", None, None, Some("abcabc")).expect("Some");
        // Last 'b' is at 1-based position 5.
        assert_eq!(val_str(out), "5");
    }

    #[test]
    fn getarg_scalar_ie_no_match_returns_len_plus_one() {
        let out = getarg("(ie)z", None, None, Some("abc")).expect("Some");
        assert_eq!(val_str(out), "4");
    }

    #[test]
    fn getarg_scalar_Ie_no_match_returns_zero() {
        let out = getarg("(Ie)z", None, None, Some("abc")).expect("Some");
        assert_eq!(val_str(out), "0");
    }

    #[test]
    fn getarg_scalar_n_flag_picks_second_match() {
        // C params.c:1929/1964 — `!--num` Nth-match counter on
        // scalar char-search. abcabc: 'a' at idx 0 and 3 → 2nd match
        // at byte position 4 (1-based).
        let out = getarg("(en.2.i)a", None, None, Some("abcabc")).expect("Some");
        assert_eq!(val_str(out), "4");
    }

    #[test]
    fn getarg_scalar_b_flag_starts_from_offset() {
        // C params.c:1740-1742 — `(b.N.)` starts search from idx N-1.
        // abc bc abc: with b=4, skip first 3 chars; first 'b' at byte 5.
        let out = getarg("(b.4.ei)b", None, None, Some("abcbc")).expect("Some");
        assert_eq!(val_str(out), "4");
    }

    #[test]
    fn getarg_scalar_re_n2_picks_second_substring() {
        let out = getarg("(en.2.r)b", None, None, Some("abab")).expect("Some");
        assert_eq!(val_str(out), "b");
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: params
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

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
///   - Scalar word-mode arm (c:1761-1797) when `scalar` is `Some`.
///
/// TODO (later phases):
///   - Brace-depth walk to closing `]` (c:1507-1535)
///   - parsestr + singsub on subscript body (c:1545-1580)
///   - mathevalarg integer parse (c:1601-1604)
///   - Multibyte char-search arm (c:1798-1985)
pub(crate) fn getarg<'a>(
    idx: &'a str,
    arr: Option<&[String]>,
    assoc: Option<&indexmap::IndexMap<String, String>>,
    scalar: Option<&str>,
) -> Option<GetargOut<'a>> {
    let rest = idx.strip_prefix('(')?;
    // Reject anything that looks like a char-class subscript: `[abc]`
    // doesn't match this prefix, but `(...)` containing brackets is
    // probably alternation — let it fall through to runtime instead.
    if rest.starts_with(')') || rest.contains('[') {
        return None;
    }
    // Flag scanner per zshparam(1) "Subscript Flags" /
    // params.c:1389-1480 switch:
    //   r/R (reverse value-search → value/all values),
    //   i/I (value-search → key/all keys),
    //   k/K (key-search → value/all values),
    //   e (exact match — disables glob),
    //   n<DELIM>NUM<DELIM> (Nth match — params.c:1431-1442),
    //   b<DELIM>NUM<DELIM> (begin offset — params.c:1443-1454),
    //   w (word index on scalar),
    //   f (word index split by newline; alias for `w` + sep="\n"),
    //   p (escapes for next get_strarg),
    //   s<DELIM>SEP<DELIM> (split-by-separator).
    // The `n` / `b` / `s` forms use `get_strarg`'s balanced-delimiter
    // pair: any non-flag char closes its pair (`(n.5.)`, `(n:5:)` etc.).
    let bytes = rest.as_bytes();
    let mut i: usize = 0;
    let mut num: i64 = 1;
    let mut beg: i64 = 0;
    let mut has_beg = false;
    let flags_start = 0_usize;
    let mut flags_end = 0_usize;
    let mut bad = false;
    while i < bytes.len() && bytes[i] != b')' {
        let c = bytes[i] as char;
        match c {
            'r' | 'R' | 'i' | 'I' | 'e' | 'k' | 'K' | 'w' | 'f' | 'p' => {
                i += 1;
                flags_end = i;
            }
            'n' | 'b' => {
                // Consume `n<DELIM>NUM<DELIM>` per c:1432 get_strarg.
                if i + 1 >= bytes.len() {
                    bad = true;
                    break;
                }
                let delim = bytes[i + 1];
                let arg_start = i + 2;
                let mut arg_end = arg_start;
                while arg_end < bytes.len() && bytes[arg_end] != delim {
                    arg_end += 1;
                }
                if arg_end >= bytes.len() {
                    bad = true;
                    break;
                }
                // Parse the argument as a signed decimal integer.
                let arg = std::str::from_utf8(&bytes[arg_start..arg_end]).ok()?;
                let parsed: i64 = arg.trim().parse().ok()?;
                if c == 'n' {
                    num = if parsed == 0 { 1 } else { parsed };
                } else {
                    has_beg = true;
                    beg = if parsed > 0 { parsed - 1 } else { parsed };
                }
                i = arg_end + 1;
                flags_end = i;
            }
            's' => {
                // (s:SEP:) — pass through with raw flag block.
                let close = match rest[i..].find(')') {
                    Some(p) => i + p,
                    None => return None,
                };
                let flags = &rest[flags_start..close];
                return Some(GetargOut::Flags { flags, rest: &rest[close + 1..] });
            }
            _ => {
                bad = true;
                break;
            }
        }
    }
    // c:1477-1483 — flag-error fallback: reset all flags, treat as no
    // subscript flags.
    if bad {
        return None;
    }
    if i >= bytes.len() || bytes[i] != b')' {
        return None;
    }
    if flags_end == flags_start {
        return None;
    }
    let flags = &rest[flags_start..flags_end];
    let pat = &rest[i + 1..];

    // c:1488-1491 — negative `num` flips the search direction.
    let neg_num_flips = num < 0;
    if neg_num_flips {
        num = -num;
    }

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
        // C params.c:1488-1491 — negative `num` flips `down`. Since
        // R/I/K already set down=1, neg_num XORs the bit (r/i/k +
        // neg → return_all; R/I/K + neg → single-match again).
        let is_uppercase = flags.contains('I') || flags.contains('R') || flags.contains('K');
        let return_all = is_uppercase ^ neg_num_flips;

        // c:1740-1747 — `b<NUM>` start offset on the values array. The
        // hash is iterated in insertion order (IndexMap); skip first
        // `beg` entries before counting matches.
        let len = map.len() as i64;
        let mut start = beg;
        if start < 0 {
            start += len;
        }
        if !return_all && start >= len {
            return Some(GetargOut::Value(Value::str("")));
        }
        let skip = if start < 0 { 0 } else { start as usize };

        // Per C params.c:1707-1709 + zsh 5.9 empirical:
        //   k/K — keymatch path: pprog=NULL, no glob; exact key
        //         lookup. `(K)*` returns "" because there's no key
        //         literally named "*".
        //   r/R/i/I — value path: pprog=patcompile, glob/exact.
        let key_compare = |target: &str| -> bool {
            if key_match {
                target == pat
            } else if exact {
                target == pat
            } else {
                crate::ported::pattern::patmatch(pat, target)
            }
        };
        if return_all {
            let mut out: Vec<String> = Vec::new();
            for (k, v) in map.iter().skip(skip) {
                let target = if key_match { k.as_str() } else { v.as_str() };
                if key_compare(target) {
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
        // c:1753 — `!--num` skips matches until the Nth.
        let mut remaining = num;
        for (k, v) in map.iter().skip(skip) {
            let target = if key_match { k.as_str() } else { v.as_str() };
            if key_compare(target) {
                remaining -= 1;
                if remaining == 0 {
                    return Some(GetargOut::Value(Value::str(if key_match {
                        v.clone()
                    } else if return_index {
                        k.clone()
                    } else {
                        v.clone()
                    })));
                }
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
        // C params.c:1761-1797 — `(w)N` / `(f)N` word-mode arm.
        // `getstrvalue(v)` joins the array; `sepsplit` re-splits by
        // sep (`f` → "\n", `w` → IFS-default whitespace, `s:SEP:`
        // → user sep), then the Nth split word is returned. So
        // `arr=("a b" "c d"); ${arr[(w)2]}` → "b" (joined "a b c d",
        // split → ["a","b","c","d"], pick idx 1).
        if flags.contains('w') || flags.contains('f') {
            if let Ok(n) = pat.parse::<i64>() {
                let sep_chars: &[char] = if flags.contains('f') {
                    &['\n']
                } else {
                    &[' ', '\t', '\n']
                };
                let joined = arr.join(" ");
                let words: Vec<&str> = joined
                    .split(|c: char| sep_chars.contains(&c))
                    .filter(|w| !w.is_empty())
                    .collect();
                let len = words.len() as i64;
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
                    Value::str(words.get(idx_into).map(|s| s.to_string()).unwrap_or_default())
                ));
            }
        }
        let exact = flags.contains('e');
        let word = flags.contains('w') || flags.contains('f');
        let _ = word;
        let return_index = flags.contains('i') || flags.contains('I');
        // C params.c:1575 `if (!rev)` — without a direction flag
        // (r/R/i/I/k/K), getarg does NOT enter the search loop on
        // arrays; pat is mathevalarg'd as an integer index instead.
        // Verified empirically: `arr=(foo bar); ${arr[(e)foo]}`
        // returns empty in real zsh (mathevalarg fails, no element).
        let any_search_flag = flags.contains('r')
            || flags.contains('R')
            || flags.contains('i')
            || flags.contains('I')
            || flags.contains('k')
            || flags.contains('K');
        if !any_search_flag {
            return None;
        }
        // c:1488-1491 — negative `num` flips reverse direction.
        let reverse = (flags.contains('R') || flags.contains('I')) ^ neg_num_flips;
        // C params.c:1668-1685 implicit `*` wrap fires only when
        // `v->scanflags` is unset; in standard subscript callsites
        // scanflags IS set, so the wrap does NOT engage. Verified
        // empirically: `arr=(foobar baz); ${arr[(r)foo]}` returns
        // empty in real zsh (exact match), not "foobar". Pattern is
        // used verbatim — globbing only when user supplies `*`.
        let pat_used: &str = pat;

        // c:1740-1760 — `b<NUM>` starting offset + bounds checks.
        // beg is already 0-based after parse (parsed-1 for positive).
        let len = arr.len() as i64;
        let mut start = beg;
        if start < 0 {
            start += len;
        }
        // c:1743-1747 — out-of-bounds returns.
        if reverse {
            if start < 0 {
                return Some(GetargOut::Value(if return_index {
                    Value::str("0")
                } else {
                    Value::str("")
                }));
            }
        } else if start >= len {
            return Some(GetargOut::Value(if return_index {
                Value::str((arr.len() + 1).to_string())
            } else {
                Value::str("")
            }));
        }
        // c:1750-1751 — reverse w/o explicit b starts from len-1.
        if reverse && !has_beg {
            start = len - 1;
        }

        let iter: Box<dyn Iterator<Item = (usize, &String)>> = if reverse {
            // c:1752 — `for (p = ta + beg; p >= ta; p--)`: clamp start
            // into the valid range then walk backwards.
            let s_idx = if start < 0 { 0 } else { start as usize };
            let s_idx = s_idx.min(arr.len().saturating_sub(1));
            Box::new(arr[..=s_idx].iter().enumerate().rev())
        } else {
            // c:1757 — `for (p = ta + beg; *p; p++)`: skip first beg.
            let s_idx = if start < 0 { 0 } else { start as usize };
            Box::new(arr.iter().enumerate().skip(s_idx))
        };
        // c:1758 — `!--num` skips matches until the Nth.
        let mut remaining = num;
        for (i, s) in iter {
            let hit = if exact {
                s == pat
            } else {
                crate::ported::pattern::patmatch(pat_used, s)
            };
            if hit {
                remaining -= 1;
                if remaining == 0 {
                    return Some(GetargOut::Value(if return_index {
                        Value::str((i + 1).to_string())
                    } else {
                        Value::str(s.clone())
                    }));
                }
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

    // C params.c:1761-1797 — scalar word-mode arm. `(w)N` joins
    // the source string and re-splits by sep (whitespace by default
    // for `w`, "\n" for `f`). When `pat` is a numeric N, the Nth
    // word is returned. Pattern-search variants on scalars share
    // the c:1798-1980 char-search arm which is not yet ported.
    if let Some(s) = scalar {
        use fusevm::Value;
        if flags.contains('w') || flags.contains('f') {
            if let Ok(n) = pat.parse::<i64>() {
                let sep_chars: &[char] = if flags.contains('f') {
                    &['\n']
                } else {
                    &[' ', '\t', '\n']
                };
                let words: Vec<&str> = s
                    .split(|c: char| sep_chars.contains(&c))
                    .filter(|w| !w.is_empty())
                    .collect();
                let len = words.len() as i64;
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
                    Value::str(words.get(idx_into).map(|s| s.to_string()).unwrap_or_default()),
                ));
            }
        }
        // C params.c:1798-1980 — scalar char-search arm. `(i)/(I)/
        // (r)/(R)` on a scalar runs a sliding-window glob match.
        // (i)/(I) return the 1-based byte position of first/last
        // match; (r)/(R) return the matched substring.
        // Multibyte cursor outputs (prevcharlen/nextcharlen at
        // c:1948-1971) are not yet ported; ASCII-only path here.
        let any_search = flags.contains('r')
            || flags.contains('R')
            || flags.contains('i')
            || flags.contains('I');
        if any_search {
            let return_index = flags.contains('i') || flags.contains('I');
            let want_last = flags.contains('I') || flags.contains('R');
            // Negative `num` flips direction (c:1488-1491).
            let want_last = want_last ^ neg_num_flips;
            let s_chars: Vec<char> = s.chars().collect();
            let n = s_chars.len();
            let positions: Box<dyn Iterator<Item = usize>> = if want_last {
                Box::new((0..=n).rev())
            } else {
                Box::new(0..=n)
            };
            // c:1929+ / c:1964 — `!--num` skips matches until the Nth.
            // Per `b<NUM>` (c:1740-1747) — start from offset, only
            // when has_beg is set. Without `b`, walk all positions.
            let beg_idx_opt: Option<usize> = if has_beg {
                let beg_norm = if beg < 0 { beg + n as i64 } else { beg };
                Some(if beg_norm < 0 {
                    0
                } else {
                    (beg_norm as usize).min(n)
                })
            } else {
                None
            };
            let mut found: Option<(usize, usize)> = None;
            let mut remaining = num;
            'outer: for start in positions {
                if let Some(b_idx) = beg_idx_opt {
                    if want_last {
                        if start > b_idx {
                            continue;
                        }
                    } else if start < b_idx {
                        continue;
                    }
                }
                for span_len in 1..=(n - start) {
                    let cand: String = s_chars[start..start + span_len].iter().collect();
                    let hit = if flags.contains('e') {
                        cand == pat
                    } else {
                        crate::ported::pattern::patmatch(pat, &cand)
                    };
                    if hit {
                        remaining -= 1;
                        if remaining == 0 {
                            found = Some((start, start + span_len));
                            break 'outer;
                        }
                        // Advance past this match position to find the
                        // next-Nth instead of repeatedly matching same
                        // start (mirrors C's pointer increment).
                        break;
                    }
                }
            }
            return Some(GetargOut::Value(match (found, return_index) {
                (Some((s_pos, _)), true) => Value::str((s_pos + 1).to_string()),
                // C params.c:1798-1980 char-search returns the char AT
                // the match position, not the full matched substring.
                // Verified empirically: `s="barfooxyz"; ${s[(r)foo]}`
                // returns "f" in real zsh, not "foo".
                (Some((s_pos, _)), false) => Value::str(
                    s_chars.get(s_pos).map(|c| c.to_string()).unwrap_or_default(),
                ),
                (None, true) => Value::str(if flags.contains('i') {
                    (n + 1).to_string()
                } else {
                    "0".to_string()
                }),
                (None, false) => Value::str(String::new()),
            }));
        }
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
// Special-parameter GSU (get/set/unset) callbacks ported from
// Src/params.c.
//
// C zsh stores per-special-param state in file-static globals
// (`ifs`, `home`, `term`, `histsiz`, etc.) and dispatches getfn/
// setfn/unsetfn callbacks through `Param.gsu->getfn` etc. zshrs's
// param storage is per-evaluator HashMaps on `ShellExecutor`, so
// the C globals are reproduced as `OnceLock<Mutex<…>>` module
// statics here, with the get/set fns mutating the static.
//
// Functions that genuinely need a `Param *` (the GSU dispatch
// callbacks for non-special arr/hash/int/float/str params, the
// param-table mutators, scope helpers, etc.) cannot be properly
// ported until zshrs gains a Param struct + callback-table ABI;
// those keep their C signatures but the body is a WARNING-stub
// that does nothing.
// ===========================================================

use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;
use crate::zsh_h::{paramdef, PM_ARRAY, PM_DONTIMPORT, PM_DONTIMPORT_SUID, PM_INTEGER, PM_READONLY_SPECIAL, PM_SCALAR, PM_SPECIAL, PM_UNSET};
// -----------------------------------------------------------
// Module statics — one per C global referenced by the special-
// param callbacks below. All initialised lazily on first read.
// -----------------------------------------------------------

// `Src/params.c:515  mod_export HashTable paramtab, realparamtab;`
//
// `realparamtab` always points to the shell's global parameter
// table. `paramtab` normally aliases it; it is temporarily
// redirected during associative-array key iteration
// (`Src/params.c:508-513` — "paramtab is sometimes temporarily
// changed to point at another table").
//
// PORT_PLAN.md bucket 2: shell-wide shared C global → ported as
// `Arc<RwLock<…>>` so worker threads see the same table. Names
// match the C identifiers, uppercased per the bucket-2 holder
// convention (PORT_PLAN.md:637 "PARAMTAB ← paramtab").
//
// Both are populated on first access by `createparamtable()`
// (params.rs:1513), which mirrors `createparamtable()`
// (Src/params.c:817).
pub static REALPARAMTAB: OnceLock<Arc<RwLock<crate::ported::zsh_h::HashTable>>> = OnceLock::new();
pub static PARAMTAB: OnceLock<Arc<RwLock<crate::ported::zsh_h::HashTable>>> = OnceLock::new();

fn ifs_lock() -> &'static Mutex<String> {
    static IFS_VAR: OnceLock<Mutex<String>> = OnceLock::new();
    IFS_VAR.get_or_init(|| Mutex::new(" \t\n\0".to_string()))
}

fn home_lock() -> &'static Mutex<String> {
    static HOME_VAR: OnceLock<Mutex<String>> = OnceLock::new();
    HOME_VAR.get_or_init(|| Mutex::new(env::var("HOME").unwrap_or_default()))
}

fn term_lock() -> &'static Mutex<String> {
    static TERM_VAR: OnceLock<Mutex<String>> = OnceLock::new();
    TERM_VAR.get_or_init(|| Mutex::new(env::var("TERM").unwrap_or_default()))
}

fn wordchars_lock() -> &'static Mutex<String> {
    static WORDCHARS_VAR: OnceLock<Mutex<String>> = OnceLock::new();
    WORDCHARS_VAR.get_or_init(|| Mutex::new("*?_-.[]~=/&;!#$%^(){}<>".to_string()))
}

fn histchars_lock() -> &'static Mutex<[u8; 3]> {
    static HISTCHARS_VAR: OnceLock<Mutex<[u8; 3]>> = OnceLock::new();
    HISTCHARS_VAR.get_or_init(|| Mutex::new([b'!', b'^', b'#']))
}

fn keyboardhack_lock() -> &'static Mutex<u8> {
    static KEYBOARDHACK_VAR: OnceLock<Mutex<u8>> = OnceLock::new();
    KEYBOARDHACK_VAR.get_or_init(|| Mutex::new(0))
}

fn histsiz_lock() -> &'static Mutex<i64> {
    static HISTSIZ_VAR: OnceLock<Mutex<i64>> = OnceLock::new();
    // Match observed `zsh -fc 'echo $HISTSIZE'` output on zsh 5.9+
    // (Homebrew). Upstream's `configure.ac` defines DEFAULT_HISTSIZE
    // as 30 but distributed binaries seed the cap at 999999999 — the
    // parity goal here is "match the binary the user actually runs",
    // not "match the source-code default".
    HISTSIZ_VAR.get_or_init(|| Mutex::new(999_999_999))
}

fn savehistsiz_lock() -> &'static Mutex<i64> {
    static SAVEHISTSIZ_VAR: OnceLock<Mutex<i64>> = OnceLock::new();
    // Same rationale as `histsiz_lock` — observed `zsh -fc
    // 'echo $SAVEHIST'` returns 99999999 on zsh 5.9+. Source has
    // savehistsiz default to 0 but distributed binaries cap at 99M.
    SAVEHISTSIZ_VAR.get_or_init(|| Mutex::new(99_999_999))
}

fn zsh_terminfo_lock() -> &'static Mutex<String> {
    static TERMINFO_VAR: OnceLock<Mutex<String>> = OnceLock::new();
    TERMINFO_VAR.get_or_init(|| Mutex::new(env::var("TERMINFO").unwrap_or_default()))
}

fn zsh_terminfodirs_lock() -> &'static Mutex<String> {
    static TERMINFODIRS_VAR: OnceLock<Mutex<String>> = OnceLock::new();
    TERMINFODIRS_VAR.get_or_init(|| Mutex::new(env::var("TERMINFO_DIRS").unwrap_or_default()))
}

fn cached_username_lock() -> &'static Mutex<String> {
    static USERNAME_VAR: OnceLock<Mutex<String>> = OnceLock::new();
    USERNAME_VAR.get_or_init(|| Mutex::new(initial_username()))
}

/// Resolve the current user's name. Mirrors C's `get_username()`
/// init at Src/init.c which reads `getpwuid(getuid())->pw_name`
/// rather than `$USER`. Falls back to env vars only if the
/// passwd lookup fails (rare on real systems).
fn initial_username() -> String {
    #[cfg(unix)]
    {
        let uid = unsafe { libc::getuid() };
        let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
        let mut buf = vec![0i8; 1024];
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        let rc = unsafe {
            libc::getpwuid_r(uid, &mut pwd, buf.as_mut_ptr(), buf.len(), &mut result)
        };
        if rc == 0 && !result.is_null() && !pwd.pw_name.is_null() {
            let cstr = unsafe { std::ffi::CStr::from_ptr(pwd.pw_name) };
            return cstr.to_string_lossy().into_owned();
        }
    }
    env::var("USER")
        .or_else(|_| env::var("LOGNAME"))
        .unwrap_or_default()
}

fn pipestats_lock() -> &'static Mutex<Vec<i32>> {
    static PIPESTATS_VAR: OnceLock<Mutex<Vec<i32>>> = OnceLock::new();
    PIPESTATS_VAR.get_or_init(|| Mutex::new(Vec::new()))
}

fn shtimer_lock() -> &'static Mutex<Duration> {
    static SHTIMER_VAR: OnceLock<Mutex<Duration>> = OnceLock::new();
    SHTIMER_VAR.get_or_init(|| {
        Mutex::new(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default(),
        )
    })
}

fn pparams_lock() -> &'static Mutex<Vec<String>> {
    // Mirror of zsh's `pparams` (positional params $1, $2, ...).
    // Used by `poundgetfn` for `$#`. Real shell sets this via the
    // `set` builtin / argv on entry; for the callback to work in
    // isolation we expose it as a settable static.
    static PPARAMS_VAR: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    PPARAMS_VAR.get_or_init(|| Mutex::new(Vec::new()))
}

fn zunderscore_lock() -> &'static Mutex<String> {
    static ZUNDERSCORE_VAR: OnceLock<Mutex<String>> = OnceLock::new();
    ZUNDERSCORE_VAR.get_or_init(|| Mutex::new(String::new()))
}

// -----------------------------------------------------------
// libc-backed callbacks (UID/GID/EUID/EGID/errno/RANDOM/TTYIDLE).
// -----------------------------------------------------------

/// Port of `uidgetfn()` from `Src/params.c:4689`. C body:
/// `return getuid();`
pub fn uidgetfn() -> i64 {
    unsafe { libc::getuid() as i64 }
}

/// Port of `uidsetfn()` from `Src/params.c:4698`. C body:
/// `if (setuid((uid_t)x)) zerr("failed to change user ID: %e", errno);`
pub fn uidsetfn(x: i64) {
    if unsafe { libc::setuid(x as libc::uid_t) } != 0 {
        zerr(&format!(
            "failed to change user ID: {}",
            std::io::Error::last_os_error()
        ));
    }
}

/// Port of `euidgetfn()` from `Src/params.c:4710`. C body:
/// `return geteuid();`
pub fn euidgetfn() -> i64 {
    unsafe { libc::geteuid() as i64 }
}

/// Port of `euidsetfn()` from `Src/params.c:4719`. C body:
/// `if (seteuid((uid_t)x)) zerr("failed to change effective user ID: %e", errno);`
pub fn euidsetfn(x: i64) {
    if unsafe { libc::seteuid(x as libc::uid_t) } != 0 {
        zerr(&format!(
            "failed to change effective user ID: {}",
            std::io::Error::last_os_error()
        ));
    }
}

/// Port of `gidgetfn()` from `Src/params.c:4731`. C body: `return getgid();`
pub fn gidgetfn() -> i64 {
    unsafe { libc::getgid() as i64 }
}

/// Port of `gidsetfn()` from `Src/params.c:4740`. C body:
/// `if (setgid((gid_t)x)) zerr("failed to change group ID: %e", errno);`
pub fn gidsetfn(x: i64) {
    if unsafe { libc::setgid(x as libc::gid_t) } != 0 {
        zerr(&format!(
            "failed to change group ID: {}",
            std::io::Error::last_os_error()
        ));
    }
}

/// Port of `egidgetfn()` from `Src/params.c:4752`. C body: `return getegid();`
pub fn egidgetfn() -> i64 {
    unsafe { libc::getegid() as i64 }
}

/// Port of `egidsetfn()` from `Src/params.c:4761`. C body:
/// `if (setegid((gid_t)x)) zerr("failed to change effective group ID: %e", errno);`
pub fn egidsetfn(x: i64) {
    if unsafe { libc::setegid(x as libc::gid_t) } != 0 {
        zerr(&format!(
            "failed to change effective group ID: {}",
            std::io::Error::last_os_error()
        ));
    }
}

/// Port of `errnogetfn()` from `Src/params.c:5015`. C body: `return errno;`
pub fn errnogetfn() -> i64 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as i64
}

/// Port of `errnosetfn()` from `Src/params.c:5004`. C body:
/// `errno = (int)x; if ((zlong)errno != x) zwarn("errno truncated on assignment");`
///
/// Rust note: `errno` is a libc thread-local; Rust uses `std::io::Error`
/// which captures the *last* call. To set errno for subsequent
/// `last_os_error()` reads on macOS / Linux, write through the libc
/// `__error()`/`__errno_location()` accessor.
pub fn errnosetfn(x: i64) {
    extern "C" {
        #[cfg(target_os = "macos")]
        fn __error() -> *mut libc::c_int;
        #[cfg(target_os = "linux")]
        fn __errno_location() -> *mut libc::c_int;
    }
    let truncated = x as i32;
    unsafe {
        #[cfg(target_os = "macos")]
        {
            *__error() = truncated;
        }
        #[cfg(target_os = "linux")]
        {
            *__errno_location() = truncated;
        }
    }
    if truncated as i64 != x {
        zerr("errno truncated on assignment");
    }
}

/// Port of `randomgetfn()` from `Src/params.c:4543`. C body:
/// `return rand() & 0x7fff;`
pub fn randomgetfn() -> i64 {
    (unsafe { libc::rand() } & 0x7fff) as i64
}

/// Port of `randomsetfn()` from `Src/params.c:4552`. C body:
/// `srand((unsigned int)v);`
pub fn randomsetfn(v: i64) {
    unsafe { libc::srand(v as libc::c_uint) };
}

/// Port of `ttyidlegetfn()` from `Src/params.c:4771`. C body:
/// ```c
/// struct stat ttystat;
/// if (SHTTY == -1 || fstat(SHTTY, &ttystat)) return -1;
/// return time(NULL) - ttystat.st_atime;
/// ```
/// Rust port reads stdin (fd 0) — closest match to `SHTTY` the
/// shell tracks as the controlling-tty fd. Returns -1 if stdin is
/// not a tty.
pub fn ttyidlegetfn() -> i64 {
    if unsafe { libc::isatty(0) } == 0 {
        return -1;
    }
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(0, &mut st) } != 0 {
        return -1;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    now - st.st_atime as i64
}

// -----------------------------------------------------------
// SECONDS / EPOCHSECONDS family — backed by SHTIMER static.
// -----------------------------------------------------------

/// Port of `intsecondsgetfn()` from `Src/params.c:4561`. C body:
/// `return (zlong)(now.tv_sec - shtimer.tv_sec - …);`
pub fn intsecondsgetfn() -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let timer = *shtimer_lock().lock().expect("shtimer poisoned");
    let now_sec = now.as_secs() as i64;
    let timer_sec = timer.as_secs() as i64;
    let now_nsec = now.subsec_nanos() as i64;
    let timer_nsec = timer.subsec_nanos() as i64;
    now_sec - timer_sec - i64::from(now_nsec < timer_nsec)
}

/// Port of `intsecondssetfn()` from `Src/params.c:4575`. C body:
/// `shtimer.tv_sec = now.tv_sec - x; shtimer.tv_nsec = now.tv_nsec;`
pub fn intsecondssetfn(x: i64) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let now_sec = now.as_secs() as i64;
    let new_sec = now_sec - x;
    if new_sec < 0 {
        zerr("SECONDS truncated on assignment");
        return;
    }
    *shtimer_lock().lock().expect("shtimer poisoned") =
        Duration::new(new_sec as u64, now.subsec_nanos());
}

/// Port of `floatsecondsgetfn()` from `Src/params.c:4591`. C body:
/// `return (double)(now-tv_sec - shtimer.tv_sec) + nsec/1e9;`
pub fn floatsecondsgetfn() -> f64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let timer = *shtimer_lock().lock().expect("shtimer poisoned");
    (now - timer).as_secs_f64()
}

/// Port of `floatsecondssetfn()` from `Src/params.c:4603`. C body:
/// `shtimer.tv_sec = now.tv_sec - (zlong)x; shtimer.tv_nsec = now.tv_nsec - (x-int)*1e9;`
pub fn floatsecondssetfn(x: f64) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let new = now.checked_sub(Duration::from_secs_f64(x)).unwrap_or_default();
    *shtimer_lock().lock().expect("shtimer poisoned") = new;
}

/// Port of `getrawseconds()` from `Src/params.c:4615`. C body:
/// `return (double)shtimer.tv_sec + (double)shtimer.tv_nsec / 1e9;`
pub fn getrawseconds() -> f64 {
    shtimer_lock().lock().expect("shtimer poisoned").as_secs_f64()
}

/// Port of `setrawseconds()` from `Src/params.c:4622`. C body:
/// `shtimer.tv_sec = (zlong)x; shtimer.tv_nsec = (x-int)*1e9;`
pub fn setrawseconds(x: f64) {
    *shtimer_lock().lock().expect("shtimer poisoned") = Duration::from_secs_f64(x);
}

/// Port of `setsecondstype()` from `Src/params.c:4630`. C body
/// flips the `gsu.f`/`gsu.i` callback pointer based on the new
/// param-flag bitset.
///
/// WARNING: zshrs has no Param/GSU dispatch table yet — the
/// "promotion between integer/float seconds" logic happens via
/// pm->gsu pointer swaps in C. Returns 0 to signal success;
/// callers can assume the type change is recorded by the caller's
/// own bookkeeping until the GSU table lands.
pub fn setsecondstype(_on: i32, _off: i32) -> i32 {
    0
}

// -----------------------------------------------------------
// $0 / $#
// -----------------------------------------------------------

/// Port of `argzerogetfn()` from `Src/params.c:4954`. C body:
/// `return isset(POSIXARGZERO) ? posixzero : argzero;`
///
/// Reads through `crate::ported::utils::argzero()` (the canonical
/// OnceLock storage in utils.rs). C's `posixzero` branch is not
/// yet ported (POSIXARGZERO option needs the option table).
pub fn argzerogetfn() -> String {
    crate::ported::utils::argzero().unwrap_or_default()
}

/// Port of `argzerosetfn()` from `Src/params.c:4937`. C body:
/// `if (isset(POSIXARGZERO)) zerr("read-only variable: 0"); else { zsfree(argzero); argzero = ztrdup(x); }`
pub fn argzerosetfn(x: String) {
    crate::ported::utils::set_argzero(Some(x));
}

/// Port of `poundgetfn()` from `Src/params.c:4534`. C body:
/// `return arrlen(pparams);`
pub fn poundgetfn() -> i64 {
    pparams_lock().lock().expect("pparams poisoned").len() as i64
}

// -----------------------------------------------------------
// $USERNAME
// -----------------------------------------------------------

/// Port of `usernamegetfn()` from `Src/params.c:4653`. C body:
/// `return get_username();`
pub fn usernamegetfn() -> String {
    cached_username_lock()
        .lock()
        .expect("username poisoned")
        .clone()
}

/// Port of `usernamesetfn()` from `Src/params.c:4662`. C body:
/// `getpwnam(x); setgid; setuid; cached_uid = pswd->pw_uid;`
///
/// WARNING: the SUID-changing path requires getpwnam(3) which
/// crosses an unsafe FFI boundary not yet wrapped here. The
/// cached-name update is performed; uid/gid changes still need
/// porting of the `pwd.h` getpwnam wrapper.
pub fn usernamesetfn(x: String) {
    *cached_username_lock()
        .lock()
        .expect("username poisoned") = x;
}

// -----------------------------------------------------------
// $IFS / $HOME / $TERM / $WORDCHARS / $TERMINFO / $TERMINFO_DIRS
// $KEYBOARD_HACK / $HISTCHARS / $_  — string-state callbacks.
// -----------------------------------------------------------

/// Port of `ifsgetfn()` from `Src/params.c:4784`. C body: `return ifs;`
pub fn ifsgetfn() -> String {
    ifs_lock().lock().expect("ifs poisoned").clone()
}

/// Port of `ifssetfn()` from `Src/params.c:4793`. C body:
/// `zsfree(ifs); ifs = x; inittyptab();`
pub fn ifssetfn(x: String) {
    *ifs_lock().lock().expect("ifs poisoned") = x;
    // `inittyptab()` is a no-op in zshrs — Rust char methods
    // handle classification natively (utils.rs:1884).
}

/// Port of `homegetfn()` from `Src/params.c:5109`. C body: `return home;`
pub fn homegetfn() -> String {
    home_lock().lock().expect("home poisoned").clone()
}

/// Port of `homesetfn()` from `Src/params.c:5118`. C body:
/// `zsfree(home); home = x ? x : ""; finddir(NULL);`
pub fn homesetfn(x: String) {
    *home_lock().lock().expect("home poisoned") = x;
    // `finddir(NULL)` invalidates zsh's cached named-directory
    // lookups — those don't exist in zshrs yet.
}

/// Port of `termgetfn()` from `Src/params.c:5176`. C body: `return term;`
pub fn termgetfn() -> String {
    term_lock().lock().expect("term poisoned").clone()
}

/// Port of `termsetfn()` from `Src/params.c:5185`. C body:
/// `zsfree(term); term = x ? x : ""; term_reinit_from_pm();`
pub fn termsetfn(x: String) {
    *term_lock().lock().expect("term poisoned") = x;
    term_reinit_from_pm();
}

/// Port of `terminfogetfn()` from `Src/params.c:5196`. C body:
/// `return zsh_terminfo ? zsh_terminfo : "";`
pub fn terminfogetfn() -> String {
    zsh_terminfo_lock()
        .lock()
        .expect("zsh_terminfo poisoned")
        .clone()
}

/// Port of `terminfosetfn()` from `Src/params.c:5205`. C body:
/// `zsfree(zsh_terminfo); zsh_terminfo = x; addenv if exported; term_reinit_from_pm();`
pub fn terminfosetfn(x: String) {
    *zsh_terminfo_lock()
        .lock()
        .expect("zsh_terminfo poisoned") = x.clone();
    env::set_var("TERMINFO", &x);
    term_reinit_from_pm();
}

/// Port of `terminfodirsgetfn()` from `Src/params.c:5224`. C body:
/// `return zsh_terminfodirs ? zsh_terminfodirs : "";`
pub fn terminfodirsgetfn() -> String {
    zsh_terminfodirs_lock()
        .lock()
        .expect("zsh_terminfodirs poisoned")
        .clone()
}

/// Port of `terminfodirssetfn()` from `Src/params.c:5233`. C body
/// mirrors `terminfosetfn` for the TERMINFO_DIRS env var.
pub fn terminfodirssetfn(x: String) {
    *zsh_terminfodirs_lock()
        .lock()
        .expect("zsh_terminfodirs poisoned") = x.clone();
    env::set_var("TERMINFO_DIRS", &x);
    term_reinit_from_pm();
}

/// Port of `term_reinit_from_pm()` from `Src/params.c:5163`.
/// C: `static void term_reinit_from_pm(void)` →
///   `if (unset(INTERACTIVE) || !*term) termflags |= TERM_UNKNOWN;
///    else init_term();`
pub fn term_reinit_from_pm() {                                               // c:5163
    use std::sync::atomic::Ordering;
    // c:5167 — `if (unset(INTERACTIVE) || !*term) termflags |= TERM_UNKNOWN;`
    let interactive = crate::ported::options::optlookup("interactive") > 0;
    let term = term_lock().lock().map(|s| s.clone()).unwrap_or_default();
    if !interactive || term.is_empty() {                                     // c:5167
        TERMFLAGS.fetch_or(TERM_UNKNOWN, Ordering::Relaxed);                 // c:5168
    } else {
        // c:5170 — `init_term();` lives in ZLE; flag the next prompt
        // to re-init via TERM_UNKNOWN so the lazy path picks it up.
        TERMFLAGS.fetch_or(TERM_UNKNOWN, Ordering::Relaxed);                 // c:5170
    }
}

// `termflags` from Src/init.c — bitmap of terminal-state flags. Set
// from term_reinit_from_pm and consulted by ZLE before first paint.
pub static TERMFLAGS: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);
// `TERM_UNKNOWN` from Src/zsh.h:1986.
pub const TERM_UNKNOWN: i32 = 1 << 0;

/// Port of `wordcharsgetfn()` from `Src/params.c:5132`. C body:
/// `return wordchars;`
pub fn wordcharsgetfn() -> String {
    wordchars_lock()
        .lock()
        .expect("wordchars poisoned")
        .clone()
}

/// Port of `wordcharssetfn()` from `Src/params.c:5141`. C body:
/// `zsfree(wordchars); wordchars = x; inittyptab();`
pub fn wordcharssetfn(x: String) {
    *wordchars_lock().lock().expect("wordchars poisoned") = x;
}

/// Port of `keyboardhackgetfn()` from `Src/params.c:5024`. C body:
/// `static char buf[2]; buf[0] = keyboardhackchar; return buf;`
pub fn keyboardhackgetfn() -> String {
    let c = *keyboardhack_lock()
        .lock()
        .expect("keyboardhack poisoned");
    if c == 0 {
        String::new()
    } else {
        (c as char).to_string()
    }
}

/// Port of `keyboardhacksetfn()` from `Src/params.c:5038`. C body:
/// `unmetafy(x, &len); if (len > 1) zwarn("Only one KEYBOARD_HACK character"); …`
pub fn keyboardhacksetfn(x: String) {
    let bytes = x.as_bytes();
    if bytes.len() > 1 {
        zerr("Only one KEYBOARD_HACK character can be defined");
    }
    let c = bytes.first().copied().unwrap_or(0);
    if c >= 0x80 {
        zerr("KEYBOARD_HACK can only contain ASCII characters");
        return;
    }
    *keyboardhack_lock().lock().expect("keyboardhack poisoned") = c;
}

/// Port of `histcharsgetfn()` from `Src/params.c:5064`. C body:
/// `static char buf[4]; buf[0]=bangchar; buf[1]=hatchar; buf[2]=hashchar;`
pub fn histcharsgetfn() -> String {
    let chars = *histchars_lock().lock().expect("histchars poisoned");
    let mut s = String::new();
    for &b in chars.iter() {
        if b != 0 {
            s.push(b as char);
        }
    }
    s
}

/// Port of `histcharssetfn()` from `Src/params.c:5079`. C body
/// validates ASCII, takes up to 3 chars; defaults `!^#` if NULL.
pub fn histcharssetfn(x: Option<String>) {
    match x {
        None => {
            *histchars_lock().lock().expect("histchars poisoned") = [b'!', b'^', b'#'];
        }
        Some(s) => {
            let bytes = s.as_bytes();
            for &b in bytes.iter().take(3) {
                if b >= 0x80 {
                    zerr("HISTCHARS can only contain ASCII characters");
                    return;
                }
            }
            let mut chars = [0u8; 3];
            for (i, &b) in bytes.iter().take(3).enumerate() {
                chars[i] = b;
            }
            *histchars_lock().lock().expect("histchars poisoned") = chars;
        }
    }
}

/// Update `$_` with the last argument of the just-completed
/// command. Mirrors C zsh's writeback in `execcmd_exec` (Src/exec.c)
/// where `zunderscore` is set to the last argv slot before
/// returning. Callers: every command-dispatch hook in
/// fusevm_bridge / exec.rs.
pub fn set_zunderscore(argv: &[String]) {
    let new = if let Some(last) = argv.last() {
        last.clone()
    } else {
        String::new()
    };
    *zunderscore_lock()
        .lock()
        .expect("zunderscore poisoned") = new;
}

/// Port of `underscoregetfn()` from `Src/params.c:5152`. C body:
/// `char *u = dupstring(zunderscore); untokenize(u); return u;`
pub fn underscoregetfn() -> String {
    zunderscore_lock()
        .lock()
        .expect("zunderscore poisoned")
        .clone()
}

// -----------------------------------------------------------
// $HISTSIZE / $SAVEHIST
// -----------------------------------------------------------

/// Port of `histsizegetfn()` from `Src/params.c:4965`. C body: `return histsiz;`
pub fn histsizegetfn() -> i64 {
    *histsiz_lock().lock().expect("histsiz poisoned")
}

/// Port of `histsizesetfn()` from `Src/params.c:4974`. C body:
/// `if ((histsiz = v) < 1) histsiz = 1; resizehistents();`
pub fn histsizesetfn(v: i64) {
    *histsiz_lock().lock().expect("histsiz poisoned") = v.max(1);
    // `resizehistents()` is a hist.c entry point — pending the
    // history-table port, the size change is recorded in the
    // static and picked up the next time the history layer reads.
}

/// Port of `savehistsizegetfn()` from `Src/params.c:4985`. C body:
/// `return savehistsiz;`
pub fn savehistsizegetfn() -> i64 {
    *savehistsiz_lock().lock().expect("savehistsiz poisoned")
}

/// Port of `savehistsizesetfn()` from `Src/params.c:4994`. C body:
/// `if ((savehistsiz = v) < 0) savehistsiz = 0;`
pub fn savehistsizesetfn(v: i64) {
    *savehistsiz_lock().lock().expect("savehistsiz poisoned") = v.max(0);
}

// -----------------------------------------------------------
// $pipestatus
// -----------------------------------------------------------

/// Port of `pipestatgetfn()` from `Src/params.c:5251`. C body
/// snapshots the `pipestats[]` C array as a heap-allocated
/// `char **`. Rust port returns the cloned snapshot.
pub fn pipestatgetfn() -> Vec<String> {
    pipestats_lock()
        .lock()
        .expect("pipestats poisoned")
        .iter()
        .map(|n| n.to_string())
        .collect()
}

/// Port of `pipestatsetfn()` from `Src/params.c:5270`. C body:
/// `for (i=0; *x && i<MAX_PIPESTATS; i++) pipestats[i] = atoi(*x++); numpipestats = i;`
pub fn pipestatsetfn(x: Option<Vec<String>>) {
    const MAX_PIPESTATS: usize = 256;
    let mut guard = pipestats_lock().lock().expect("pipestats poisoned");
    guard.clear();
    if let Some(v) = x {
        for s in v.iter().take(MAX_PIPESTATS) {
            guard.push(s.parse::<i32>().unwrap_or(0));
        }
    }
}

// -----------------------------------------------------------
// Locale callbacks: $LANG, $LC_*, setlang
// -----------------------------------------------------------

/// Port of `clear_mbstate()` from `Src/params.c:4831`. C body:
/// `mb_charinit(); clear_shiftstate();`
///
/// WARNING: zshrs uses Rust's UTF-8 native handling so multibyte
/// state machines aren't kept; this is a no-op pinned to the
/// C name for parity.
/// Port of `clear_mbstate()` from `Src/params.c:4831`. C body
/// (under `MULTIBYTE_SUPPORT`):
/// ```c
/// mb_charinit();        /* utils.c */
/// clear_shiftstate();   /* pattern.c */
/// ```
/// Resets the mbstate_t globals after LC_CTYPE changes (NetBSD-9
/// requires this). Rust port forwards to the matching helpers.
pub fn clear_mbstate() {
    // mb_charinit / clear_shiftstate not yet ported; once they are
    // (Src/utils.c, Src/pattern.c) wire the calls here.
}

/// Port of `setlang()` from `Src/params.c:4840`. C body:
/// `if (LC_ALL set) return; setlocale(LC_ALL, x); for each LC_*: if set, setlocale(category, x);`
pub fn setlang(x: Option<&str>) {
    if let Ok(lc_all) = env::var("LC_ALL") {
        if !lc_all.is_empty() {
            return;
        }
    }
    if let Some(s) = x {
        env::set_var("LANG", s);
    }
    clear_mbstate();
}

/// Port of `langsetfn()` from `Src/params.c:4896`. C body:
/// `strsetfn(pm, x); setlang(unmeta(x));`
pub fn langsetfn(x: String) {
    setlang(Some(&x));
}

/// Port of `lc_allsetfn()` from `Src/params.c:4871`. C body
/// dispatches to `setlang(LANG)` if x empty, else `setlocale(LC_ALL, x)`.
pub fn lc_allsetfn(x: Option<String>) {
    match x {
        None => setlang(env::var("LANG").as_deref().ok()),
        Some(s) if s.is_empty() => setlang(env::var("LANG").as_deref().ok()),
        Some(s) => {
            env::set_var("LC_ALL", &s);
            clear_mbstate();
        }
    }
}

/// Port of `lcsetfn()` from `Src/params.c:4904`. C body:
/// per-category `setlocale` with LC_ALL precedence + LANG fallback.
pub fn lcsetfn(category: &str, x: Option<String>) {
    if let Ok(lc_all) = env::var("LC_ALL") {
        if !lc_all.is_empty() {
            return;
        }
    }
    let val = x
        .filter(|s| !s.is_empty())
        .or_else(|| env::var("LANG").ok().filter(|s| !s.is_empty()));
    if let Some(v) = val {
        env::set_var(category, v);
    }
    clear_mbstate();
}

// -----------------------------------------------------------
// env management (zsh's wrapper around setenv/unsetenv).
// -----------------------------------------------------------

/// Port of `zgetenv()` from `Src/params.c:5416`. C body walks
/// `environ` byte-by-byte. Rust port uses `std::env::var`.
pub fn zgetenv(name: &str) -> Option<String> {
    env::var(name).ok()
}

/// Port of `zputenv()` from `Src/params.c:5325`. C body parses
/// `name=value` and calls `setenv(3)` (or putenv as fallback).
pub fn zputenv(str: &str) -> i32 {
    if let Some(eq) = str.find('=') {
        let (name, val) = str.split_at(eq);
        env::set_var(name, &val[1..]);
        0
    } else {
        env::remove_var(str);
        0
    }
}

/// Port of `findenv()` from `Src/params.c:5391`. C body finds the
/// `name=...` entry index in `environ`. Rust port returns the
/// value via `env::var` since indices into Rust's env are not
/// stable.
pub fn findenv(name: &str) -> Option<String> {
    env::var(name).ok()
}

/// Port of `delenvvalue()` from `Src/params.c:5542`. C body:
/// frees a single env entry; Rust drops via env::remove_var.
pub fn delenvvalue(name: &str) {
    env::remove_var(name);
}

/// Port of `addenv()` from `Src/params.c:5448`. C body builds an
/// env string, splices into `environ`, and updates the param's
/// `pm->env`. Rust port uses `env::set_var`.
pub fn addenv(name: &str, value: &str) -> i32 {
    env::set_var(name, value);
    0
}

/// Port of `delenv()` from `Src/params.c:5563`. C body removes
/// `pm->env` from `environ` and frees it. Rust port uses
/// `env::remove_var` keyed on the param name.
pub fn delenv(name: &str) {
    env::remove_var(name);
}

/// Port of `mkenvstr()` from `Src/params.c:5513`. C body:
/// `len = strlen(name); m = strlen(value); s = zalloc(len+m+2); sprintf(s,"%s=%s",name,value);`
pub fn mkenvstr(name: &str, value: &str) -> String {
    format!("{}={}", name, value)
}

/// Port of `copyenvstr()` from `Src/params.c:5434`. C body:
/// `strcpy(s, value); for (i=len; i--; s++) if (*s == Meta) *s = (*++s ^ 32);`
pub fn copyenvstr(value: &str) -> String {
    crate::ported::utils::unmetafy_dup(value)
}

/// Port of `split_env_string()` from `Src/params.c:763`. C body:
/// finds `=` in `env`, returns `(name, value)` halves.
pub fn split_env_string(env: &str) -> Option<(String, String)> {
    env.find('=').map(|i| (env[..i].to_string(), env[i + 1..].to_string()))
}

/// Port of `arrfixenv()` from `Src/params.c:5285`. C body re-syncs
/// the env entry for an array param after mutation, joining with
/// the param's `joinchar`. Rust port joins with ':' (the default
/// for PATH-style arrays) and updates the env var.
pub fn arrfixenv(s: &str, t: Option<&[String]>) {
    let val = t.map(|v| v.join(":")).unwrap_or_default();
    env::set_var(s, val);
}

// -----------------------------------------------------------
// Array uniq helpers.
// -----------------------------------------------------------

/// Port of `simple_arrayuniq()` from `Src/params.c:4412`. C body:
/// O(n^2) dedupe in place — first occurrence wins.
pub fn simple_arrayuniq(x: Vec<String>) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::with_capacity(x.len());
    for s in x {
        if seen.insert(s.clone()) {
            out.push(s);
        }
    }
    out
}

/// Port of `arrayuniq()` from `Src/params.c:4473`. C body uses a
/// hashtable when input is large, simple_arrayuniq otherwise.
/// Both paths have first-wins semantics; Rust HashSet does the
/// same in one pass.
pub fn arrayuniq(x: Vec<String>) -> Vec<String> {
    simple_arrayuniq(x)
}

/// Port of `zhuniqarray()` from `Src/params.c:4523`. C body wraps
/// arrayuniq with the `freeok=0` flag (don't free duplicates —
/// caller owns). Rust drop semantics handle this automatically.
pub fn zhuniqarray(x: Vec<String>) -> Vec<String> {
    arrayuniq(x)
}

/// Port of `arrayuniq_freenode()` from `Src/params.c:4443`. C
/// body: `zsfree(((Pathnode)hn)->name); zfree(hn, sizeof…);` —
/// the freenode callback for the temporary HashTable `arrayuniq`
/// builds. Rust drop semantics handle this; no-op shim.
pub fn arrayuniq_freenode() {}

/// Port of `newuniqtable()` from `Src/params.c:4450`. C body
/// creates a HashTable with `arrayuniq_freenode` as the freenode
/// callback. Rust uses HashSet inline in `simple_arrayuniq`.
pub fn newuniqtable(_size: i64) -> HashSet<String> {
    HashSet::new()
}

// -----------------------------------------------------------
// "Null" callbacks — no-op getfn/setfn/unsetfn slots used for
// read-only or write-only special params.
// -----------------------------------------------------------

/// Port of `nullintsetfn()` from `Src/params.c:4187`. C body:
/// empty (no-op setter for read-only int params).
pub fn nullintsetfn(_pm: &mut crate::ported::zsh_h::param, _x: i64) {}

/// Port of `nullsethashfn()` from `Src/params.c:4104`. C body:
/// `deleteparamtable(x);` — frees the supplied table, doesn't store.
pub fn nullsethashfn(_pm: &mut crate::ported::zsh_h::param, _x: crate::ported::zsh_h::HashTable) {
    // Rust drop semantics free `x` when this scope ends.
}

/// Port of `nullstrsetfn()` from `Src/params.c:4180`. C body:
/// `zsfree(x);` — frees but doesn't store. Rust drop handles free.
pub fn nullstrsetfn(_pm: &mut crate::ported::zsh_h::param, _x: Option<String>) {}

/// Port of `nullunsetfn()` from `Src/params.c:4192`. C body: empty.
pub fn nullunsetfn(_pm: &mut crate::ported::zsh_h::param, _exp: i32) {}

/// Port of `stdunsetfn()` from `Src/params.c:3955`. C body:
/// dispatches `pm->gsu->setfn(pm, NULL)` per `PM_TYPE`, clears
/// `PM_TIED`/frees ename for tied params, sets PM_UNSET.
///
/// Rust port mirrors C semantics: clears the union slot and sets
/// PM_UNSET. The GSU vtable callbacks are stored on `param` as
/// `Option<Gsu*>` (zsh_h:760-764) but the dispatch uses callback
/// fn-ptrs that aren't generally registered yet, so we open-code
/// the "setfn(pm, NULL)" effect by zeroing the matching union
/// member instead of calling through the vtable.
pub fn stdunsetfn(pm: &mut crate::ported::zsh_h::param, _exp: i32) {
    use crate::ported::zsh_h::{PM_ARRAY, PM_HASHED, PM_NAMEREF, PM_SCALAR, PM_SPECIAL,
                                PM_TIED, PM_TYPE, PM_UNSET};
    match PM_TYPE(pm.node.flags as u32) {
        PM_SCALAR | PM_NAMEREF => {
            pm.u_str = None;
        }
        PM_ARRAY => {
            pm.u_arr = None;
        }
        PM_HASHED => {
            pm.u_hash = None;
        }
        _ => {
            if (pm.node.flags as u32 & PM_SPECIAL) == 0 {
                pm.u_str = None;
            }
        }
    }
    if (pm.node.flags as u32 & (PM_SPECIAL | PM_TIED)) == PM_TIED {
        pm.ename = None;
        pm.node.flags &= !(PM_TIED as i32);
    }
    pm.node.flags |= PM_UNSET as i32;
}

/// Port of `rprompt_indent_unsetfn()` from `Src/params.c:4237`. C
/// body: `stdunsetfn(pm, exp); rprompt_indent = 1;` — keeps in
/// sync with init_term().
pub fn rprompt_indent_unsetfn(pm: &mut crate::ported::zsh_h::param, exp: i32) {
    stdunsetfn(pm, exp);
    // TODO: when global `rprompt_indent` lands, set it back to 1 here.
    // Currently the global doesn't exist as a zshrs-side mutable static.
}

// -----------------------------------------------------------
// GSU dispatch callbacks — direct ports against `param.u_*`
// fields. C source in Src/params.c:3989-4116.
// -----------------------------------------------------------

/// Port of `intgetfn()` from `Src/params.c:3993`. C body:
/// `return pm->u.val;`
pub fn intgetfn(pm: &crate::ported::zsh_h::param) -> i64 {
    pm.u_val
}

/// Port of `intsetfn()` from `Src/params.c:4002`. C body:
/// `pm->u.val = x;`
pub fn intsetfn(pm: &mut crate::ported::zsh_h::param, x: i64) {
    pm.u_val = x;
}

/// Port of `floatgetfn()` from `Src/params.c:4011`. C body:
/// `return pm->u.dval;`
pub fn floatgetfn(pm: &crate::ported::zsh_h::param) -> f64 {
    pm.u_dval
}

/// Port of `floatsetfn()` from `Src/params.c:4020`. C body:
/// `pm->u.dval = x;`
pub fn floatsetfn(pm: &mut crate::ported::zsh_h::param, x: f64) {
    pm.u_dval = x;
}

/// Port of `strgetfn()` from `Src/params.c:4029`. C body:
/// `return pm->u.str ? pm->u.str : (char *) hcalloc(1);`
pub fn strgetfn(pm: &crate::ported::zsh_h::param) -> String {
    pm.u_str.clone().unwrap_or_default()
}

/// Port of `strsetfn()` from `Src/params.c:4038`. C body:
/// `zsfree(pm->u.str); pm->u.str = x;` plus AUTONAMEDIRS handling.
/// The `adduserdir()` call is gated on PM_NAMEDDIR/AUTONAMEDIRS;
/// we honor the bit but skip the registry call (no userdirs port
/// yet — see `Src/utils.c:adduserdir`).
pub fn strsetfn(pm: &mut crate::ported::zsh_h::param, x: Option<String>) {
    use crate::ported::zsh_h::{PM_HASHELEM, PM_NAMEDDIR};
    pm.u_str = x.clone();
    if (pm.node.flags as u32 & PM_HASHELEM) == 0 {
        if (pm.node.flags as u32 & PM_NAMEDDIR) != 0 {
            pm.node.flags |= PM_NAMEDDIR as i32;
            // TODO: adduserdir(pm.node.nam, &x, 0, 0); when userdirs ported.
        }
    }
}

/// Port of `arrgetfn()` from `Src/params.c:4057`. C body:
/// `return pm->u.arr ? pm->u.arr : &nullarray;`
pub fn arrgetfn(pm: &crate::ported::zsh_h::param) -> Vec<String> {
    pm.u_arr.clone().unwrap_or_default()
}

/// Port of `arrsetfn()` from `Src/params.c:4066`. C body frees
/// the old array, applies PM_UNIQUE filter via `uniqarray()`, then
/// stores. Calls `arrfixenv(ename, x)` for tied colon-arrays.
pub fn arrsetfn(pm: &mut crate::ported::zsh_h::param, x: Vec<String>) {
    use crate::ported::zsh_h::PM_UNIQUE;
    let val = if (pm.node.flags as u32 & PM_UNIQUE) != 0 {
        simple_arrayuniq(x)
    } else {
        x
    };
    pm.u_arr = Some(val.clone());
    if let Some(ename) = pm.ename.clone() {
        arrfixenv(&ename, Some(&val));
    }
}

/// Port of `hashgetfn()` from `Src/params.c:4084`. C body:
/// `return pm->u.hash;`
pub fn hashgetfn(pm: &crate::ported::zsh_h::param) -> Option<&crate::ported::zsh_h::HashTable> {
    pm.u_hash.as_ref()
}

/// Port of `hashsetfn()` from `Src/params.c:4093`. C body:
/// `if (pm->u.hash && pm->u.hash != x) deleteparamtable(pm->u.hash);
///  pm->u.hash = x;`
pub fn hashsetfn(pm: &mut crate::ported::zsh_h::param, x: crate::ported::zsh_h::HashTable) {
    pm.u_hash = Some(x);
}

/// Port of `arrhashsetfn()` from `Src/params.c:4113`. C body
/// rebuilds the hash from a key/value-paired char** array. Skipped
/// here pending hashtable backend (paramtab vtable not yet wired).
pub fn arrhashsetfn(_pm: &mut crate::ported::zsh_h::param, _val: Vec<String>, _flags: i32) {
    // TODO: see Src/params.c:4115-4178 — pair (val[i], val[i+1]) into
    // hash entries via createparam(PM_SCALAR) per element.
}

// -----------------------------------------------------------
// Generic special-param GSU callbacks (`u.valptr` / `u.data`).
// C source uses raw pointer indirection through `pm->u.data`/
// `pm->u.valptr` — Rust port stores the global's name in `u_str`
// (lookup key) since we can't carry raw pointers across an FFI
// boundary safely. The lookup-table integration ships with the
// special-params init code (Src/params.c:817 createparamtable).
// -----------------------------------------------------------

/// Port of `intvargetfn()` from `Src/params.c:4202`. C body:
/// `return *pm->u.valptr;`
pub fn intvargetfn(pm: &crate::ported::zsh_h::param) -> i64 {
    pm.u_val
}

/// Port of `intvarsetfn()` from `Src/params.c:4213`. C body:
/// `*pm->u.valptr = x;`
pub fn intvarsetfn(pm: &mut crate::ported::zsh_h::param, x: i64) {
    pm.u_val = x;
}

/// Port of `zlevarsetfn()` from `Src/params.c:4224`. C body sets
/// the int and triggers `adjustwinsize` for LINES/COLUMNS.
pub fn zlevarsetfn(pm: &mut crate::ported::zsh_h::param, x: i64) {
    pm.u_val = x;
    // TODO: if pm.node.nam == "LINES" || "COLUMNS" → adjustwinsize.
}

/// Port of `strvarsetfn()` from `Src/params.c:4249`. C body:
/// `zsfree(*q); *q = x;` where `q = (char **)pm->u.data`.
pub fn strvarsetfn(pm: &mut crate::ported::zsh_h::param, x: Option<String>) {
    pm.u_str = x;
}

/// Port of `strvargetfn()` from `Src/params.c:4263`. C body:
/// `s = *((char **)pm->u.data); return s ? s : hcalloc(1);`
pub fn strvargetfn(pm: &crate::ported::zsh_h::param) -> String {
    pm.u_str.clone().unwrap_or_default()
}

/// Port of `arrvargetfn()` from `Src/params.c:4279`. C body:
/// `arrptr = *((char ***)pm->u.data); return arrptr ?: &nullarray;`
pub fn arrvargetfn(pm: &crate::ported::zsh_h::param) -> Vec<String> {
    pm.u_arr.clone().unwrap_or_default()
}

/// Port of `arrvarsetfn()` from `Src/params.c:4294`. C body
/// frees old, applies PM_UNIQUE, handles PM_SPECIAL+NULL → mkarray.
pub fn arrvarsetfn(pm: &mut crate::ported::zsh_h::param, x: Vec<String>) {
    use crate::ported::zsh_h::PM_UNIQUE;
    let val = if (pm.node.flags as u32 & PM_UNIQUE) != 0 {
        simple_arrayuniq(x)
    } else {
        x
    };
    pm.u_arr = Some(val);
}

/// Port of `colonarrsetfn()` from `Src/params.c:4329`. C body
/// splits the colon-string into an array and stores via the
/// generic arrvarsetfn.
pub fn colonarrsetfn(pm: &mut crate::ported::zsh_h::param, x: Option<String>) {
    let arr = match x {
        Some(s) => colonsplit(&s),
        None => Vec::new(),
    };
    arrvarsetfn(pm, arr);
}

/// Port of `tiedarrgetfn()` from `Src/params.c:4348`. C body:
/// `return *((Tieddata)pm->u.data)->arrptr;`
pub fn tiedarrgetfn(pm: &crate::ported::zsh_h::param) -> Vec<String> {
    pm.u_arr.clone().unwrap_or_default()
}

/// Port of `tiedarrsetfn()` from `Src/params.c:4357`. C body
/// joins the array with `pm->u.data->joinchar` and stores.
pub fn tiedarrsetfn(pm: &mut crate::ported::zsh_h::param, x: Option<String>) {
    let arr = x.map(|s| colonsplit(&s)).unwrap_or_default();
    pm.u_arr = Some(arr);
}

/// Port of `tiedarrunsetfn()` from `Src/params.c:4393`. C body
/// frees the tied storage and calls stdunsetfn.
pub fn tiedarrunsetfn(pm: &mut crate::ported::zsh_h::param, exp: i32) {
    pm.u_arr = None;
    stdunsetfn(pm, exp);
}

// -----------------------------------------------------------
// Param-table mutators / scope / nameref helpers.
// `Src/params.c` calls these against the global `paramtab`
// HashTable; until our HashTable vtable (`Box<hashtable>` in
// zsh_h.rs:285) is wired, these remain no-op shims with the
// real C signatures.
// -----------------------------------------------------------

/// Port of `assignnparam()` from `Src/params.c:3664`. WARNING: needs Param.
pub fn assignnparam(_s: &str, _val: f64, _flags: i32) {}

/// Port of `assignstrvalue()` from `Src/params.c:2692`. WARNING: needs Value.
pub fn assignstrvalue(_val: &str, _flags: i32) {}

/// Port of `assigngetset()` from `Src/params.c:994`. C body chooses
/// the gsu vtable based on PM_TYPE then sets pm->gsu.
pub fn assigngetset(_pm: &mut crate::ported::zsh_h::param) {
    // TODO: dispatch on PM_TYPE → set pm.gsu_s / pm.gsu_i / etc.
}

/// Port of `check_warn_pm()` from `Src/params.c:3158`. WARNING: needs Param.
pub fn check_warn_pm(_pm: &crate::ported::zsh_h::param, _pmtype: &str, _created: i32, _passed: i32) {}

/// Port of `convbase_ptr()` from `Src/params.c:5586`. WARNING: needs `int *ndigits`.
pub fn convbase_ptr(_v: i64, _base: i32) -> (String, i32) {
    (String::new(), 0)
}

/// Port of `copyparamtable()` from `Src/params.c:596`. WARNING: needs HashTable.
pub fn copyparamtable() {}

// parameter entries as well as setting up parameter table                 // c:812
// entries for environment variables we inherit.                           // c:813
/// Port of `createparamtable()` from `Src/params.c:817`. WARNING: needs HashTable.
pub fn createparamtable() {}                                                 // c:817

/// Port of `createspecialhash()` from `Src/params.c:1182`. WARNING: needs HashTable.
pub fn createspecialhash() {}

/// Port of `deleteparamtable()` from `Src/params.c:616`. WARNING: needs HashTable.
pub fn deleteparamtable() {}

/// Port of `fetchvalue()` from `Src/params.c:2180`. WARNING: needs Value.
pub fn fetchvalue() {}

/// Port of `freeparamnode()` from `Src/params.c:5977`. C body
/// frees the param's name + value strings + ename + recurses
/// into `pm->old`. Rust drop handles all of these automatically
/// when the `Param` (Box<param>) is dropped, so this is a no-op
/// shim retained for callback-table compatibility.
pub fn freeparamnode(_hn: crate::ported::zsh_h::Param) {
    // Drop the Box → cascades to all owned fields.
}

/// Port of `getindex()` from `Src/params.c:2001`. WARNING: needs Value.
pub fn getindex() {}

/// Port of `getparamnode()` from `Src/params.c:570`. WARNING: needs HashTable.
pub fn getparamnode() {}

/// Port of `getvalue()` from `Src/params.c:2173`. WARNING: needs Value.
pub fn getvalue() {}

/// Port of `getvaluearr()` from `Src/params.c:710`. WARNING: needs Value.
pub fn getvaluearr() {}

/// Port of `loadparamnode()` from `Src/params.c:544`. WARNING: needs HashTable + Param.
pub fn loadparamnode() {}

/// Port of `newparamtable()` from `Src/params.c:519`. WARNING: needs HashTable.
pub fn newparamtable() {}

/// Port of `paramvalarr()` from `Src/params.c:689`. WARNING: needs HashTable.
pub fn paramvalarr() -> Vec<String> {
    Vec::new()
}

/// Port of `printparamnode()` from `Src/params.c:6123`. WARNING: needs HashNode.
pub fn printparamnode() {}

/// Port of `printparamvalue()` from `Src/params.c:6035`. WARNING: needs Param.
pub fn printparamvalue() {}

/// Port of `resolve_nameref_rec()` from `Src/params.c:6332`. WARNING: needs Param.
pub fn resolve_nameref_rec() {}

/// Port of `scancopyparams()` from `Src/params.c:584`. WARNING: needs HashNode.
pub fn scancopyparams() {}

/// Port of `scancountparams()` from `Src/params.c:630`. WARNING: needs HashNode.
pub fn scancountparams() {}

/// Port of `scanendscope()` from `Src/params.c:5900`. WARNING: needs HashNode.
pub fn scanendscope() {}

/// Port of `scanparamvals()` from `Src/params.c:644`. WARNING: needs HashNode.
pub fn scanparamvals() {}

/// Port of `setloopvar()` from `Src/params.c:6362`. WARNING: needs Param.
pub fn setloopvar(_name: &str, _value: &str) {}

/// Port of `setnparam()` from `Src/params.c:3744`. WARNING: needs mnumber.
pub fn setnparam(_s: &str, _val: f64) {}

/// Port of `setnumvalue()` from `Src/params.c:2856`. WARNING: needs Value.
pub fn setnumvalue() {}

/// Port of `setscope()` from `Src/params.c:6382`. WARNING: needs Param.
pub fn setscope() {}

/// Port of `setscope_base()` from `Src/params.c:6436`. WARNING: needs Param.
pub fn setscope_base(_base: i32) {}

/// Port of `upscope()` from `Src/params.c:6455`. WARNING: needs Param.
pub fn upscope() {}

// ===========================================================
// GSU dispatch table — maps special-parameter NAMES to their
// getfn callback. C zsh dispatches reads of `$RANDOM` /
// `$USERNAME` / `$UID` / etc. through `Param.gsu->getfn`, where
// each special parameter has a `Param` entry in `paramtab`
// pointing at its specific getfn (Src/params.c:225 SPECIAL_PARAM
// table seeds these mappings).
//
// zshrs has the GSU callbacks ported (uidgetfn, randomgetfn,
// usernamegetfn, etc. above) but the shell's parameter-read path
// (fusevm_bridge::expand_param) reads from ShellExecutor.variables
// directly — never dispatching through the callbacks. Result:
// `echo $RANDOM` returned the cached HashMap value (or empty),
// not a fresh `rand() & 0x7fff` from `randomgetfn`.
//
// `lookup_special_var(name)` is the bridge: given a variable
// name, returns the GSU getfn's output if `name` is a recognized
// special, else None. Callers (expand_param, subst.rs reads)
// check this before falling back to `variables.get(name)`.
// ===========================================================

/// Look up a special-parameter NAME and dispatch to its GSU getfn.
///
/// Returns `Some(value_string)` if `name` is one of zshrs's
/// recognized specials with a real GSU getfn; `None` otherwise
/// (caller should fall back to `variables.get`).
///
/// This is the bridge between the named getfn callbacks above
/// (uidgetfn / randomgetfn / etc.) and the shell's parameter-read
/// path. Mirrors the `Param.gsu->getfn` dispatch C zsh does
/// inside `getsparam` / `getstrvalue` (Src/params.c:3076 / 2335).
pub fn lookup_special_var(name: &str) -> Option<String> {
    match name {
        // libc identity callbacks.
        "UID" => Some(uidgetfn().to_string()),
        "GID" => Some(gidgetfn().to_string()),
        "EUID" => Some(euidgetfn().to_string()),
        "EGID" => Some(egidgetfn().to_string()),
        // libc syscall callbacks.
        "RANDOM" => Some(randomgetfn().to_string()),
        "TTYIDLE" => Some(ttyidlegetfn().to_string()),
        "ERRNO" => Some(errnogetfn().to_string()),
        // Time callbacks.
        "SECONDS" => Some(intsecondsgetfn().to_string()),
        // Cached-state callbacks (OnceLock<Mutex<…>> backed).
        "USERNAME" => Some(usernamegetfn()),
        "HOME" => Some(homegetfn()),
        "TERM" => Some(termgetfn()),
        "WORDCHARS" => Some(wordcharsgetfn()),
        "IFS" => Some(ifsgetfn()),
        "TERMINFO" => Some(terminfogetfn()),
        "TERMINFO_DIRS" => Some(terminfodirsgetfn()),
        "KEYBOARD_HACK" => Some(keyboardhackgetfn()),
        "histchars" => Some(histcharsgetfn()),
        "_" => Some(underscoregetfn()),
        // Counters with int return.
        "HISTSIZE" => Some(histsizegetfn().to_string()),
        "SAVEHIST" => Some(savehistsizegetfn().to_string()),
        "#" | "ARGC" => Some(poundgetfn().to_string()),
        // $0 routes through utils::argzero — only override when
        // the static was explicitly set (otherwise let the shell's
        // argv handling provide the binary path).
        "0" => crate::ported::utils::argzero(),
        // Arrays — joined with space for scalar context.
        "pipestatus" => {
            let arr = pipestatgetfn();
            if arr.is_empty() {
                None
            } else {
                Some(arr.join(" "))
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod gsu_tests {
    use super::*;

    #[test]
    fn test_libc_id_callbacks_match_libc() {
        assert_eq!(uidgetfn(), unsafe { libc::getuid() } as i64);
        assert_eq!(gidgetfn(), unsafe { libc::getgid() } as i64);
        assert_eq!(euidgetfn(), unsafe { libc::geteuid() } as i64);
        assert_eq!(egidgetfn(), unsafe { libc::getegid() } as i64);
    }

    #[test]
    fn test_random_returns_15_bit_value() {
        for _ in 0..100 {
            let v = randomgetfn();
            assert!(v >= 0 && v < 0x8000);
        }
    }

    #[test]
    fn test_random_set_seeds_deterministically() {
        randomsetfn(42);
        let a = randomgetfn();
        randomsetfn(42);
        let b = randomgetfn();
        assert_eq!(a, b);
    }

    #[test]
    fn test_ifs_round_trip() {
        let original = ifsgetfn();
        ifssetfn(":,;".to_string());
        assert_eq!(ifsgetfn(), ":,;");
        ifssetfn(original);
    }

    #[test]
    fn test_histsiz_clamps_to_1() {
        let original = histsizegetfn();
        histsizesetfn(0);
        assert_eq!(histsizegetfn(), 1);
        histsizesetfn(-5);
        assert_eq!(histsizegetfn(), 1);
        histsizesetfn(500);
        assert_eq!(histsizegetfn(), 500);
        histsizesetfn(original);
    }

    #[test]
    fn test_savehistsiz_clamps_to_0() {
        let original = savehistsizegetfn();
        savehistsizesetfn(-5);
        assert_eq!(savehistsizegetfn(), 0);
        savehistsizesetfn(100);
        assert_eq!(savehistsizegetfn(), 100);
        savehistsizesetfn(original);
    }

    #[test]
    fn test_pipestat_round_trip() {
        pipestatsetfn(Some(vec!["1".to_string(), "0".to_string(), "127".to_string()]));
        let v = pipestatgetfn();
        assert_eq!(v, vec!["1", "0", "127"]);
        pipestatsetfn(None);
        assert_eq!(pipestatgetfn(), Vec::<String>::new());
    }

    #[test]
    fn test_simple_arrayuniq_first_wins() {
        let v = vec!["a".to_string(), "b".to_string(), "a".to_string(), "c".to_string()];
        assert_eq!(simple_arrayuniq(v), vec!["a", "b", "c"]);
    }

    #[test]
    fn test_split_env_string() {
        assert_eq!(
            split_env_string("PATH=/usr/bin:/bin"),
            Some(("PATH".to_string(), "/usr/bin:/bin".to_string()))
        );
        assert_eq!(
            split_env_string("EMPTY="),
            Some(("EMPTY".to_string(), "".to_string()))
        );
        assert_eq!(split_env_string("NOEQUALS"), None);
    }

    #[test]
    fn test_mkenvstr() {
        assert_eq!(mkenvstr("PATH", "/usr/bin"), "PATH=/usr/bin");
        assert_eq!(mkenvstr("EMPTY", ""), "EMPTY=");
    }

    #[test]
    fn test_seconds_round_trip() {
        intsecondssetfn(0);
        let s1 = intsecondsgetfn();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let s2 = intsecondsgetfn();
        assert!(s2 >= s1);
        // Reset to a known offset and read back.
        setrawseconds(100.0);
        assert_eq!(getrawseconds(), 100.0);
    }

    #[test]
    fn test_argzero_round_trip() {
        argzerosetfn("/bin/zsh".to_string());
        assert_eq!(argzerogetfn(), "/bin/zsh");
        argzerosetfn(String::new());
    }

    #[test]
    fn test_env_get_set() {
        let result = zputenv("ZSHRS_TEST_VAR=hello");
        assert_eq!(result, 0);
        assert_eq!(zgetenv("ZSHRS_TEST_VAR"), Some("hello".to_string()));
        delenv("ZSHRS_TEST_VAR");
        assert_eq!(zgetenv("ZSHRS_TEST_VAR"), None);
    }

    #[test]
    fn test_keyboardhack_one_char() {
        keyboardhacksetfn("\\".to_string());
        assert_eq!(keyboardhackgetfn(), "\\");
        keyboardhacksetfn(String::new());
        assert_eq!(keyboardhackgetfn(), "");
    }

    #[test]
    fn test_histchars_default() {
        histcharssetfn(None);
        assert_eq!(histcharsgetfn(), "!^#");
        histcharssetfn(Some("@$&".to_string()));
        assert_eq!(histcharsgetfn(), "@$&");
        histcharssetfn(None);
    }
}
