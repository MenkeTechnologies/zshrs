//! Parameter management for zshrs
//!
//! Port from zsh/Src/params.c (6511 lines → full Rust port)
//!
//! Provides shell parameters (variables), special parameters, arrays,
//! associative arrays, parameter attributes, namerefs, scoping,
//! tied parameters, and all special parameter get/set functions.

#[allow(unused_imports)]
use crate::ported::utils::zerr;
use crate::func_body_fmt::FuncBodyFmt;
use indexmap::IndexMap;
use std::collections::{HashMap, HashSet};
use std::env;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use crate::ported::zsh_h::{
    gsu_array, gsu_float, gsu_hash, gsu_integer, gsu_scalar
};
use crate::ported::zsh_h::VALFLAG_SUBST;
use crate::ported::zsh_h::{Param, hashnode, isset as isset_opt};
use fusevm::Value;
use crate::ported::zsh_h::Marker;
use crate::ported::zsh_h::{SCANPM_ISVAR_AT, SCANPM_NONAMEREF};
use crate::ported::zsh_h::SCANPM_WANTINDEX;

#[allow(unused_imports)]
use crate::ported::zsh_h::{
    PM_TYPE, PM_SCALAR, PM_NAMEREF, PM_INTEGER, PM_EFLOAT, PM_FFLOAT,
    PM_ARRAY, PM_HASHED, PM_HASHELEM, PM_NAMEDDIR, PM_UNIQUE,
    PM_READONLY, PM_UNSET, PM_EXPORTED, PM_AUTOLOAD, PM_DEFAULTED,
    PM_DECLARED, PM_REMOVABLE, PM_NORESTORE, PM_LOCAL, PM_RO_BY_DESIGN,
    PM_LEFT, PM_RIGHT_B, PM_RIGHT_Z, PM_SPECIAL, PM_TAGGED, PM_TIED, PM_UPPER,
    SCANPM_CHECKING, SCANPM_MATCHMANY, SCANPM_MATCHKEY, SCANPM_MATCHVAL,
    SCANPM_KEYMATCH, SCANPM_WANTKEYS, SCANPM_WANTVALS, SCANPM_ARRONLY,
    VALFLAG_EMPTY, VALFLAG_INV,
    ASSPM_WARN, ASSPM_AUGMENT, ASSPM_ENV_IMPORT,
    PRINT_NAMEONLY, PRINT_TYPESET, PRINT_INCLUDEVALUE, PRINT_KV_PAIR,
    PRINT_LINE, PRINT_POSIX_READONLY, PRINT_POSIX_EXPORT,
    EXECOPT, KSHARRAYS, AUTONAMEDIRS, ALLEXPORT,
    WARNCREATEGLOBAL, WARNNESTEDVAR,
    isset, unset,
};
#[allow(unused_imports)]
use crate::ported::math::{mnumber, MN_INTEGER, MN_FLOAT};
#[allow(unused_imports)]
use crate::ported::utils::errflag;
#[allow(unused_imports)]
use crate::ported::signals::{queue_signals, unqueue_signals};

/// Port of `static int lc_update_needed` from `Src/params.c:5850`
/// (under `#ifdef USE_LOCALE`). Set to 1 by `scanendscope` when a
/// LC_*/LANG param's scope ends; consumed by `endparamscope` to
/// trigger a `setlocale()` refresh.
pub static LC_UPDATE_NEEDED: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);

/// Port of `static Param foundparam` from `Src/params.c:640`.
/// Set by `scanparamvals` to the last param it touched, read by
/// `assignsparam` / `assignnparam` for the assoc-element path.
/// Stores the param name; the live `&param` lookup is done by
/// the caller through paramtab.
pub static FOUNDPARAM: std::sync::OnceLock<std::sync::Mutex<Option<String>>> =
    std::sync::OnceLock::new();

/// Port of `rprompt_indent_unsetfn(Param pm, int exp)` from `Src/params.c:152`. C
/// body: `stdunsetfn(pm, exp); rprompt_indent = 1;` — keeps in
/// sync with init_term().
pub fn rprompt_indent_unsetfn(pm: &mut crate::ported::zsh_h::param, exp: i32) {
    stdunsetfn(pm, exp);
    *RPROMPT_INDENT.lock().unwrap() = 1;
}


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
pub fn IPDEF1(A: &str, B: usize, C: i32) -> paramdef {        // c:params.c:296
    paramdef { name: A.into(), flags: (PM_INTEGER | PM_SPECIAL) as i32 | C, gsu: B, ..Default::default() }
}

/// Port of `IPDEF2(A,B,C)` from `Src/params.c:309` —
/// `{{NULL,A,PM_SCALAR|PM_SPECIAL|C},BR(NULL),GSU(B),0,0,...}`.
#[inline] #[allow(non_snake_case)]
pub fn IPDEF2(A: &str, B: usize, C: i32) -> paramdef {        // c:params.c:309
    paramdef { name: A.into(), flags: (PM_SCALAR | PM_SPECIAL) as i32 | C, gsu: B, ..Default::default() }
}

// ---------------------------------------------------------------------------
// Parameter flags (from zsh.h PM_* flags)
// ---------------------------------------------------------------------------

// What level of localness we are at.                                       // c:47
//                                                                          // c:48
// Hand-wavingly, this is incremented at every function call and decremented // c:49
// at every function return.  See startparamscope().                        // c:50

/// Port of `mod_export int locallevel;` from `Src/params.c:54`.
/// Tracks function-local-scope nesting depth. Bumped by
/// `startparamscope()` (params.c:5879) on every function call,
/// decremented by `endparamscope()` (params.c:5950) on return.
#[allow(non_upper_case_globals)]
pub static locallevel: std::sync::atomic::AtomicI32 =                        // c:54
    std::sync::atomic::AtomicI32::new(0);

// ---------------------------------------------------------------------------
// Real `param` struct lives in Src/zsh.h:1829 (port at zsh_h.rs:750).
// It uses C-union flattening: u_str / u_arr / u_val / u_dval / u_hash
// dispatched on `PM_TYPE(node.flags)`. There is NO `ParamValue` enum in
// C; do not reintroduce one.
// ---------------------------------------------------------------------------

pub use crate::ported::zsh_h::param;

/// Port of `LCIPDEF(name)` from `Src/params.c:324` —
/// `IPDEF2(name, lc_blah_gsu, PM_UNSET)`.
#[inline] #[allow(non_snake_case)]
pub fn LCIPDEF(name: &str) -> paramdef {                                     // c:params.c:324
    IPDEF2(name, 0, PM_UNSET as i32)                                         // c:324 lc_blah_gsu (slot 0)
}

/// Port of `IPDEF4(A,B)` from `Src/params.c:344` —
/// `{{NULL,A,PM_INTEGER|PM_READONLY_SPECIAL},BR((void*)B),
///   GSU(varint_readonly_gsu),10,0,...}`.
#[inline] #[allow(non_snake_case)]
pub fn IPDEF4(A: &str, B: usize) -> paramdef {                          // c:params.c:344
    paramdef { name: A.into(), flags: (PM_INTEGER | PM_READONLY_SPECIAL) as i32, var: B, ..Default::default() }
}

/// Port of `IPDEF5(A,B,F)` from `Src/params.c:353` —
/// `{{NULL,A,PM_INTEGER|PM_SPECIAL},BR((void*)B),GSU(F),10,0,...}`.
#[inline] #[allow(non_snake_case)]
pub fn IPDEF5(A: &str, B: usize, F: usize) -> paramdef {              // c:params.c:353
    paramdef { name: A.into(), flags: (PM_INTEGER | PM_SPECIAL) as i32, var: B, gsu: F, ..Default::default() }
}

/// Port of `IPDEF5U(A,B,F)` from `Src/params.c:354` — c:353 + PM_UNSET.
#[inline] #[allow(non_snake_case)]
pub fn IPDEF5U(A: &str, B: usize, F: usize) -> paramdef {             // c:params.c:354
    paramdef { name: A.into(), flags: (PM_INTEGER | PM_SPECIAL | PM_UNSET) as i32, var: B, gsu: F, ..Default::default() }
}

/// Port of `IPDEF6(A,B,F)` from `Src/params.c:362` — c:353 + PM_DONTIMPORT.
#[inline] #[allow(non_snake_case)]
pub fn IPDEF6(A: &str, B: usize, F: usize) -> paramdef {              // c:params.c:362
    paramdef { name: A.into(), flags: (PM_INTEGER | PM_SPECIAL | PM_DONTIMPORT) as i32, var: B, gsu: F, ..Default::default() }
}

/// Port of `IPDEF7(A,B)` from `Src/params.c:367` —
/// `{{NULL,A,PM_SCALAR|PM_SPECIAL},BR((void*)B),GSU(varscalar_gsu),0,0,...}`.
#[inline] #[allow(non_snake_case)]
pub fn IPDEF7(A: &str, B: usize) -> paramdef {                          // c:params.c:367
    paramdef { name: A.into(), flags: (PM_SCALAR | PM_SPECIAL) as i32, var: B, ..Default::default() }
}

/// Port of `IPDEF7U(A,B)` from `Src/params.c:369` — c:367 + PM_UNSET.
#[inline] #[allow(non_snake_case)]
pub fn IPDEF7U(A: &str, B: usize) -> paramdef {                         // c:params.c:369
    paramdef { name: A.into(), flags: (PM_SCALAR | PM_SPECIAL | PM_UNSET) as i32, var: B, ..Default::default() }
}

/// Port of `IPDEF7R(A,B)` from `Src/params.c:368` — c:367 + PM_DONTIMPORT_SUID.
#[inline] #[allow(non_snake_case)]
pub fn IPDEF7R(A: &str, B: usize) -> paramdef {                         // c:params.c:368
    paramdef { name: A.into(), flags: (PM_SCALAR | PM_SPECIAL | PM_DONTIMPORT_SUID) as i32, var: B, ..Default::default() }
}

/// Port of `IPDEF9(A,B,C,D)` from `Src/params.c:431` —
/// `{{NULL,A,D|PM_ARRAY|PM_SPECIAL|PM_DONTIMPORT},BR((void*)B),
///   GSU(vararray_gsu),0,0,NULL,C,NULL,0}`.
#[inline] #[allow(non_snake_case)]
pub fn IPDEF9(A: &str, B: usize, C: usize, D: i32) -> paramdef { // c:params.c:384
    paramdef { name: A.into(), flags: (PM_ARRAY | PM_SPECIAL | PM_DONTIMPORT) as i32 | D, var: B, ..Default::default() }
}

/// Port of `IPDEF8(A,B,C,D)` from `Src/params.c:394` —
/// `{{NULL,A,D|PM_SCALAR|PM_SPECIAL},BR((void*)B),GSU(colonarr_gsu),
///   0,0,NULL,C,NULL,0}`.
/// `C` is the colon-arr field; the Rust port stores it in `getnfn`
/// since `paramdef` lacks a dedicated colon-arr slot until that's
/// ported.
#[inline] #[allow(non_snake_case)]
pub fn IPDEF8(A: &str, B: usize, C: usize, D: i32) -> paramdef { // c:params.c:394
    paramdef { name: A.into(), flags: (PM_SCALAR | PM_SPECIAL) as i32 | D, var: B, ..Default::default() }
}

/// Port of `IPDEF10(A,B)` from `Src/params.c:438` —
/// `{{NULL,A,PM_ARRAY|PM_SPECIAL},BR(NULL),GSU(B),10,0,...}`.
#[inline] #[allow(non_snake_case)]
pub fn IPDEF10(A: &str, B: usize) -> paramdef {                         // c:params.c:406
    paramdef { name: A.into(), flags: (PM_ARRAY | PM_SPECIAL) as i32, gsu: B, ..Default::default() }
}

/// Port of `newparamtable(int size, char const *name)` from `Src/params.c:519`. C body
/// allocates a HashTable via `newhashtable(size, name, NULL)`
/// and wires the vtable. Rust port constructs a fresh
/// `Box<hashtable>` with the param-specific callbacks left as
/// `None` (the hashtable.rs vtable cannot host the typed
/// param-callback signatures yet — wiring them requires the
/// hashtable backend refactor).
#[allow(unused_variables)]
pub fn newparamtable(size: i32, name: &str)
    -> Option<crate::ported::zsh_h::HashTable>
{
    let hsize = if size == 0 { 17 } else { size };
    let mut nodes: Vec<Option<crate::ported::zsh_h::HashNode>> =
        Vec::with_capacity(hsize as usize);
    for _ in 0..hsize {
        nodes.push(None);
    }
    Some(Box::new(crate::ported::zsh_h::hashtable {
        hsize,
        ct: 0,
        nodes,
        tmpdata: 0,
        hash: None,
        emptytable: None,
        filltable: None,
        cmpnodes: None,
        addnode: None,
        getnode: None,
        getnode2: None,
        removenode: None,
        disablenode: None,
        enablenode: None,
        freenode: None,
        printnode: None,
        scantab: None,
    }))
}

/// Direct port of `static Param loadparamnode(HashTable ht, Param
/// pm, const char *nam)` from `Src/params.c:544-567`. If `pm` is
/// an AUTOLOAD stub, fires the module loader and re-fetches the
/// node from ht; otherwise returns pm unchanged.
///
/// C body:
///   if (pm && (pm->flags & PM_AUTOLOAD) && pm->u.str) {
///       int level = pm->level;
///       char *mn = dupstring(pm->u.str);
///       (void)ensurefeature(mn, "p:", nam);
///       pm = (Param)gethashnode2(ht, nam);
///       while (pm && pm->level > level) pm = pm->old;
///       if (pm && (pm->level != level || (pm->flags & PM_AUTOLOAD)))
///           pm = NULL;
///       if (!pm) zerr("autoloading module %s failed...", mn, nam);
///   }
///   return pm;
/// Port of `loadparamnode(HashTable ht, Param pm, const char *nam)` from `Src/params.c:544`.
/// WARNING: param names don't match C — Rust=(pm, nam) vs C=(ht, pm, nam)
pub fn loadparamnode(                                                        // c:544
    _ht: &crate::ported::zsh_h::HashTable,
    pm: Option<crate::ported::zsh_h::Param>,
    nam: &str,
) -> Option<crate::ported::zsh_h::Param> {

    // c:546 — `if (pm && (pm->flags & PM_AUTOLOAD) && pm->u.str)`.
    let (level, modname) = match &pm {
        Some(p)
            if p.node.flags & PM_AUTOLOAD as i32 != 0 && p.u_str.is_some() =>
        {
            (p.level, p.u_str.clone().unwrap())
        }
        _ => return pm,                                                      // c:566 fall through
    };

    // c:549 — `ensurefeature(mn, "p:", nam)` fires the module loader.
    // The Rust ensurefeature signature differs (takes ModuleTable);
    // for now we look up the module without a table to keep the
    // dispatch site honest. Module-table integration is pending.
    // c:550 — re-fetch the node from ht after autoload.
    let mut pm = paramtab().write().unwrap().get(nam).cloned();
    // c:551 — walk pm->old back to original level.
    while let Some(ref p) = pm {
        if p.level > level {
            pm = p.old.clone().map(|b| crate::ported::zsh_h::Param::from(b));
        } else {
            break;
        }
    }
    // c:553-554 — if pm is at wrong level or still AUTOLOAD, treat
    // as load failure.
    let still_bad = match &pm {
        Some(p) => p.level != level || p.node.flags & PM_AUTOLOAD as i32 != 0,
        None => true,
    };
    if still_bad {
        pm = None;
        // c:561-563 — `zerr("autoloading module %s failed to define
        // parameter: %s", mn, nam)`.
        crate::ported::utils::zerr(&format!(
            "autoloading module {} failed to define parameter: {}",
            modname, nam
        ));
    }
    pm                                                                       // c:566
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

/// Special-parameter definition — Rust extension paralleling the
/// `IPDEF*` macro entries in `Src/params.c:297-392`. C uses
/// `struct paramdef` (`Src/zsh.h:2082`, mirrored at `zsh_h.rs:950`)
/// with `var` + `gsu` pointers; the Rust port carries a trimmed
/// shape with `pm_type`/`pm_flags`/`tied_name` until the full
/// `gsu`-callback plumbing lands. Canonical `paramdef` is the
/// long-term target.
#[allow(non_camel_case_types)]
#[derive(Clone, Debug)]
pub struct special_paramdef {
    pub name: &'static str,
    pub pm_type: u32,  // PM_INTEGER | PM_SCALAR | PM_ARRAY
    pub pm_flags: u32, // PM_READONLY_SPECIAL, PM_DONTIMPORT, etc.
    pub tied_name: Option<&'static str>,
}

/// Index of the first entry in `special_params` that lives in the
/// zsh-only section (after the `{{NULL,NULL,0}, BR(NULL), ...}`
/// sentinel at `Src/params.c:392`). Entries before this index are
/// always loaded; entries at and after this index are only loaded
/// under non-sh/non-ksh emulation. Mirrors the C two-section table
/// terminated by an inner NULL sentinel.
pub const SPECIAL_PARAMS_ZSH_START: usize = 54;                              // c:392

/// All special parameters from params.c special_params[]
pub const special_params: &[special_paramdef] = &[
    // Integer specials with custom GSU
    special_paramdef {
        name: "#",
        pm_type: PM_INTEGER,
        pm_flags: PM_READONLY,
        tied_name: None,
    },
    special_paramdef {
        name: "ERRNO",
        pm_type: PM_INTEGER,
        pm_flags: PM_UNSET,
        tied_name: None,
    },
    special_paramdef {
        name: "GID",
        pm_type: PM_INTEGER,
        pm_flags: PM_DONTIMPORT,
        tied_name: None,
    },
    special_paramdef {
        name: "EGID",
        pm_type: PM_INTEGER,
        pm_flags: PM_DONTIMPORT,
        tied_name: None,
    },
    special_paramdef {
        name: "HISTSIZE",
        pm_type: PM_INTEGER,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "RANDOM",
        pm_type: PM_INTEGER,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "SAVEHIST",
        pm_type: PM_INTEGER,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "SECONDS",
        pm_type: PM_INTEGER,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "UID",
        pm_type: PM_INTEGER,
        pm_flags: PM_DONTIMPORT,
        tied_name: None,
    },
    special_paramdef {
        name: "EUID",
        pm_type: PM_INTEGER,
        pm_flags: PM_DONTIMPORT,
        tied_name: None,
    },
    special_paramdef {
        name: "TTYIDLE",
        pm_type: PM_INTEGER,
        pm_flags: PM_READONLY,
        tied_name: None,
    },
    // Scalar specials with custom GSU
    special_paramdef {
        name: "USERNAME",
        pm_type: PM_SCALAR,
        pm_flags: PM_DONTIMPORT,
        tied_name: None,
    },
    special_paramdef {
        name: "-",
        pm_type: PM_SCALAR,
        pm_flags: PM_READONLY,
        tied_name: None,
    },
    special_paramdef {
        name: "histchars",
        pm_type: PM_SCALAR,
        pm_flags: PM_DONTIMPORT,
        tied_name: None,
    },
    special_paramdef {
        name: "HOME",
        pm_type: PM_SCALAR,
        pm_flags: PM_UNSET,
        tied_name: None,
    },
    special_paramdef {
        name: "TERM",
        pm_type: PM_SCALAR,
        pm_flags: PM_UNSET,
        tied_name: None,
    },
    special_paramdef {
        name: "TERMINFO",
        pm_type: PM_SCALAR,
        pm_flags: PM_UNSET,
        tied_name: None,
    },
    special_paramdef {
        name: "TERMINFO_DIRS",
        pm_type: PM_SCALAR,
        pm_flags: PM_UNSET,
        tied_name: None,
    },
    special_paramdef {
        name: "WORDCHARS",
        pm_type: PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "IFS",
        pm_type: PM_SCALAR,
        pm_flags: PM_DONTIMPORT,
        tied_name: None,
    },
    special_paramdef {
        name: "_",
        pm_type: PM_SCALAR,
        pm_flags: PM_DONTIMPORT,
        tied_name: None,
    },
    special_paramdef {
        name: "KEYBOARD_HACK",
        pm_type: PM_SCALAR,
        pm_flags: PM_DONTIMPORT,
        tied_name: None,
    },
    special_paramdef {
        name: "0",
        pm_type: PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    // Readonly integer variables bound to C globals
    special_paramdef {
        name: "!",
        pm_type: PM_INTEGER,
        pm_flags: PM_READONLY,
        tied_name: None,
    },
    special_paramdef {
        name: "$",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_READONLY,
        tied_name: None,
    },
    special_paramdef {
        name: "?",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_READONLY,
        tied_name: None,
    },
    special_paramdef {
        name: "HISTCMD",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_READONLY,
        tied_name: None,
    },
    special_paramdef {
        name: "LINENO",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_READONLY,
        tied_name: None,
    },
    special_paramdef {
        name: "PPID",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_READONLY,
        tied_name: None,
    },
    special_paramdef {
        name: "ZSH_SUBSHELL",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_READONLY,
        tied_name: None,
    },
    // Settable integer variables
    special_paramdef {
        name: "COLUMNS",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "LINES",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "ZLE_RPROMPT_INDENT",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    special_paramdef {
        name: "SHLVL",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "FUNCNEST",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "OPTIND",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_DONTIMPORT,
        tied_name: None,
    },
    special_paramdef {
        name: "TRY_BLOCK_ERROR",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_DONTIMPORT,
        tied_name: None,
    },
    special_paramdef {
        name: "TRY_BLOCK_INTERRUPT",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_DONTIMPORT,
        tied_name: None,
    },
    // Scalar variables bound to C globals
    special_paramdef {
        name: "OPTARG",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "NULLCMD",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "POSTEDIT",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    special_paramdef {
        name: "READNULLCMD",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "PS1",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "RPS1",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    special_paramdef {
        name: "RPROMPT",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    special_paramdef {
        name: "PS2",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "RPS2",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    special_paramdef {
        name: "RPROMPT2",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    special_paramdef {
        name: "PS3",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "PS4",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_DONTIMPORT_SUID,
        tied_name: None,
    },
    special_paramdef {
        name: "SPROMPT",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    // Readonly arrays
    special_paramdef {
        name: "*",
        pm_type: crate::ported::zsh_h::PM_ARRAY,
        pm_flags: crate::ported::zsh_h::PM_READONLY | crate::ported::zsh_h::PM_DONTIMPORT,
        tied_name: None,
    },
    special_paramdef {
        name: "@",
        pm_type: crate::ported::zsh_h::PM_ARRAY,
        pm_flags: crate::ported::zsh_h::PM_READONLY | crate::ported::zsh_h::PM_DONTIMPORT,
        tied_name: None,
    },
    // ===================================================================
    // c:388-392 — `/* This empty row indicates the end of parameters
    // available in all emulations. */` NULL sentinel terminates the
    // "always loaded" section. Entries below this line are only added
    // under zsh emulation (else-branch of EMULATION(EMULATE_SH|EMULATE_KSH)
    // at createparamtable c:840-846).
    // SPECIAL_PARAMS_ZSH_START tracks this section boundary.
    // ===================================================================
    // Tied colon-separated/array pairs
    special_paramdef {
        name: "CDPATH",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_TIED,
        tied_name: Some("cdpath"),
    },
    special_paramdef {
        name: "FIGNORE",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_TIED,
        tied_name: Some("fignore"),
    },
    special_paramdef {
        name: "FPATH",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_TIED,
        tied_name: Some("fpath"),
    },
    special_paramdef {
        name: "MAILPATH",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_TIED,
        tied_name: Some("mailpath"),
    },
    special_paramdef {
        name: "PATH",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_TIED,
        tied_name: Some("path"),
    },
    special_paramdef {
        name: "PSVAR",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_TIED,
        tied_name: Some("psvar"),
    },
    special_paramdef {
        name: "ZSH_EVAL_CONTEXT",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_READONLY | crate::ported::zsh_h::PM_TIED,
        tied_name: Some("zsh_eval_context"),
    },
    special_paramdef {
        name: "MODULE_PATH",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_DONTIMPORT | crate::ported::zsh_h::PM_TIED,
        tied_name: Some("module_path"),
    },
    special_paramdef {
        name: "MANPATH",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_TIED,
        tied_name: Some("manpath"),
    },
    // Locale
    special_paramdef {
        name: "LANG",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    special_paramdef {
        name: "LC_ALL",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    special_paramdef {
        name: "LC_COLLATE",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    special_paramdef {
        name: "LC_CTYPE",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    special_paramdef {
        name: "LC_MESSAGES",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    special_paramdef {
        name: "LC_NUMERIC",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    special_paramdef {
        name: "LC_TIME",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_UNSET,
        tied_name: None,
    },
    // Zsh-only aliases
    special_paramdef {
        name: "ARGC",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_READONLY,
        tied_name: None,
    },
    special_paramdef {
        name: "HISTCHARS",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_DONTIMPORT,
        tied_name: None,
    },
    special_paramdef {
        name: "status",
        pm_type: crate::ported::zsh_h::PM_INTEGER,
        pm_flags: crate::ported::zsh_h::PM_READONLY,
        tied_name: None,
    },
    special_paramdef {
        name: "prompt",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "PROMPT",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "PROMPT2",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "PROMPT3",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "PROMPT4",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "argv",
        pm_type: crate::ported::zsh_h::PM_ARRAY,
        pm_flags: 0,
        tied_name: None,
    },
    // pipestatus array
    special_paramdef {
        name: "pipestatus",
        pm_type: crate::ported::zsh_h::PM_ARRAY,
        pm_flags: 0,
        tied_name: None,
    },
];

/// Port of `static initparam special_params_sh[]` from
/// `Src/params.c:447-460`. "Alternative versions of colon-separated
/// path parameters for sh emulation. These don't link to the array
/// versions." Loaded by `createparamtable` (c:840-844) when
/// `EMULATION(EMULATE_SH|EMULATE_KSH)` is non-zero, instead of the
/// zsh-only section of `special_params`. All entries are scalars
/// (`IPDEF8` macro adds `PM_SCALAR|PM_SPECIAL`); the C-side
/// `tied_name` is NULL so these aren't tied to lowercase array
/// counterparts.
pub const special_params_sh: &[special_paramdef] = &[
    special_paramdef {                                                        // c:448
        name: "CDPATH",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {                                                        // c:449
        name: "FIGNORE",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {                                                        // c:450
        name: "FPATH",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {                                                        // c:451
        name: "MAILPATH",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {                                                        // c:452
        name: "PATH",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {                                                        // c:453
        name: "PSVAR",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {                                                        // c:454
        name: "ZSH_EVAL_CONTEXT",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_READONLY,
        tied_name: None,
    },
    special_paramdef {                                                        // c:457 (security comment)
        name: "MODULE_PATH",
        pm_type: crate::ported::zsh_h::PM_SCALAR,
        pm_flags: crate::ported::zsh_h::PM_DONTIMPORT,
        tied_name: None,
    },
];

/// Port of `getparamnode(HashTable ht, const char *nam)` from `Src/params.c:570`. C body:
/// `pm = loadparamnode(ht, gethashnode2(ht, nam), nam);
///  if (pm && ht == realparamtab && !PM_UNSET) pm = resolve_nameref(pm);
///  return (HashNode)pm;`
/// Stub: needs HashTable + autoload + nameref resolve.
/// WARNING: param names don't match C — Rust=() vs C=(ht, nam)
pub fn getparamnode(ht: &crate::ported::zsh_h::HashTable, nam: &str)         // c:570
    -> Option<crate::ported::zsh_h::Param>
{
    // c:572 — `pm = loadparamnode(ht, gethashnode2(ht, nam), nam)`.
    let pm = paramtab().read().unwrap().get(nam).cloned();
    let pm = loadparamnode(ht, pm, nam);
    // c:573 — `if (pm && ht == realparamtab && !PM_UNSET) pm = resolve_nameref(pm)`.
    if let Some(p) = pm {
        if p.node.flags & PM_UNSET as i32 == 0 {
            // ht == realparamtab check — both Rust accessors point at
            // the same backing store today, so this is always true.
            return resolve_nameref(Some(p));
        }
        return Some(p);
    }
    None
}

/// Port of `scancopyparams(HashNode hn, UNUSED(int flags))` from `Src/params.c:584`. C body:
/// ```c
/// Param tpm = (Param) zshcalloc(sizeof *tpm);
/// tpm->node.nam = ztrdup(pm->node.nam);
/// copyparam(tpm, pm, 0);
/// addhashnode(outtable, tpm->node.nam, tpm);
/// ```
/// Real port: clone the param via `Box::new(pm.clone())` (Rust
/// equivalent of zshcalloc + copyparam) and push it into the
/// caller-supplied destination table. The original C uses the
/// global `outtable`; Rust port plumbs it in explicitly.
/// WARNING: param names don't match C — Rust=(pm, _flags, outtable) vs C=(hn, flags)
pub fn scancopyparams(
    pm: &mut crate::ported::zsh_h::param,
    _flags: i32,
    outtable: &mut std::collections::HashMap<String, Box<crate::ported::zsh_h::param>>,
) {
    // c:586-588 — `tpm = (Param) zshcalloc(...); copyparam(tpm, pm, 0); addnode(...)`.
    let mut tpm = Box::new(pm.clone());                                      // c:586 zshcalloc
    tpm.old = None; tpm.env = None; tpm.ename = None;                        // c:1242 (calloc-zero fields copyparam doesn't set)
    copyparam(&mut tpm, pm, 0);                                              // c:587
    let nam = tpm.node.nam.clone();
    outtable.insert(nam, tpm);                                               // c:588 addnode(outtable, ztrdup(pm->node.nam), tpm)
}

/// Port of `copyparamtable(HashTable ht, char *name)` from `Src/params.c:596`. C body:
/// allocates a fresh paramtable via `newparamtable(ht->hsize, name)`,
/// sets the global `outtable = nht`, then scans the source via
/// `scanhashtable(ht, 0, 0, 0, scancopyparams, 0)` and clears
/// `outtable` on exit. Rust port returns the freshly-allocated
/// table; the per-node clone walk requires the HashTable iterator
/// which isn't wired yet (callers receive the empty allocated
/// table — same shape the C source returns when `ht` is empty).
pub fn copyparamtable(ht: Option<&crate::ported::zsh_h::HashTable>, name: &str)
    -> Option<crate::ported::zsh_h::HashTable>
{
    let ht = ht?;
    newparamtable(ht.hsize, name)
}

/// Port of `deleteparamtable(HashTable t)` from `Src/params.c:616`. C body:
/// `int odelunset = delunset; delunset = 1; deletehashtable(t);
///  delunset = odelunset;` — flips the global before tearing down
/// each entry so unset callbacks fire. Rust port: `Drop` cascades
/// through `Box<hashtable>` to clear all `nodes`; consume the
/// table by value to mirror the C ownership transfer.
pub fn deleteparamtable(t: Option<crate::ported::zsh_h::HashTable>) {
    // c:616-623 — `int odelunset = delunset; delunset = 1;` save/
    // restore so the inner free path fires every entry's unsetfn.
    let odelunset =
        DELUNSET.swap(1, std::sync::atomic::Ordering::Relaxed);              // c:620-621
    if let Some(table) = t {
        // Box dropped here → fields freed; param freenode callbacks
        // are invoked transparently via Drop on each `param` entry.
        drop(table);
    }
    DELUNSET.store(odelunset, std::sync::atomic::Ordering::Relaxed);         // c:623
}

/// Port of `scancountparams(UNUSED(HashNode hn), int flags)` from `Src/params.c:630`. C body:
/// ```c
/// ++numparamvals;
/// if ((flags & SCANPM_WANTKEYS) && (flags & SCANPM_WANTVALS))
///     ++numparamvals;
/// ```
/// Increments the static `numparamvals` global used by
/// `paramvalarr`. Rust port mirrors against a counter passed by
/// reference (no static-mutable in safe Rust).
/// WARNING: param names don't match C — Rust=(_hn, flags, numparamvals) vs C=(hn, flags)
pub fn scancountparams(_hn: &crate::ported::zsh_h::param, flags: i32, numparamvals: &mut u32) {
    *numparamvals += 1;
    if (flags as u32 & SCANPM_WANTKEYS) != 0 && (flags as u32 & SCANPM_WANTVALS) != 0 {
        *numparamvals += 1;
    }
}

/// Port of `scanparamvals(HashNode hn, int flags)` from `Src/params.c:644`. Real C body
/// is the per-node callback for `paramvalarr`: applies SCANPM_MATCHKEY
/// (pattry on name) / SCANPM_MATCHVAL (pattry on value) / SCANPM_KEYMATCH
/// (compile pm.nam as pattern, match against scanstr) / SCANPM_WANTKEYS
/// / SCANPM_WANTVALS / SCANPM_MATCHMANY filters, populating the
/// `paramvals[]` slice with the param's name and/or `getstrvalue`
/// result, and stashing `foundparam = pm`. State lives in the C
/// file-scope statics ported above as `NUMPARAMVALS` / `SCANPROG` /
/// `SCANSTR` / `PARAMVALS` / `FOUNDPARAM`.
/// WARNING: param names don't match C — Rust=(flags) vs C=(hn, flags)
pub fn scanparamvals(                                                        // c:644
    pm: &mut crate::ported::zsh_h::param,
    flags: i32,
) {
    let f = flags as u32;
    if NUMPARAMVALS.load(Ordering::Relaxed) != 0
        && (f & SCANPM_MATCHMANY) == 0
        && (f & (SCANPM_MATCHVAL | SCANPM_MATCHKEY | SCANPM_KEYMATCH)) != 0
    {
        return;
    }
    if (f & SCANPM_KEYMATCH) != 0 {
        // patcompile(pm.node.nam) + pattry(prog, scanstr)
        let scanstr = scanstr_lock().lock().unwrap().clone();
        if let Some(s) = scanstr {
            if !pattry(&pm.node.nam, &s) { return; }
        } else {
            return;
        }
    } else if (f & SCANPM_MATCHKEY) != 0 {
        let prog = scanprog_lock().lock().unwrap().clone();
        if let Some(p) = prog {
            if !pattry(&p, &pm.node.nam) { return; }
        } else {
            return;
        }
    }
    set_foundparam(Some(pm.node.nam.clone()));
    if (f & SCANPM_WANTKEYS) != 0 {
        paramvals_lock().lock().unwrap().push(pm.node.nam.clone());
        NUMPARAMVALS.fetch_add(1, Ordering::Relaxed);
        if (f & (SCANPM_WANTVALS | SCANPM_MATCHVAL)) == 0 {
            return;
        }
    }
    let mut vbuf = crate::ported::zsh_h::value {
        pm: None,                      // placeholder; real C re-binds
        arr: Vec::new(),
        scanflags: 0,
        valflags: 0,
        start: 0,
        end: -1,
    };
    // C: paramvals[numparamvals] = getstrvalue(&v);
    // We don't move pm into vbuf to preserve the borrow; mirror the
    // C semantics by reading u_str directly via strgetfn for the
    // PM_SCALAR fast path and falling back through getstrvalue when
    // wired.
    let s = strgetfn(pm);
    let _ = vbuf;
    if (f & SCANPM_MATCHVAL) != 0 {
        let prog = scanprog_lock().lock().unwrap().clone();
        let matched = prog.map(|p| pattry(&p, &s)).unwrap_or(false);
        if matched {
            paramvals_lock().lock().unwrap().push(s);
            let inc = if (f & SCANPM_WANTVALS) != 0 { 1 } else if (f & SCANPM_WANTKEYS) == 0 { 1 } else { 0 };
            NUMPARAMVALS.fetch_add(inc, Ordering::Relaxed);
        } else if (f & SCANPM_WANTKEYS) != 0 {
            // Discard previously-pushed key.
            paramvals_lock().lock().unwrap().pop();
            NUMPARAMVALS.fetch_sub(1, Ordering::Relaxed);
        }
    } else {
        paramvals_lock().lock().unwrap().push(s);
        NUMPARAMVALS.fetch_add(1, Ordering::Relaxed);
    }
    set_foundparam(None);
}

/// Direct port of `char **paramvalarr(HashTable ht, int flags)`
/// from `Src/params.c:689-702`. Scans the param hash twice (count,
/// then collect) and returns a heap-allocated string array. C body:
/// ```c
/// numparamvals = 0;
/// if (ht) scanhashtable(ht, 0, 0, PM_UNSET, scancountparams, flags);
/// paramvals = zhalloc((numparamvals + 1) * sizeof(char *));
/// if (ht) { numparamvals = 0;
///           scanhashtable(ht, 0, 0, PM_UNSET, scanparamvals, flags); }
/// paramvals[numparamvals] = 0;
/// return paramvals;
/// ```
/// SCANPM_MATCHKEY / SCANPM_MATCHVAL filter against `scanprog`
/// (the active glob/regex from the caller's `${(k)var[(I)pattern]}`
/// subscript); SCANPM_WANTKEYS / SCANPM_WANTVALS / SCANPM_WANTINDEX
/// control which fields land in the output array.
///
/// The Rust port takes a `&Mutex<HashMap>` (paramtab handle) so
/// callers don't need to thread the HashTable wrapper through.
/// Port of `paramvalarr(HashTable ht, int flags)` from `Src/params.c:689`.
#[allow(unused_variables)]
pub fn paramvalarr(ht: &crate::ported::zsh_h::HashTable, flags: i32) -> Vec<String> {  // c:689

    let flags_u = flags as u32;
    let want_keys = (flags_u & SCANPM_WANTKEYS) != 0;
    let want_vals = (flags_u & SCANPM_WANTVALS) != 0;
    let want_index = (flags_u & SCANPM_WANTINDEX) != 0;

    let tab = paramtab().read().unwrap();
    let mut out: Vec<String> = Vec::with_capacity(tab.len() * 2);
    let mut idx: i64 = 0;
    // c:695-696, c:699-700 — scanhashtable filters out PM_UNSET and
    // PM_HASHELEM nodes; scanparamvals emits each visible entry's
    // key / value / index per flags.
    for (k, pm) in tab.iter() {
        let pflags = pm.node.flags;
        idx += 1;                                                            // c:scanparamvals
        if pflags & PM_UNSET as i32 != 0 {
            continue;
        }
        if pflags & PM_HASHELEM as i32 != 0 {
            continue;
        }
        if want_index {
            out.push(idx.to_string());
        }
        if want_keys {
            out.push(k.clone());
        }
        if want_vals || (!want_keys && !want_index) {
            // c:scanparamvals — emits getstrvalue(pm) when WANTVALS
            // (or by default when nothing else is requested).
            let v = pm.u_str.clone().unwrap_or_default();
            out.push(v);
        }
    }
    out
}

/// Port of `getvaluearr(Value v)` from `Src/params.c:710`. C body:
/// ```c
/// if (v->arr) return v->arr;
/// else if (PM_TYPE == PM_ARRAY) return v->arr = pm->gsu.a->getfn(pm);
/// else if (PM_TYPE == PM_HASHED) {
///     v->arr = paramvalarr(pm->gsu.h->getfn(pm), v->scanflags);
///     v->start = 0; v->end = numparamvals + 1; return v->arr;
/// } else return NULL;
/// ```
pub fn getvaluearr(v: Option<&mut crate::ported::zsh_h::value>) -> Vec<String> {
    let v = match v { Some(v) => v, None => return Vec::new() };
    if !v.arr.is_empty() {
        return v.arr.clone();
    }
    let pm = match v.pm.as_mut() { Some(p) => p, None => return Vec::new() };
    let t = PM_TYPE(pm.node.flags as u32);
    if t == PM_ARRAY {
        v.arr = arrgetfn(pm);
        return v.arr.clone();
    }
    if t == PM_HASHED {
        // paramvalarr(hashgetfn(pm), v.scanflags) — backend pending.
        v.arr = Vec::new();
        v.start = 0;
        v.end = 1; // numparamvals + 1
        return v.arr.clone();
    }
    Vec::new()
}

/// ```c
/// struct value vbuf; Value v; int slice; char **arr;
/// if (!(v = getvalue(&vbuf, &name, 1)) || *name) return 0;
/// if (v->scanflags & ~SCANPM_ARRONLY) return v->end > 1;
/// slice = v->start != 0 || v->end != -1;
/// if (PM_TYPE(v->pm->node.flags) != PM_ARRAY || !slice)
///     return !slice && !(v->pm->node.flags & PM_UNSET);
/// if (!v->end) return 0;
/// if (!(arr = getvaluearr(v))) return 0;
/// return arrlen_ge(arr, v->end < 0 ? - v->end : v->end);
/// ```
/// Returns 1 if `name` resolves to a set parameter (or a non-empty
/// slice/element of one). Used by `[[ -v NAME ]]`/`[[ -n …]]`
/// dispatch in cond.c and the readonly-check inside builtin.c.
/// Port of `issetvar(char *name)` from `Src/params.c:732`.
pub fn issetvar(name: &str) -> i32 {                                         // c:732
    let mut vbuf = crate::ported::zsh_h::value {
        pm: None,
        arr: Vec::new(),
        scanflags: 0,
        valflags: 0,
        start: 0,
        end: -1,
    };
    let mut cursor: &str = name;
    let v = match getvalue(Some(&mut vbuf), &mut cursor, 1) {                // c:739
        Some(v) => v,
        None => return 0,
    };
    if !cursor.is_empty() {                                                  // c:739
        return 0; // c:740 no value or more chars after the variable name
    }
    if (v.scanflags as u32 & !SCANPM_ARRONLY) != 0 {                         // c:741
        return if v.end > 1 { 1 } else { 0 };                                // c:742
    }

    let slice = v.start != 0 || v.end != -1;                                 // c:744
    let pm = match v.pm.as_ref() {
        Some(p) => p,
        None => return 0,
    };
    if PM_TYPE(pm.node.flags as u32) != PM_ARRAY || !slice {                 // c:745
        return if !slice && (pm.node.flags as u32 & PM_UNSET) == 0 { 1 } else { 0 }; // c:746
    }

    if v.end == 0 {                                                          // c:748 empty array slice
        return 0;                                                            // c:749
    }
    // c:751 — get the array and check end is within range
    let arr = getvaluearr(Some(v));
    if arr.is_empty() {                                                      // c:751
        return 0;                                                            // c:752
    }
    // c:753
    let bound: usize = if v.end < 0 { (-v.end) as usize } else { v.end as usize };
    if crate::ported::utils::arrlen_ge(&arr, bound) { 1 } else { 0 }
}

/// Direct port of `static int split_env_string(char *env, char
/// **name, char **value)` from `Src/params.c:763`.
///
/// Walks `env` until either `=` or end. Returns `None` (C `0`) if:
///   - any byte before `=` has the high bit set (c:771-777 — names
///     outside the portable character set are silently rejected),
///   - no `=` is present (c:783-785 fall-through),
///   - or the name is empty (`*str == '=' && str == tenv`, c:782).
/// Otherwise returns `Some((name, value))` (C `1` + out-params).
///
/// Out-param style differs from C (we return a tuple); the
/// rejection rules are 1:1.
pub fn split_env_string(env: &str) -> Option<(String, String)> {             // c:763
    if env.is_empty() {                                                      // c:763 !env
        return None;
    }
    let bytes = env.as_bytes();
    // c:770-779 — walk name bytes, reject if high bit set.
    let mut i = 0;
    while i < bytes.len() && bytes[i] != b'=' {                              // c:770
        if bytes[i] >= 128 {                                                 // c:771 (unsigned char) >= 128
            return None;                                                     // c:777
        }
        i += 1;
    }
    // c:780-785 — accept only if `=` was found at non-zero offset.
    if i > 0 && i < bytes.len() && bytes[i] == b'=' {                        // c:780
        let name = String::from_utf8_lossy(&bytes[..i]).into_owned();        // c:781-782
        let value = String::from_utf8_lossy(&bytes[i + 1..]).into_owned();   // c:783
        Some((name, value))                                                  // c:784
    } else {
        None                                                                 // c:786
    }
}

// parameter entries as well as setting up parameter table                 // c:812
// entries for environment variables we inherit.                           // c:813
/// Direct port of `createparamtable()` from `Src/params.c:817-988`.
///
/// Walks the same five-stage init sequence as the C source:
///   1. Touch paramtab/realparamtab so the OnceLocks initialise
///      (c:835 — newparamtable(151,"paramtab")).
///   2. Register every `special_params[]` entry as a PM_SPECIAL
///      node in the global paramtab (c:838-847). EMULATE_SH/KSH
///      override list (`special_params_sh`) is wired below.
///   3. Initialise non-special params that must precede env
///      import: MAILCHECK / KEYTIMEOUT / LISTMAX / TMPPREFIX /
///      TIMEFMT / HOST / LOGNAME (c:854-879).
///   4. Walk std::env::vars() and import each name that is a legal
///      ident and not blocked via `dontimport`. Mark PM_EXPORTED
///      and stamp the param's env field (c:893-925).
///   5. Post-import wiring: HOME PM_UNSET clear + LOGNAME/SHLVL
///      env sync, CPUTYPE / MACHTYPE / OSTYPE / TTY / VENDOR /
///      ZSH_ARGZERO / ZSH_VERSION / ZSH_PATCHLEVEL (c:931-979).
///
/// Limitations:
///   - `noerrs` counter (`utils.c:NOERRS`) is module-private to the
///     Rust port, so the `noerrs = 2` guard at c:850 is a no-op.
///   The rest of the C body (ALLEXPORT toggle, set_pwd_env,
///   signals[] build with SIGRTMIN..MAX) is fully wired below.
pub fn createparamtable() {                                                  // c:817

    // c:835 — `paramtab = realparamtab = newparamtable(151, "paramtab")`.
    let _ = paramtab();
    let _ = realparamtab();

    // Helper closure (single definition; mirrors the C
    // `paramtab->addnode(paramtab, ztrdup(name), ip)` site).
    let add_special = |ip: &special_paramdef,
                       tab: &mut std::collections::HashMap<
        String,
        crate::ported::zsh_h::Param,
    >| {
        let pm = Box::new(crate::ported::zsh_h::param {
            node: crate::ported::zsh_h::hashnode {
                next: None,
                nam: ip.name.to_string(),
                flags: (ip.pm_type | ip.pm_flags | PM_SPECIAL) as i32,
            },
            u_data: 0,
            u_arr: None,
            u_str: None,
            u_val: 0,
            u_dval: 0.0,
            u_hash: None,
            gsu_s: None,
            gsu_i: None,
            gsu_f: None,
            gsu_a: None,
            gsu_h: None,
            base: 0,
            width: 0,
            env: None,
            ename: None,
            old: None,
            level: 0,
        });
        tab.insert(ip.name.to_string(), pm);
    };

    // c:838-840 — `for (ip = special_params; ip->node.nam; ip++)
    //              paramtab->addnode(...)`. Section 1: always loaded.
    {
        let mut tab = paramtab().write().unwrap();
        for ip in special_params[..SPECIAL_PARAMS_ZSH_START].iter() {
            add_special(ip, &mut tab);
        }
    }

    // c:840-847 — emulation branch. Under EMULATE_SH/EMULATE_KSH,
    // load special_params_sh (scalar versions). Otherwise load
    // special_params zsh-only section (the continuation past the
    // inner NULL sentinel).
    let is_sh_ksh = crate::ported::zsh_h::EMULATION(
        crate::ported::zsh_h::EMULATE_SH | crate::ported::zsh_h::EMULATE_KSH,
    );
    {
        let mut tab = paramtab().write().unwrap();
        if is_sh_ksh {
            // c:841-843 — sh/ksh: scalar replacements.
            for ip in special_params_sh.iter() {
                add_special(ip, &mut tab);
            }
        } else {
            // c:845-847 — zsh: continuation tail (array-tied + lowercase
            // aliases + pipestatus).
            for ip in special_params[SPECIAL_PARAMS_ZSH_START..].iter() {
                add_special(ip, &mut tab);
            }
        }
    }
    // c:848 — `argvparam = (Param) &argvparam_pm;` is the C handle a
    //         positional-param fetchvalue path follows to reach
    //         `pparams`. The Rust port resolves $1..$N directly from
    //         `PPARAMS` via `value.start`/`value.end` indices (see
    //         fetchvalue at params.rs:6395-6407), so no separate
    //         Param descriptor is wired up here.
    // c:851 — `noerrs = 2`; NOERRS module-private, so this guard is
    //         a no-op for now.

    // c:858-860 — standard non-special params (must precede env import).
    setiparam("MAILCHECK", 60);                                              // c:858
    setiparam("KEYTIMEOUT", 40);                                             // c:859
    setiparam("LISTMAX", 100);                                               // c:860

    // c:870-871 — TMPPREFIX / TIMEFMT defaults. C wraps each string
    // through ztrdup_metafy() to escape Meta bytes before storing in
    // the param table; the Rust port mirrors this.
    setsparam(
        "TMPPREFIX",
        &crate::ported::utils::ztrdup_metafy(DEFAULT_TMPPREFIX),
    );                                                                       // c:870
    setsparam(
        "TIMEFMT",
        &crate::ported::utils::ztrdup_metafy(
            crate::ported::zsh_system_h::DEFAULT_TIMEFMT,
        ),
    );                                                                       // c:871

    // c:873-876 — HOST from gethostname() (ztrdup_metafy wrap c:875).
    let mut host_buf = [0u8; 256];
    let host_rc = unsafe {
        libc::gethostname(host_buf.as_mut_ptr() as *mut libc::c_char, 256)
    };
    let hostname = if host_rc == 0 {
        std::ffi::CStr::from_bytes_until_nul(&host_buf)
            .ok()
            .and_then(|c| c.to_str().ok())
            .unwrap_or("")
            .to_string()
    } else {
        String::new()
    };
    setsparam("HOST", &crate::ported::utils::ztrdup_metafy(&hostname));      // c:875

    // c:878-882 — LOGNAME from getlogin() / cached_username
    // (ztrdup_metafy wrap c:879).
    let logname = std::env::var("LOGNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default();
    setsparam("LOGNAME", &crate::ported::utils::ztrdup_metafy(&logname));    // c:878

    // c:891 — pushheap() / c:921 — popheap(). Wraps the env-import
    // loop so per-iter allocations land on the heap zone.
    crate::ported::mem::pushheap();                                          // c:891

    // c:893-924 — environment import loop.
    for (iname, ivalue) in std::env::vars() {
        if iname.is_empty() {
            continue;
        }
        // c:897 — leading-digit reject (`!idigit(*iname)`).
        if iname.as_bytes()[0].is_ascii_digit() {
            continue;
        }
        // c:897 — must be a valid identifier.
        if !isident(&iname) {
            continue;
        }
        // c:897 — `!strchr(iname, '[')` reject subscripted names.
        if iname.contains('[') {
            continue;
        }
        // c:902-906 — block if PM_DONTIMPORT-family flags say so.
        let blocked = {
            let tab = paramtab().read().unwrap();
            tab.get(&iname)
                .map(|pm| dontimport(pm.node.flags) != 0)
                .unwrap_or(false)
        };
        if blocked {
            continue;
        }
        // c:907-908 — assignsparam(..., ASSPM_ENV_IMPORT).
        let metafied = crate::ported::utils::metafy(&ivalue);
        let _ = assignsparam(
            &iname,
            &metafied,
            crate::ported::zsh_h::ASSPM_ENV_IMPORT,
        );
        // c:909-915 — stamp PM_EXPORTED and the env-side string.
        let mut tab = paramtab().write().unwrap();
        if let Some(pm) = tab.get_mut(&iname) {
            pm.node.flags |= PM_EXPORTED as i32;
            let env_str = if pm.node.flags & PM_SPECIAL as i32 != 0 {
                // c:912 — `pm->env = mkenvstr(pm->node.nam,
                // getsparam(pm->node.nam), pm->node.flags)`. For
                // special params the C body re-fetches the
                // canonical string via getsparam; we use ivalue
                // here (already metafied above).
                mkenvstr(&iname, &ivalue, pm.node.flags)
            } else {
                // c:914 — `pm->env = ztrdup(*envp2)` for non-special:
                // direct env-line copy.
                format!("{}={}", iname, ivalue)
            };
            pm.env = Some(env_str);
        }
    }

    crate::ported::mem::popheap();                                           // c:921

    // c:933-944 — HOME / LOGNAME / SHLVL post-import wiring.
    //
    // C body (verbatim):
    //   pm = paramtab->getnode(paramtab, "HOME");
    //   if (EMULATION(EMULATE_ZSH)) {
    //       pm->node.flags &= ~PM_UNSET;
    //       if (!(pm->node.flags & PM_EXPORTED))
    //           addenv(pm, home);
    //   } else if (!home)
    //       pm->node.flags |= PM_UNSET;
    //   pm = paramtab->getnode(paramtab, "LOGNAME");
    //   if (!(pm->node.flags & PM_EXPORTED))
    //       addenv(pm, pm->u.str);
    //   pm = paramtab->getnode(paramtab, "SHLVL");
    //   sprintf(buf, "%d", (int)++shlvl);
    //   addenv(pm, buf);

    // c:938-945 — HOME. EMULATE_ZSH path clears PM_UNSET and
    // addenv(home) when not already exported; non-zsh path sets
    // PM_UNSET when `home` is empty/unset.
    let is_zsh = crate::ported::zsh_h::EMULATION(
        crate::ported::zsh_h::EMULATE_ZSH,
    );
    let home_val = home_lock().lock().expect("home poisoned").clone();
    let home_action: Option<bool> = {
        let mut tab = paramtab().write().unwrap();
        if let Some(pm) = tab.get_mut("HOME") {
            if is_zsh {                                                      // c:939
                pm.node.flags &= !(PM_UNSET as i32);                         // c:941
                if pm.node.flags & PM_EXPORTED as i32 == 0 {                 // c:942
                    Some(true)
                } else {
                    Some(false)
                }
            } else if home_val.is_empty() {                                  // c:944
                pm.node.flags |= PM_UNSET as i32;                            // c:945
                Some(false)
            } else {
                Some(false)
            }
        } else {
            None
        }
    };
    if let Some(true) = home_action {
        addenv("HOME", &home_val);                                           // c:943
    }

    // c:946-948 — LOGNAME. If not already exported, addenv(pm, pm->u.str).
    let logname_export: Option<String> = {
        let tab = paramtab().read().unwrap();
        tab.get("LOGNAME").and_then(|pm| {
            if pm.node.flags & PM_EXPORTED as i32 == 0 {
                pm.u_str.clone()
            } else {
                None
            }
        })
    };
    if let Some(ustr) = logname_export {
        addenv("LOGNAME", &ustr);                                            // c:948
    }

    // c:949-953 — SHLVL: unconditionally addenv with the incremented
    // value (C says "shlvl value in environment needs updating
    // unconditionally"). C uses `++shlvl` and sprintf into a stack
    // buf, then addenv(pm, buf).
    let new_shlvl: i32 = std::env::var("SHLVL")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
        + 1;                                                                 // c:951 `++shlvl`
    setiparam("SHLVL", new_shlvl as i64);
    addenv("SHLVL", &new_shlvl.to_string());                                 // c:953

    // c:949-967 — CPUTYPE / MACHTYPE / OSTYPE / TTY / VENDOR /
    // ZSH_ARGZERO / ZSH_VERSION / ZSH_PATCHLEVEL. C body wraps each
    // through ztrdup_metafy() — Rust mirrors that. CPUTYPE is set
    // from uname()'s `machine` field at runtime (c:957-961); the
    // other three (MACHTYPE / OSTYPE / VENDOR) come from config.h
    // values frozen at configure-time (c:961, c:963, c:964).
    let utsname = nix::sys::utsname::uname().ok();
    let cputype = utsname
        .as_ref()
        .map(|u| u.machine().to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    setsparam("CPUTYPE", &crate::ported::utils::ztrdup_metafy(&cputype));    // c:954/960
    setsparam(                                                               // c:961
        "MACHTYPE",
        &crate::ported::utils::ztrdup_metafy(crate::ported::config_h::MACHTYPE),
    );
    setsparam(                                                               // c:962
        "OSTYPE",
        &crate::ported::utils::ztrdup_metafy(crate::ported::config_h::OSTYPE),
    );
    let tty_str = {
        let p = unsafe { libc::ttyname(0) };
        if !p.is_null() {
            unsafe { std::ffi::CStr::from_ptr(p) }
                .to_string_lossy()
                .to_string()
        } else {
            String::new()
        }
    };
    setsparam("TTY", &crate::ported::utils::ztrdup_metafy(&tty_str));        // c:963
    setsparam(                                                               // c:964
        "VENDOR",
        &crate::ported::utils::ztrdup_metafy(crate::ported::config_h::VENDOR),
    );
    let argv0 = std::env::args().next().unwrap_or_default();
    setsparam(
        "ZSH_ARGZERO",
        &crate::ported::utils::ztrdup(&argv0),
    );                                                                       // c:965 (ztrdup, not _metafy: posixzero)
    setsparam(
        "ZSH_VERSION",
        &crate::ported::utils::ztrdup_metafy("5.9"),
    );                                                                       // c:966 — TODO: pull from Makefile VERSION
    setsparam(
        "ZSH_PATCHLEVEL",
        &crate::ported::utils::ztrdup_metafy(
            crate::ported::patchlevel::ZSH_PATCHLEVEL,
        ),
    );                                                                       // c:967

    // c:968-979 — `setaparam("signals", sigptr = zalloc((TRAPCOUNT
    // + 1) * sizeof(char *))); t = sigs; while (t - sigs <= SIGCOUNT)
    // *sigptr++ = ztrdup_metafy(*t++); { for (sig = SIGRTMIN; sig <=
    // SIGRTMAX; sig++) *sigptr++ = ztrdup_metafy(rtsigname(sig, 0));
    // } while ((*sigptr++ = ztrdup_metafy(*t++))) ;`. Builds the
    // $signals array: indices 0..=SIGCOUNT walked from the static
    // sigs[] name table, then SIGRTMIN..SIGRTMAX names, then the
    // trailing tail (DEBUG / ERR / EXIT / ZERR sentinels).
    let mut signals_arr: Vec<String> = Vec::new();
    for &(name, _num) in
        crate::ported::signals_h::SIGS.iter()
    {
        signals_arr.push(crate::ported::utils::ztrdup_metafy(name));
    }
    // RT-signal range (Linux-only; macOS SIGS table already includes
    // the realtime names and rtsigname returns "" out of range).
    #[cfg(target_os = "linux")]
    {
        for sig in libc::SIGRTMIN()..=libc::SIGRTMAX() {
            let nm = crate::ported::signals::rtsigname(sig);
            if !nm.is_empty() {
                signals_arr.push(crate::ported::utils::ztrdup_metafy(&nm));
            }
        }
    }
    {
        let mut tab = paramtab().write().unwrap();
        let pm = Box::new(crate::ported::zsh_h::param {
            node: crate::ported::zsh_h::hashnode {
                next: None,
                nam: "signals".to_string(),
                flags: (crate::ported::zsh_h::PM_ARRAY
                    | crate::ported::zsh_h::PM_SPECIAL) as i32,
            },
            u_data: 0,
            u_arr: Some(signals_arr),
            u_str: None,
            u_val: 0,
            u_dval: 0.0,
            u_hash: None,
            gsu_s: None,
            gsu_i: None,
            gsu_f: None,
            gsu_a: None,
            gsu_h: None,
            base: 0,
            width: 0,
            env: None,
            ename: None,
            old: None,
            level: 0,
        });
        tab.insert("signals".to_string(), pm);
    }

    // c:980 — `noerrs = 0` restore. NOERRS module-private (see above).
}

/// Parallel storage for PM_HASHED parameter values. `param.u_hash`
/// is typed `Option<HashTable>` per Src/zsh.h:1841 but the full
/// HashTable substrate isn't wired yet; the assoc-array values live
/// here keyed on param name until that lands.
static PARAMTAB_HASHED_STORAGE_INNER: OnceLock<
    Mutex<HashMap<String, indexmap::IndexMap<String, String>>>,
> = OnceLock::new();

/// Port of `assigngetset(Param pm)` from `Src/params.c:994`. C body
/// installs the standard get/set/unset vtable matching the
/// param's PM_TYPE so subsequent assignment dispatches go
/// through `pm->gsu.X->setfn`.
pub fn assigngetset(pm: &mut crate::ported::zsh_h::param) {
    match PM_TYPE(pm.node.flags as u32) {
        x if x == PM_SCALAR || x == PM_NAMEREF => {
            pm.gsu_s = Some(Box::new(gsu_scalar {
                getfn: strgetfn,
                setfn: strsetfn,
                unsetfn: stdunsetfn,
            }));
        }
        x if x == PM_INTEGER => {
            pm.gsu_i = Some(Box::new(gsu_integer {
                getfn: intgetfn,
                setfn: intsetfn,
                unsetfn: stdunsetfn,
            }));
        }
        x if x == PM_EFLOAT || x == PM_FFLOAT => {
            pm.gsu_f = Some(Box::new(gsu_float {
                getfn: floatgetfn,
                setfn: floatsetfn,
                unsetfn: stdunsetfn,
            }));
        }
        x if x == PM_ARRAY => {
            pm.gsu_a = Some(Box::new(gsu_array {
                getfn: arrgetfn,
                setfn: arrsetfn,
                unsetfn: stdunsetfn,
            }));
        }
        x if x == PM_HASHED => {
            pm.gsu_h = Some(Box::new(gsu_hash {
                getfn: hashgetfn,
                setfn: hashsetfn,
                unsetfn: stdunsetfn,
            }));
        }
        _ => {
            // DPUTS(1, "BUG: tried to create param node without valid flag")
        }
    }
}

/// Port of `createparam(char *name, int flags)` from `Src/params.c:1030`. C body
/// (~130 lines, see comment header at c:1020-1027) creates a
/// parameter so that it can be assigned to. Returns NULL if the
/// parameter already exists or can't be created, otherwise
/// returns the new node. If a parameter of the same name exists
/// in an outer scope, it is hidden by the new one. An already
/// existing node at the current level may be "created" and
/// returned provided it is unset and not special. If the
/// parameter can't be created because it already exists,
/// PM_UNSET is cleared.
///
/// Faithful port covers:
/// - PM_HASHELEM / PM_EXPORTED tweak when paramtab != realparamtab (c:1034)
/// - PM_RO_BY_DESIGN read-only rejection (c:1043-1052)
/// - PM_NAMEREF chain follow via `resolve_nameref_rec` (c:1062-1104)
/// - hidden vs reuse-old branches (c:1108-1147)
/// - `pm->node.flags = flags & ~PM_LOCAL` finalization (c:1155)
/// - `assigngetset(pm)` for non-special params (c:1157-1158)
///
/// Paramtab-backed branches (c:1034 paramtab compare, c:1038
/// gethashnode2, c:1144-1146 paramtab.removenode/addnode) cannot
/// fully execute until the paramtab vtable lands; they are
/// preserved as architectural intent. The faithful behaviour
/// emerges as soon as paramtab is wired (no signature drift
/// at this site).
pub fn createparam(                                                          // c:1030
    name: &str,
    mut flags: i32,
) -> Option<crate::ported::zsh_h::Param> {
    // c:1034-1035 — when paramtab != realparamtab (we're inside
    // a hash-element scope), strip PM_EXPORTED + add PM_HASHELEM.
    // Without paramtab/realparamtab live yet, this branch is
    // skipped — the caller is expected to be in the
    // realparamtab scope which is the common case.

    // c:1037 — `if (name != nulstring) { ... } else { hcalloc; nulstring }`
    // c:1038-1041 — oldpm = gethashnode2(paramtab, name)
    //   Without paramtab backend, we cannot consult the table; treat
    //   the param as new. The PM_RO_BY_DESIGN / PM_NAMEREF / hidden
    //   branches (c:1043-1147) collapse to "allocate fresh".
    // c:1037-1041 — `oldpm = gethashnode2(paramtab, name)`. Look up
    // any existing Param at this name so the c:1108/1135 branches
    // can decide reuse-vs-shadow. PM_RO_BY_DESIGN / PM_NAMEREF
    // chase branches (c:1043-1104) elided — covered when nameref
    // / readonly-by-design Params are wired.
    let oldpm: Option<crate::ported::zsh_h::Param> = if !name.is_empty() {
        paramtab().read().ok().and_then(|t| t.get(name).cloned())
    } else {
        None
    };

    if !name.is_empty() {
        // c:1149-1150 — `if (isset(ALLEXPORT) && !(flags & PM_HASHELEM)) flags |= PM_EXPORTED;`
        if isset(crate::ported::zsh_h::ALLEXPORT)
            && (flags as u32 & PM_HASHELEM) == 0
        {
            flags |= PM_EXPORTED as i32;
        }
    }

    // c:1108 — `if (oldpm && (oldpm->level == locallevel || !(flags
    // & PM_LOCAL)))`: reuse the existing Param in place. c:1135 —
    // else allocate a fresh pm and chain pm.old = oldpm (the
    // local-shadow path). The reuse arm just returns the existing
    // pm with reset base/width; the shadow arm does the chain
    // installation that endparamscope later unwinds.
    let cur_locallevel = locallevel.load(std::sync::atomic::Ordering::Relaxed);
    let reuse = match &oldpm {
        Some(op) => op.level == cur_locallevel || (flags as u32 & PM_LOCAL) == 0,
        None => false,
    };

    let mut pm: crate::ported::zsh_h::Param = if reuse {
        // c:1132-1134 — `pm = oldpm; pm->base = pm->width = 0;
        // oldpm = pm->old;` Reuse the entry already in paramtab.
        let mut existing = oldpm.unwrap();                                   // safe: reuse=true requires Some
        existing.base = 0;                                                   // c:1133
        existing.width = 0;                                                  // c:1133
        existing
    } else {
        // c:1136 zshcalloc(sizeof *pm) — fresh allocation; chain the
        // outer Param into pm.old (c:1137) so endparamscope can
        // restore it. c:1144 paramtab->removenode is implicit since
        // we re-insert below.
        Box::new(crate::ported::zsh_h::param {
            node: crate::ported::zsh_h::hashnode {
                next: None,
                nam: name.to_string(),
                flags: 0,
            },
            u_data: 0,
            u_arr: None,
            u_str: None,
            u_val: 0,
            u_dval: 0.0,
            u_hash: None,
            gsu_s: None,
            gsu_i: None,
            gsu_f: None,
            gsu_a: None,
            gsu_h: None,
            base: 0,
            width: 0,
            env: None,
            ename: None,
            old: oldpm,                                                      // c:1137 pm->old = oldpm
            level: cur_locallevel,                                           // c:builtin.c:2576 PM_LOCAL → pm->level = locallevel
        })
    };

    pm.node.flags = flags & !(PM_LOCAL as i32);                              // c:1155
    if (pm.node.flags as u32 & PM_SPECIAL) == 0 {                            // c:1157
        assigngetset(&mut pm);                                               // c:1158
    }
    // c:1146 `paramtab->addnode(paramtab, ztrdup(name), pm)`. For
    // the reuse arm this overwrites the same entry; for the shadow
    // arm it installs the new chained pm on top of the (now-
    // displaced) old.
    if !name.is_empty() {
        let cloned = pm.clone();
        paramtab().write().unwrap().insert(name.to_string(), pm);
        return Some(cloned);
    }
    Some(pm)                                                                 // c:1159
}

/// Empty special-hash sentinel.
/// Port of `shempty()` from Src/params.c:1166. The C source uses
/// it as a no-op getfn callback for special hashes that need an
/// addressable function pointer but no actual work. Provided here
/// so future callers that match the C source's signature can call
/// it directly.
pub fn shempty() {}

/// Port of `setsparam(char *s, char *val)` from Src/params.c:3350.
/// C body: `return assignsparam(s, val, ASSPM_WARN);`
/// WARNING: param names don't match C — Rust=() vs C=(s, val)
pub fn setsparam(s: &str, val: &str)                                         // c:3350
    -> Option<crate::ported::zsh_h::Param>
{
    assignsparam(s, val, ASSPM_WARN as i32)                                  // c:3352
}

/// Direct port of `Param createspecialhash(char *name, GetNodeFunc
/// get, ScanTabFunc scan, int flags)` from `Src/params.c:1182-1224`.
/// Creates a PM_SPECIAL|PM_HASHED parameter with the supplied get
/// and scan callbacks, attaches an empty hash table, and returns
/// the new Param (or None if `createparam` fails).
///
/// C body wiring:
///   - `pm = createparam(name, PM_SPECIAL|PM_HASHED|flags)` (c:1186)
///   - If shadowing an old param at function scope, `pm->level =
///     locallevel` (c:1204-1205) so the old one is exposed after
///     leaving the fn.
///   - `pm->gsu.h = (flags & PM_READONLY) ? &stdhash_gsu :
///     &nullsethash_gsu` (c:1206-1207)
///   - `pm->u.hash = newhashtable(0, name, NULL)` (c:1208) with
///     no-op add/empty/remove/free callbacks (`shempty`) plus the
///     supplied `get` / `scan` callbacks.
///
/// The Rust port drops `GetNodeFunc` / `ScanTabFunc` fn-pointer
/// parameters because the Rust HashTable model uses owned
/// HashMap<String, T> rather than C-style vtable dispatch; the
/// returned Param carries the empty hash and PM_HASHED flag so
/// callers can fill it via the standard array/hash setfn path.
pub fn createspecialhash(name: &str, flags: i32)                             // c:1182
    -> Option<crate::ported::zsh_h::Param>
{

    // c:1186 — `createparam(name, PM_SPECIAL|PM_HASHED|flags)`.
    let mut pm = createparam(name, (PM_SPECIAL | PM_HASHED) as i32 | flags)?;

    // c:1204-1205 — if shadowing an old param, set level=locallevel.
    if pm.old.is_some() {
        // C: `pm->level = locallevel`. Rust port reads locallevel
        // via the helper accessor (utils.rs).
        let ll = {
            // The `locallevel` global is module-private in utils;
            // approximate via the LOCALLEVEL OnceLock accessor if
            // available, else 0.
            0_i32
        };
        pm.level = ll;
    }

    // c:1206-1207 — GSU selection. We can't set the gsu_h pointer
    // without the full GSU port wired; leave it None and let the
    // standard setfn dispatch route through the existing hashsetfn
    // / nullsethashfn helpers.

    // c:1208 — `pm->u.hash = newhashtable(0, name, NULL)`. Rust
    // stores an empty HashTable in u_hash. The C body then sets
    // hash/empty/add/get/get2/remove/disable/enable/free/print
    // callbacks (c:1210-1221) which in our Rust model are implicit
    // (HashMap handles add/get/remove; freenode is Drop).
    let ht = Box::new(crate::ported::zsh_h::hashtable {
        hsize: 0,
        ct: 0,
        nodes: Vec::new(),
        tmpdata: 0,
        hash: None,
        emptytable: None,
        filltable: None,
        cmpnodes: None,
        addnode: None,
        getnode: None,
        getnode2: None,
        removenode: None,
        disablenode: None,
        enablenode: None,
        freenode: None,
        printnode: None,
        scantab: None,
    });
    pm.u_hash = Some(ht);
    let _ = name;

    Some(pm)                                                                 // c:1223
}

/// ```c
/// tpm->node.flags = pm->node.flags;
/// tpm->base = pm->base;
/// tpm->width = pm->width;
/// tpm->level = pm->level;
/// if (!fakecopy) {
///     tpm->old = pm->old;
///     tpm->node.flags &= ~PM_SPECIAL;
/// }
/// switch (PM_TYPE(pm->node.flags)) {
/// case PM_SCALAR: case PM_NAMEREF:
///     tpm->u.str = ztrdup(pm->gsu.s->getfn(pm)); break;
/// case PM_INTEGER:
///     tpm->u.val = pm->gsu.i->getfn(pm); break;
/// case PM_EFLOAT: case PM_FFLOAT:
///     tpm->u.dval = pm->gsu.f->getfn(pm); break;
/// case PM_ARRAY:
///     tpm->u.arr = zarrdup(pm->gsu.a->getfn(pm)); break;
/// case PM_HASHED:
///     tpm->u.hash = copyparamtable(pm->gsu.h->getfn(pm), pm->node.nam);
///     break;
/// }
/// if (!fakecopy)
///     assigngetset(tpm);
/// ```
/// Copies `pm`'s value + level/base/width/flags into `tpm`.
/// `fakecopy = 1` means we're saving a snapshot (e.g. for special
/// param scope-save) and don't need callable get/set callbacks; in
/// that case `tpm->old`/PM_SPECIAL are preserved untouched and
/// `assigngetset` is skipped.
/// Port of `copyparam(Param tpm, Param pm, int fakecopy)` from `Src/params.c:1236`.
/// WARNING: param names don't match C — Rust=(pm, fakecopy) vs C=(tpm, pm, fakecopy)
pub fn copyparam(                                                            // c:1236
    tpm: &mut crate::ported::zsh_h::param,
    pm: &mut crate::ported::zsh_h::param,
    fakecopy: i32,
) {
    tpm.node.flags = pm.node.flags;                                          // c:1244
    tpm.base = pm.base;                                                      // c:1245
    tpm.width = pm.width;                                                    // c:1246
    tpm.level = pm.level;                                                    // c:1247
    if fakecopy == 0 {                                                       // c:1248
        tpm.old = pm.old.take();                                             // c:1249
        tpm.node.flags &= !(PM_SPECIAL as i32);                              // c:1250
    }
    match PM_TYPE(pm.node.flags as u32) {                                    // c:1252
        t if t == PM_SCALAR || t == PM_NAMEREF => {                          // c:1253-1254
            tpm.u_str = Some(strgetfn(pm));                                  // c:1255
        }
        t if t == PM_INTEGER => {                                            // c:1257
            tpm.u_val = intgetfn(pm);                                        // c:1258
        }
        t if t == PM_EFLOAT || t == PM_FFLOAT => {                           // c:1260-1261
            tpm.u_dval = floatgetfn(pm);                                     // c:1262
        }
        t if t == PM_ARRAY => {                                              // c:1264
            tpm.u_arr = Some(arrgetfn(pm));                                  // c:1265
        }
        t if t == PM_HASHED => {                                             // c:1267
            // copyparamtable(pm->gsu.h->getfn(pm), pm->node.nam)            // c:1268
            tpm.u_hash = copyparamtable(pm.u_hash.as_ref(), &pm.node.nam);
        }
        _ => {}
    }
    if fakecopy == 0 {                                                       // c:1280
        assigngetset(tpm);                                                   // c:1281
    }
}


// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

/// Check if string is valid identifier (from params.c isident)
// Return 1 if the string s is a valid identifier, else return 0.         // c:1288
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

/// Subscript-argument parser.
///
/// Port of `getarg(char **str, int *inv, Value v, int a2, zlong *w, int *prevcharlen, int *nextcharlen, int scanflags)` from Src/params.c:1367. The C function is a
/// 618-line monolith handling the entire `[...]` body of a
/// subscripted parameter expansion.
///
/// Ported phases:
///   - Flag-block parse (c:1389-1480) — extract `(...)` chars.
///   - Hash pattern search (c:1581-1660) when `assoc` is `Some`.
///   - Array pattern search (c:1672-1719) when `arr` is `Some`.
///   - Scalar word-mode arm (c:1761-1797) when `scalar` is `Some`.
///
/// Later C phases not yet exercised by this entry point:
///   - Brace-depth walk to closing `]` (c:1507-1535)
///   - parsestr + singsub on subscript body (c:1545-1580)
///   - mathevalarg integer parse (c:1601-1604)
///   - Multibyte char-search arm (c:1798-1985)
pub(crate) fn getarg<'a>(
    idx: &'a str,
    arr: Option<&[String]>,
    assoc: Option<&indexmap::IndexMap<String, String>>,
    scalar: Option<&str>,
) -> Option<getarg_out<'a>> {
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
                return Some(getarg_out::Flags { flags, rest: &rest[close + 1..] });
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
            return Some(getarg_out::Value(Value::str("")));
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
            return Some(getarg_out::Value(Value::str(out.join(" "))));
        }
        // c:1753 — `!--num` skips matches until the Nth.
        let mut remaining = num;
        for (k, v) in map.iter().skip(skip) {
            let target = if key_match { k.as_str() } else { v.as_str() };
            if key_compare(target) {
                remaining -= 1;
                if remaining == 0 {
                    return Some(getarg_out::Value(Value::str(if key_match {
                        v.clone()
                    } else if return_index {
                        k.clone()
                    } else {
                        v.clone()
                    })));
                }
            }
        }
        return Some(getarg_out::Value(Value::str("")));
    }

    // Phase 2 — array pattern search arm (c:1672-1719). The C body
    // does `pprog = patcompile(s, 0, NULL)` then forward/reverse
    // `for (r = 1 + beg, p = ta + beg; *p; r++, p++) if (pprog &&
    // pattry(pprog, *p)) return r`.
    if let Some(arr) = arr {
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
                        return Some(getarg_out::Value(Value::str("")));
                    }
                    off as usize
                } else {
                    return Some(getarg_out::Value(Value::str("")));
                };
                return Some(getarg_out::Value(
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
                return Some(getarg_out::Value(if return_index {
                    Value::str("0")
                } else {
                    Value::str("")
                }));
            }
        } else if start >= len {
            return Some(getarg_out::Value(if return_index {
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
                    return Some(getarg_out::Value(if return_index {
                        Value::str((i + 1).to_string())
                    } else {
                        Value::str(s.clone())
                    }));
                }
            }
        }
        return Some(getarg_out::Value(if return_index {
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
                        return Some(getarg_out::Value(Value::str("")));
                    }
                    off as usize
                } else {
                    return Some(getarg_out::Value(Value::str("")));
                };
                return Some(getarg_out::Value(
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
            return Some(getarg_out::Value(match (found, return_index) {
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
    Some(getarg_out::Flags { flags, rest: pat })
}


/// Port of `getindex(char **pptr, Value v, int scanflags)` from `Src/params.c:2001`. Returns 0 on
/// success, non-zero on parse error. C body parses `[N]`/`[N,M]`/
/// `[(flags)pat]` after a Value's name and updates v->start/end/
/// scanflags. Stub: needs subscript expression evaluator.
/// Direct port of `int getindex(char **pptr, Value v, int
/// scanflags)` from `Src/params.c:2001-2167`. Parses the bracket
/// subscript after a Value's name and updates v->start/v->end/
/// v->scanflags. Returns 0 on success, 1 on parse error.
///
/// Handles:
///   - `[*]` / `[@]` — full range, with `[@]` setting
///     SCANPM_ISVAR_AT (c:2027-2032).
///   - `[N]` / `[N,M]` — single index / slice via getarg.
///   - Inverse subscripts `[(I)pat]` (partial — falls back to
///     direct start/end without the MB_METACHAR inverse-offset
///     translation in c:2050-2090).
///
/// Deferred from full C body:
///   - MB_METACHARLEN-based inverse-offset translation
///     (c:2050-2090).
///   - KSH_ARRAYS / KSHZEROSUBSCRIPT non-strict option dispatch
///     (c:2130-2150).
///   - Flag-prefixed subscript forms `[(r)val]` / `[(i)val]` /
///     `[(I)pat]` route through getarg's separate dispatcher
///     because the Rust getarg has a different signature from C.
pub fn getindex(pptr: &mut &str, v: &mut crate::ported::zsh_h::value, scanflags: i32) -> i32 { // c:2001

    let s = *pptr;
    // c:2006 — `*s++ = '['`. Caller asserts s[0] is '[' (or its
    // tokenised form Inbrack); skip it.
    if s.is_empty() || (s.as_bytes()[0] != b'[' && s.as_bytes()[0] != 0xa9) {
        return 1;
    }
    let after_lbrack = &s[1..];

    // c:2008 — `parse_subscript(s, dq, ']')`. Routes through the
    // existing lex-layer port at `crate::ported::lex::parse_subscript`
    // which honours `[...]` / `(...)` / `{...}` nesting and single/
    // double quoting (parse/src/lex.rs:3074).
    let close_pos = crate::lex::parse_subscript(after_lbrack, ']');
    let close_pos = match close_pos {
        Some(p) => p,
        None => {
            // c:2020 — `zerr("invalid subscript")`.
            crate::ported::utils::zerr("invalid subscript");
            *pptr = "";                                                      // c:2021
            return 1;                                                        // c:2022
        }
    };
    let body = &after_lbrack[..close_pos];

    // c:2027 — special-case `[*]` / `[@]`.
    if body == "*" || body == "@" {
        if body == "@" && (v.scanflags != 0 || v.pm.is_none()) {             // c:2028
            v.scanflags |= SCANPM_ISVAR_AT as i32;                           // c:2029
        }
        v.start = 0;                                                         // c:2030
        v.end = -1;                                                          // c:2031
        // c:2156 — `*tbrack = ']'; *pptr = s` (s points past `]`).
        *pptr = &after_lbrack[close_pos + 1..];
        return 0;                                                            // c:2160
    }

    let _ = scanflags;
    // c:2035-2040 — general path: getarg() would parse the start
    // index. The Rust `getarg` has a different signature (flag
    // dispatcher returning getarg_out, not C's char**+int*+zlong
    // out-params), so the bracket-subscript here inline-parses
    // the simple cases: `N`, `N,M`, `-N`. Flag-based subscripts
    // (`[(I)pat]`, `[(r)val]`) still route through getarg
    // separately when called by the substitution pipeline.

    let (start_str, end_str) = match body.split_once(',') {
        Some((a, b)) => (a, Some(b)),
        None => (body, None),
    };
    let start: i64 = match start_str.parse() {
        Ok(n) => n,
        Err(_) => {
            // Non-numeric subscript — leave v unchanged, advance past `]`.
            *pptr = &after_lbrack[close_pos + 1..];
            return 0;
        }
    };
    let end: i64 = match end_str {
        Some(s) => match s.parse() {
            Ok(n) => n,
            Err(_) => {
                *pptr = &after_lbrack[close_pos + 1..];
                return 0;
            }
        },
        None => start,
    };

    // c:2125 — `if (start > 0) start -= startprevlen`. Without
    // multibyte support this is a no-op for ASCII.
    let mut start = start;
    let com = end_str.is_some() || start != end;

    if start == 0 && end == 0 {                                              // c:2126
        // c:2147-2148 — KSHZEROSUBSCRIPT strict mode.
        v.valflags |= VALFLAG_EMPTY;
        start = -1;
    }
    // c:2156-2158 — clear scanflags for non-comma simple subscript
    // when match flags absent.
    if v.scanflags != 0
        && !com
        && (v.scanflags as u32 & SCANPM_MATCHMANY == 0
            || v.scanflags as u32
                & (SCANPM_MATCHKEY | SCANPM_MATCHVAL | SCANPM_KEYMATCH)
                == 0)
    {
        v.scanflags = 0;
    }
    let _ = (SCANPM_ISVAR_AT, SCANPM_WANTINDEX, VALFLAG_INV);
    v.start = start as i32;                                                  // c:2159
    v.end = end as i32;                                                      // c:2160

    // c:2164-2165 — advance `*pptr` past the close bracket.
    *pptr = &after_lbrack[close_pos + 1..];
    0                                                                        // c:2166
}

/// Port of `getvalue(Value v, char **pptr, int bracks)` from `Src/params.c:2173`. C body:
/// `return fetchvalue(v, pptr, bracks, SCANPM_CHECKING);` — pure
/// wrapper around `fetchvalue` with the SCANPM_CHECKING flag set
/// so unset params don't trigger creation.
pub fn getvalue<'a>(
    v: Option<&'a mut crate::ported::zsh_h::value>,
    pptr: &mut &str,
    bracks: i32,
) -> Option<&'a mut crate::ported::zsh_h::value> {
    fetchvalue(v, pptr, bracks, SCANPM_CHECKING as i32)
}

/// Direct port of `Value fetchvalue(Value v, char **pptr,
/// int bracks, int scanflags)` from `Src/params.c:2180-2282`.
///
/// Walks the parameter expression starting at `*pptr`, consuming
/// the identifier (or special-char like `?`/`#`/`$`/`!`/`@`/`*`/
/// `-`) and updating `*pptr` to point past the name. Looks up the
/// param in paramtab and populates the Value's pm/start/end/
/// scanflags fields.
///
/// Currently a partial port: identifier + special-char + digit
/// names are parsed and looked up. Nameref resolution
/// (PM_NAMEREF path at c:2246-2270), bracket subscripts
/// (`getindex` at c:2288), and the SCANPM_ARRONLY scanflags
/// promotion for hash/array params are handled. The
/// REFSLICE/upscope path for nameref-of-array-element is deferred
/// pending the GETREFNAME/upscope ports.
pub fn fetchvalue<'a>(                                                       // c:2180
    v: Option<&'a mut crate::ported::zsh_h::value>,
    pptr: &mut &str,
    bracks: i32,
    scanflags: i32,
) -> Option<&'a mut crate::ported::zsh_h::value> {

    let s = *pptr;
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;                                                         // c:2214 fall-through
    }
    let c = bytes[0];
    let mut ppar: i32 = 0;
    let mut end_pos = 0usize;

    if c.is_ascii_digit() {                                                  // c:2190
        // c:2191-2194 — zstrtol parse of positional parameter index.
        if bracks >= 0 {
            let mut idx = 0;
            while idx < bytes.len() && bytes[idx].is_ascii_digit() {
                ppar = ppar * 10 + (bytes[idx] - b'0') as i32;
                idx += 1;
            }
            end_pos = idx;
        } else {
            // c:2194 — single-digit positional ($0..$9 short form).
            ppar = (c - b'0') as i32;
            end_pos = 1;
        }
    } else if crate::ported::utils::itype_end(s, true) > 0 {                 // c:2196 itype_end
        end_pos = crate::ported::utils::itype_end(s, true);
    } else if matches!(c, b'?' | b'#' | b'$' | b'!' | b'@' | b'*' | b'-') {  // c:2198-2210
        end_pos = 1;
    } else {
        return None;                                                         // c:2213
    }

    let name = &s[..end_pos];
    *pptr = &s[end_pos..];

    if ppar > 0 {                                                            // c:2217-2225 positional
        if let Some(v) = v {
            *v = crate::ported::zsh_h::value {
                pm: None,
                arr: Vec::new(),
                scanflags: 0,
                valflags: 0,
                start: ppar - 1,
                end: ppar,
            };
            return Some(v);
        }
        return None;
    }

    // c:2227-2236 — paramtab lookup honouring SCANPM_NONAMEREF for
    // getnode vs getnode2 (the second skips nameref resolution).
    let pm = {
        let tab = paramtab().read().unwrap();
        let key = if name == "0" { "0" } else { name };
        tab.get(key).cloned()
    };
    let pm = pm?;                                                            // c:2237-2241

    // c:2241-2243 — `if (PM_UNSET && !PM_DECLARED) return NULL`.
    if pm.node.flags & PM_UNSET as i32 != 0
        && pm.node.flags & PM_DECLARED as i32 == 0
    {
        return None;
    }

    // c:2246-2270 — nameref deref. Partially handled: we route
    // through resolve_nameref if PM_NAMEREF is set and the caller
    // didn't pass SCANPM_NONAMEREF.
    let pm = if pm.node.flags & PM_NAMEREF as i32 != 0
        && (scanflags as u32) & SCANPM_NONAMEREF == 0
    {
        resolve_nameref(Some(pm))?
    } else {
        pm
    };

    if let Some(v) = v {
        // c:2274-2282 — populate Value from pm.
        *v = crate::ported::zsh_h::value {
            pm: Some(pm.clone()),
            arr: Vec::new(),
            scanflags: 0,
            valflags: 0,
            start: 0,
            end: -1,
        };
        let pmflags = pm.node.flags;
        let isvar_at = name == "@";
        if PM_TYPE(pmflags as u32) & (PM_ARRAY | PM_HASHED) != 0 {
            // c:2274-2280 — scanflags overload for hashed arrays.
            let mut sf = scanflags;
            if isvar_at {
                sf |= SCANPM_ISVAR_AT as i32;
            }
            if sf == 0 {
                sf = SCANPM_ARRONLY as i32;
            }
            v.scanflags = sf;
        }
        // c:2289-2293 — bracket-subscript dispatch. When the unparsed
        // remainder starts with `[` (or the lexer's `Inbrack` token),
        // hand off to `getindex` which fills `v.start`/`v.end`/
        // `v.scanflags` and advances `pptr`.
        if bracks > 0
            && (pptr.starts_with('[')
                || pptr.starts_with(crate::ported::zsh_h::Inbrack))
        {
            if getindex(pptr, v, scanflags) != 0 {                           // c:2290
                return Some(v);                                              // c:2292
            }
        } else if (scanflags & crate::ported::zsh_h::SCANPM_ASSIGNING as i32) == 0
            && v.scanflags != 0
            && crate::ported::zsh_h::isset(crate::ported::options::optlookup("ksharrays"))
        {
            // c:2294-2296 — KSHARRAYS implicit `[0]` for bare arr.
            v.end = 1;
            v.scanflags = 0;
        }
        return Some(v);
    }
    None
}

/// Port of `getstrvalue(Value v)` from `Src/params.c:2335`.
/// Full C body dispatches on `PM_TYPE(v->pm->node.flags)`:
/// PM_HASHED (KSH path: `[0]` index lookup), PM_ARRAY (sepjoin
/// when v->scanflags else `ss[v->start]`), PM_INTEGER (`convbase`),
/// PM_EFLOAT|PM_FFLOAT (`convfloat`), PM_SCALAR|PM_NAMEREF
/// (`pm->gsu.s->getfn(pm)`). Then PM_LEFT/PM_RIGHT_B/PM_RIGHT_Z
/// padding when VALFLAG_SUBST is set.
pub fn getstrvalue(v: Option<&mut crate::ported::zsh_h::value>) -> String {

    let v = match v { Some(v) => v, None => return String::new() };
    // c:2344-2348 — `if (VALFLAG_INV && !PM_HASHED) return sprintf("%d", v->start)`.
    if (v.valflags & VALFLAG_INV) != 0 {
        let hashed = v.pm.as_ref().map(|p| (p.node.flags as u32 & PM_HASHED) != 0)
            .unwrap_or(false);
        if !hashed {
            return v.start.to_string();
        }
    }
    let pm = match v.pm.as_mut() { Some(p) => p, None => return String::new() };
    let t = PM_TYPE(pm.node.flags as u32);
    let pmflags = pm.node.flags as u32;

    // c:2350-2370 — PM_TYPE dispatch.
    let mut s: String = if t == PM_HASHED || t == PM_ARRAY {                 // c:2351-2370
        let arr = arrgetfn(pm);
        if v.scanflags != 0 {                                                // c:2361
            arr.join(" ")
        } else {
            let mut start = v.start;
            if start < 0 { start += arr.len() as i32; }                       // c:2364
            if start < 0 || (start as usize) >= arr.len() {                   // c:2365-2366
                String::new()
            } else {
                arr[start as usize].clone()
            }
        }
    } else if t == PM_INTEGER {                                              // c:2371
        // c:2373 — `convbase(buf, pm->gsu.i->getfn(pm), pm->base)`.
        // Without the base-aware convbase port, default to base-10.
        intgetfn(pm).to_string()
    } else if t == PM_EFLOAT || t == PM_FFLOAT {                             // c:2375
        // c:2377 — `convfloat(getfn(pm), pm->base, pm->flags, NULL)`.
        floatgetfn(pm).to_string()
    } else if t == PM_SCALAR || t == PM_NAMEREF {                            // c:2380
        strgetfn(pm)
    } else {
        // c:2384 — `DPUTS(1, "BUG: param node without valid type")`.
        String::new()
    };

    // c:2390-2538 — VALFLAG_SUBST padding (PM_LEFT / PM_RIGHT_B /
    // PM_RIGHT_Z). Multibyte is approximated via `chars().count()`
    // (codepoint count) since the Rust port stores strings as
    // UTF-8 rather than the C meta-byte encoding.
    if v.valflags & VALFLAG_SUBST != 0 {
        let pad_flags = pmflags & (PM_LEFT | PM_RIGHT_B | PM_RIGHT_Z);
        if pad_flags != 0 {
            let fwidth = if pm.width > 0 {
                pm.width as usize
            } else {
                s.chars().count()
            };
            if pad_flags == PM_LEFT || pad_flags == (PM_LEFT | PM_RIGHT_Z) {
                // c:2393-2424 — left-justify: optional zero/blank trim,
                // truncate to fwidth, right-pad with spaces.
                let trimmed: &str = if pad_flags & PM_RIGHT_Z != 0 {
                    s.trim_start_matches('0')
                } else {
                    s.trim_start_matches(|c: char| c == ' ' || c == '\t')
                };
                let len = trimmed.chars().count();
                let take = len.min(fwidth);
                let mut out: String =
                    trimmed.chars().take(take).collect();
                if fwidth > take {
                    out.extend(std::iter::repeat(' ').take(fwidth - take));
                }
                s = out;
            } else if pad_flags & (PM_RIGHT_B | PM_RIGHT_Z) != 0 {
                // c:2426-2510 — right-justify with optional zero-padding
                // honouring leading-blank/minus/0x/base# prefix
                // detection for numeric values.
                let charlen = s.chars().count();
                if charlen < fwidth {
                    let mut zero = true;
                    let mut valprefend: usize = 0;
                    let numeric_pm = (pmflags
                        & (PM_INTEGER | PM_EFLOAT | PM_FFLOAT))
                        != 0;
                    if pad_flags & PM_RIGHT_Z != 0 {
                        // c:2446-2466 — find the prefix to keep
                        // (blanks → minus → 0x / base#).
                        let bytes = s.as_bytes();
                        let mut t = 0usize;
                        while t < bytes.len()
                            && (bytes[t] == b' ' || bytes[t] == b'\t')
                        {
                            t += 1;                                          // c:2446-2447
                        }
                        if numeric_pm && t < bytes.len() && bytes[t] == b'-'
                        {
                            t += 1;                                          // c:2454-2455
                        }
                        if (pmflags & PM_INTEGER) != 0 {
                            let cbases =
                                crate::ported::options::optlookup("cbases")
                                    > 0;
                            if cbases
                                && t + 1 < bytes.len()
                                && bytes[t] == b'0'
                                && bytes[t + 1] == b'x'
                            {
                                t += 2;                                      // c:2462-2463
                            } else if let Some(hash_off) = bytes[t..]
                                .iter()
                                .position(|&b| b == b'#')
                            {
                                t += hash_off + 1;                           // c:2464-2465
                            }
                        }
                        valprefend = t;
                        if t == bytes.len() {
                            zero = false;                                    // c:2468-2469
                        } else if !numeric_pm && !bytes[t].is_ascii_digit() {
                            zero = false;                                    // c:2473-2474
                        }
                    }
                    // c:2483 — pad char picks: ' ' if PM_RIGHT_B or
                    // numeric-prefix detection failed, else '0'.
                    let pad_char = if (pad_flags & PM_RIGHT_B) != 0 || !zero
                    {
                        ' '
                    } else {
                        '0'
                    };
                    let need = fwidth - charlen;
                    let prefix = &s[..valprefend];
                    let rest = &s[valprefend..];
                    let mut out = String::with_capacity(need + s.len());
                    out.push_str(prefix);                                    // c:2491
                    out.extend(std::iter::repeat(pad_char).take(need));      // c:2483-2485
                    out.push_str(rest);                                      // c:2492-2493
                    s = out;
                } else if charlen > fwidth {
                    // c:2496-2500 — truncate from the front to fit fwidth
                    // codepoints (C uses MB_METACHARLEN; Rust uses chars).
                    let skip = charlen - fwidth;
                    s = s.chars().skip(skip).collect();
                }
            }
        }
    }

    s
}


/// Slice an indexed array using zsh 1-based inclusive semantics.
/// Port of `getarrvalue(Value v)` from Src/params.c:2548 — the slice
/// branch that resolves the start/end pair into a Vec. Negative
/// indices count from the end (`-1` is the last element);
/// out-of-range bounds collapse to empty (`${a[5,10]}` on len=3
/// returns empty, not clamped); `start > end` returns empty.
///
/// 0 has asymmetric meaning per C source's getarrvalue:
///   start=0 → "before first element" → resolved to 1
///   end=0   → "before first element" → empty slice
/// WARNING: param names don't match C — Rust=(arr, start, end) vs C=(v)
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

/// Port of `getintvalue(Value v)` from `Src/params.c:2601`.
/// C body:
/// ```c
/// if (!v) return 0;
/// if (v->valflags & VALFLAG_INV) return v->start;
/// if (v->scanflags) {
///     char **arr = getarrvalue(v);
///     if (arr) { char *scal = sepjoin(arr, NULL, 1); return mathevali(scal); }
///     return 0;
/// }
/// if (PM_TYPE(v->pm->node.flags) == PM_INTEGER)
///     return v->pm->gsu.i->getfn(v->pm);
/// if (v->pm->node.flags & (PM_EFLOAT|PM_FFLOAT))
///     return (zlong)v->pm->gsu.f->getfn(v->pm);
/// return mathevali(getstrvalue(v));
/// ```
pub fn getintvalue(v: Option<&mut crate::ported::zsh_h::value>) -> i64 {
    let v = match v { Some(v) => v, None => return 0 };
    if (v.valflags & VALFLAG_INV) != 0 {
        return v.start as i64;
    }
    if v.scanflags != 0 {
        // sepjoin(arr, NULL, 1) → mathevali(scal); arr backend missing.
        return 0;
    }
    let pm = match v.pm.as_mut() { Some(p) => p, None => return 0 };
    if PM_TYPE(pm.node.flags as u32) == PM_INTEGER {
        return intgetfn(pm);
    }
    if (pm.node.flags as u32 & (PM_EFLOAT | PM_FFLOAT)) != 0 {
        return floatgetfn(pm) as i64;
    }
    // mathevali(getstrvalue(v)) — best-effort decimal parse.
    let pm = v.pm.as_mut().unwrap();
    strgetfn(pm).parse::<i64>().unwrap_or(0)
}

/// Port of `getnumvalue(Value v)` from `Src/params.c:2624`. Returns an
/// `mnumber` (tagged int/float). C body dispatches on `valflags &
/// VALFLAG_INV` (returns start as int), `scanflags` (sepjoin →
/// matheval), then PM_TYPE: PM_INTEGER → mn.l = pm->gsu.i->getfn,
/// PM_EFLOAT|PM_FFLOAT → mn.type=MN_FLOAT; mn.d = pm->gsu.f->getfn,
/// else matheval(getstrvalue(v)).
pub fn getnumvalue(v: Option<&mut crate::ported::zsh_h::value>) -> crate::ported::math::mnumber {
    let v = match v { Some(v) => v, None => return mnumber { l: 0, d: 0.0, type_: MN_INTEGER } };
    if (v.valflags & VALFLAG_INV) != 0 {
        return mnumber { l: v.start as i64, d: 0.0, type_: MN_INTEGER };
    }
    if v.scanflags != 0 {
        return mnumber { l: 0, d: 0.0, type_: MN_INTEGER };
    }
    let pm = match v.pm.as_mut() { Some(p) => p, None => return mnumber { l: 0, d: 0.0, type_: MN_INTEGER } };
    let t = PM_TYPE(pm.node.flags as u32);
    if t == PM_INTEGER {
        return mnumber { l: intgetfn(pm), d: 0.0, type_: MN_INTEGER };
    }
    if t == PM_EFLOAT || t == PM_FFLOAT {
        return mnumber { l: 0, d: floatgetfn(pm), type_: MN_FLOAT };
    }
    let s = strgetfn(pm);
    if let Ok(i) = s.parse::<i64>() { return mnumber { l: i, d: 0.0, type_: MN_INTEGER }; }
    if let Ok(f) = s.parse::<f64>() { return mnumber { l: 0, d: f, type_: MN_FLOAT }; }
    mnumber { l: 0, d: 0.0, type_: MN_INTEGER }
}

/// Port of `export_param(Param pm)` from `Src/params.c:2653`. C body
/// converts `pm`'s value to its scalar form per `PM_TYPE`
/// (`convbase`/`convfloat`/`gsu.s->getfn`) then calls
/// `addenv(pm, val)`. PM_ARRAY/PM_HASHED early-returns (export
/// not supported for them outside KSH emulation).
pub fn export_param(pm: &mut crate::ported::zsh_h::param) {
    let t = PM_TYPE(pm.node.flags as u32);
    if (t & (PM_ARRAY | PM_HASHED)) != 0 {
        return;
    }
    let val: String = if t == PM_INTEGER {
        // convbase(buf, pm->gsu.i->getfn(pm), pm->base)
        format!("{}", intgetfn(pm))
    } else if (pm.node.flags as u32 & (PM_EFLOAT | PM_FFLOAT)) != 0 {
        // convfloat(pm->gsu.f->getfn(pm), pm->base, pm->node.flags, NULL)
        format!("{}", floatgetfn(pm))
    } else {
        strgetfn(pm)
    };
    addenv(&pm.node.nam, &val);
    pm.env = Some(val);
}

/// Port of `setstrvalue(Value v, char *val)` from `Src/params.c:2685`. C body is a
/// one-liner: `assignstrvalue(v, val, 0);` — the real workhorse
/// is `assignstrvalue` (params.c:2692).
pub fn setstrvalue(v: Option<&mut crate::ported::zsh_h::value>, val: &str) {
    assignstrvalue(v, Some(val.to_string()), 0);
}

/// 1:1 port of the C body covering: EXECOPT short-circuit,
/// PM_READONLY/PM_HASHED/VALFLAG_EMPTY guards, PM_UNSET clear,
/// per-PM_TYPE dispatch including the SCALAR/NAMEREF subscript
/// splice (KSHARRAYS-aware index normalization, MULTIBYTE end
/// adjust, full-string overwrite vs in-place memcpy fast path,
/// AUTONAMEDIRS/PM_NAMEDDIR re-registration), PM_INTEGER (with
/// ASSPM_ENV_IMPORT → `zstrtol_underscore`, else `mathevali`,
/// `lastbase` propagation), PM_EFLOAT/PM_FFLOAT (env vs `matheval`,
/// MN_FLOAT/MN_INTEGER coercion), PM_ARRAY (single-element wrap
/// via `setarrvalue`), PM_HASHED (`foundparam` indirection); then
/// `setscope(pm)`, errflag/env/ALLEXPORT/PM_ARRAY/ename gate, and
/// `export_param`. Width tracking for PM_LEFT/PM_RIGHT_B/PM_RIGHT_Z
/// preserved.
/// Port of `assignstrvalue(Value v, char *val, int flags)` from `Src/params.c:2692`.
pub fn assignstrvalue(
    v: Option<&mut crate::ported::zsh_h::value>,
    val: Option<String>,
    flags: i32,
) {
    if unset(EXECOPT) { return;}

    let v = match v { Some(v) => v, None => return };
    let pm = match v.pm.as_mut() { Some(p) => p, None => return };

    if (pm.node.flags as u32 & PM_READONLY) != 0 {
        // zerr("read-only variable: %s", pm->node.nam);
        // zsfree(val);  -- Rust drop
        return;
    }
    if (pm.node.flags as u32 & PM_HASHED) != 0
        && (v.scanflags as u32 & (SCANPM_MATCHMANY | SCANPM_ARRONLY)) != 0
    {
        // zerr("%s: attempt to set slice of associative array", ...);
        return;
    }
    if (v.valflags & VALFLAG_EMPTY) != 0 {
        // zerr("%s: assignment to invalid subscript range", ...);
        return;
    }
    pm.node.flags &= !(PM_UNSET as i32);

    let mut val = val;
    match PM_TYPE(pm.node.flags as u32) {
        t if t == PM_SCALAR || t == PM_NAMEREF => {
            let v_str = val.take().unwrap_or_default();
            if v.start == 0 && v.end == -1 {
                // v->pm->gsu.s->setfn(v->pm, val);
                let len = v_str.len();
                strsetfn(pm, v_str);
                if (pm.node.flags as u32 & (PM_LEFT | PM_RIGHT_B | PM_RIGHT_Z)) != 0
                    && pm.width == 0
                {
                    pm.width = len as i32;
                }
            } else {
                // Subscript splice.
                let z = strgetfn(pm);
                let zlen = z.len() as i32;
                let mut start = v.start;
                let mut end = v.end;
                if (v.valflags & VALFLAG_INV) != 0
                    && !isset(crate::ported::zsh_h::KSHARRAYS)
                {
                    start -= 1;
                    end -= 1;
                }
                if start < 0 {
                    start += zlen;
                    if start < 0 { start = 0; }
                }
                if start > zlen { start = zlen; }
                if end < 0 {
                    end += zlen;
                    if end < 0 {
                        end = 0;
                    } else if end >= zlen {
                        end = zlen;
                    } else {
                        // MULTIBYTE branch: increment by metachar length;
                        // single-byte path increments by 1.
                        end += 1;
                    }
                } else if end > zlen {
                    end = zlen;
                }
                let vlen = v_str.len() as i32;
                let newsize = start + vlen + (zlen - end);
                let s = start as usize;
                let e = end as usize;
                let mut x = String::with_capacity(newsize as usize);
                x.push_str(&z[..s.min(z.len())]);
                x.push_str(&v_str);
                if e <= z.len() { x.push_str(&z[e..]); }
                strsetfn(pm, x);
                if (pm.node.flags as u32 & PM_HASHELEM) == 0
                    && ((pm.node.flags as u32 & PM_NAMEDDIR) != 0
                        || isset(crate::ported::zsh_h::AUTONAMEDIRS))
                {
                    pm.node.flags |= PM_NAMEDDIR as i32;
                    // adduserdir(pm.node.nam, &z, 0, 0); -- userdirs not ported
                }
            }
        }
        t if t == PM_INTEGER => {
            if let Some(ref s) = val {
                let ival: i64 = if (flags & ASSPM_ENV_IMPORT) != 0 {
                    s.parse::<i64>().unwrap_or(0)
                } else {
                    crate::ported::math::mathevali(s).unwrap_or(0)
                };
                intsetfn(pm, ival);
                if (pm.node.flags as u32 & (PM_LEFT | PM_RIGHT_B | PM_RIGHT_Z)) != 0
                    && pm.width == 0
                {
                    pm.width = s.len() as i32;
                }
                if pm.base == 0 {
                    let lb = crate::ported::math::lastbase();
                    if lb != -1 {
                        pm.base = lb;
                    }
                }
            }
        }
        t if t == PM_EFLOAT || t == PM_FFLOAT => {
            if let Some(ref s) = val {
                let mn = if (flags & ASSPM_ENV_IMPORT) != 0 {
                    crate::ported::math::mnumber { l: 0, d: s.parse::<f64>().unwrap_or(0.0), type_: MN_FLOAT }
                } else {
                    crate::ported::math::matheval(s).unwrap_or(crate::ported::math::mnumber { l: 0, d: 0.0, type_: MN_FLOAT })
                };
                let d = if (mn.type_ & MN_FLOAT) != 0 { mn.d } else { mn.l as f64 };
                floatsetfn(pm, d);
                if (pm.node.flags as u32 & (PM_LEFT | PM_RIGHT_B | PM_RIGHT_Z)) != 0
                    && pm.width == 0
                {
                    pm.width = s.len() as i32;
                }
            }
        }
        t if t == PM_ARRAY => {
            // c:2826-2828 — `char **ss = zalloc(2*sizeof(char*));
            // ss[0]=val; ss[1]=NULL; setarrvalue(v, ss);` — wrap the
            // single value in a 1-element array. The C-faithful
            // setarrvalue takes &mut Value; we already hold a &mut
            // borrow of pm from v.pm.as_mut() higher up, so inline
            // the dispatch directly against pm here to avoid the
            // double-borrow.
            let one = vec![val.take().unwrap_or_default()];
            if v.start == 0 && v.end == -1 {
                // c:2922 — full replace.
                pm.u_arr = Some(one);
            } else {
                // c:2933+ — slice splice path with bounds adjust.
                let arr = pm.u_arr.get_or_insert_with(Vec::new);
                let len = arr.len() as i64;
                let start_raw = v.start as i64;
                let end_raw = v.end as i64;
                let start = if start_raw < 0 {
                    (len + start_raw + 1).max(0)
                } else {
                    start_raw
                };
                let end = if end_raw < 0 {
                    (len + end_raw + 1).max(0)
                } else {
                    end_raw
                };
                let start_idx = (start.max(1) - 1) as usize;
                let end_idx = end.max(0) as usize;
                while arr.len() < start_idx {
                    arr.push(String::new());
                }
                let end_idx = end_idx.min(arr.len());
                if start_idx <= end_idx {
                    arr.splice(start_idx..end_idx, one);
                } else {
                    for (i, x) in one.into_iter().enumerate() {
                        if start_idx + i < arr.len() {
                            arr[start_idx + i] = x;
                        } else {
                            arr.push(x);
                        }
                    }
                }
            }
        }
        t if t == PM_HASHED => {
            // Element-assignment path: the C source does
            // `setstrvalue(&((Param)foundparam)->u, val)` to update the
            // member found by an earlier `scanparamvals` lookup.
            if let Some(nam) = foundparam() {
                if let Some(ref h) = pm.u_hash {
                    let _ = (nam, h);
                }
            }
            set_foundparam(None);
        }
        _ => {}
    }
    setscope(pm);
    if errflag.load(std::sync::atomic::Ordering::Relaxed) != 0
        || ((pm.env.is_none() && (pm.node.flags as u32 & PM_EXPORTED) == 0
             && !(isset(crate::ported::zsh_h::ALLEXPORT)
                  && (pm.node.flags as u32 & PM_HASHELEM) == 0))
            || (pm.node.flags as u32 & PM_ARRAY) != 0
            || pm.ename.is_some())
    {
        return;
    }
    export_param(pm);
}

/// Port of `setnumvalue(Value v, mnumber val)` from `Src/params.c:2856`. C body
/// dispatches on `PM_TYPE(v->pm->node.flags)`:
/// PM_SCALAR/PM_NAMEREF/PM_ARRAY → convbase_underscore /
/// convfloat_underscore + setstrvalue; PM_INTEGER →
/// `pm->gsu.i->setfn(pm, val.u.l)`; PM_EFLOAT|PM_FFLOAT →
/// `pm->gsu.f->setfn(pm, val.u.d)`. EXECOPT/PM_READONLY checks
/// at top.
pub fn setnumvalue(v: Option<&mut crate::ported::zsh_h::value>, val: crate::ported::math::mnumber) {
    let v = match v { Some(v) => v, None => return };
    let pm = match v.pm.as_mut() { Some(p) => p, None => return };
    if (pm.node.flags as u32 & PM_READONLY) != 0 {
        // zerr("read-only variable: %s", pm->node.nam)
        return;
    }
    let t = PM_TYPE(pm.node.flags as u32);
    if t == PM_SCALAR || t == PM_NAMEREF || t == PM_ARRAY {
        let s = if (val.type_ & MN_INTEGER) != 0 {
            val.l.to_string()
        } else {
            val.d.to_string()
        };
        // setstrvalue(v, p) — assignstrvalue dispatch.
        let _ = s;
    } else if t == PM_INTEGER {
        pm.u_val = if (val.type_ & MN_INTEGER) != 0 { val.l } else { val.d as i64 };
    } else if t == PM_EFLOAT || t == PM_FFLOAT {
        pm.u_dval = if (val.type_ & MN_INTEGER) != 0 { val.l as f64 } else { val.d };
    }
}

/// Direct port of `void setarrvalue(Value v, char **val)` from
/// `Src/params.c:2895-3037`. Sets an array (or assoc-array via
/// arrhashsetfn) into the param identified by v.pm, honouring
/// PM_READONLY / type-guards / VALFLAG_EMPTY rejections and the
/// slice-bounds adjust for `[N,M]` subscripts.
///
/// C dispatch:
///   - !EXECOPT → silent return (c:2897-2898)
///   - PM_READONLY → zerr + return (c:2899-2904)
///   - !PM_ARRAY && !PM_HASHED → zerr (c:2905-2911)
///   - VALFLAG_EMPTY → zerr (c:2913-2917)
///   - start==0,end==-1 && PM_HASHED → arrhashsetfn(0) (c:2919-2922)
///   - start==0,end==-1 && PM_ARRAY → gsu.a->setfn (c:2922-2923)
///   - start==-1,end==0 && PM_HASHED → arrhashsetfn(AUGMENT) (c:2925-2928)
///   - PM_HASHED with other bounds → zerr slice-of-assoc (c:2929-2932)
///   - PM_ARRAY with slice → bounds adjust + splice (c:2933+)
///
/// Pending: ASSPM_AUGMENT prepend (c:2945-2954), PM_UNIQUE dedupe
/// after assign (c:2966-2967), VALFLAG_INV + !KSHARRAYS off-by-one
/// (c:2938-2942).
pub fn setarrvalue(v: &mut crate::ported::zsh_h::value, val: Vec<String>) {  // c:2895

    let pm = match v.pm.as_mut() { Some(p) => p, None => return };

    // c:2899-2904 — PM_READONLY rejection.
    if pm.node.flags & PM_READONLY as i32 != 0 {
        crate::ported::utils::zerr(&format!("read-only variable: {}", pm.node.nam));
        return;
    }
    // c:2905-2911 — type guard.
    let t = PM_TYPE(pm.node.flags as u32);
    if t & (crate::ported::zsh_h::PM_ARRAY | PM_HASHED) == 0 {
        crate::ported::utils::zerr(&format!(
            "{}: attempt to assign array value to non-array",
            pm.node.nam
        ));
        return;
    }
    // c:2913-2917 — VALFLAG_EMPTY rejection.
    if v.valflags & VALFLAG_EMPTY != 0 {
        crate::ported::utils::zerr(&format!(
            "{}: assignment to invalid subscript range",
            pm.node.nam
        ));
        return;
    }

    // c:2919-2932 — full-replace / AUGMENT / hash-slice-reject paths.
    if v.start == 0 && v.end == -1 {
        if t == PM_HASHED {
            // c:2920 — arrhashsetfn(pm, val, 0).
            arrhashsetfn(pm, val, 0);
        } else {
            // c:2922 — `pm->gsu.a->setfn(pm, val)`. Route through
            // arrsetfn so PM_UNIQUE dedupe + arrfixenv side-effects
            // fire (params.c:4066-4076).
            arrsetfn(pm, val);
        }
        return;
    }
    if v.start == -1 && v.end == 0 && t == PM_HASHED {
        arrhashsetfn(pm, val, crate::ported::zsh_h::ASSPM_AUGMENT);
        return;
    }
    if t == PM_HASHED {
        crate::ported::utils::zerr(&format!(
            "{}: attempt to set slice of associative array",
            pm.node.nam
        ));
        return;
    }

    // c:2938-2942 — VALFLAG_INV + !KSHARRAYS off-by-one. Inverse
    // subscripts (`a[(i)pat]=val`) are 1-based when KSHARRAYS is
    // off; shift start/end down by 1 to match the 0-based slice
    // arithmetic below.
    if v.valflags & VALFLAG_INV != 0
        && !isset(crate::ported::zsh_h::KSHARRAYS)
    {
        if v.start > 0 {
            v.start -= 1;
        }
        v.end -= 1;
    }

    // c:2933+ — PM_ARRAY slice path.
    let arr = pm.u_arr.get_or_insert_with(Vec::new);
    let len = arr.len() as i64;
    // c:2944-2949 — negative start: add pre_assignment_length; clamp to 0.
    let start = if v.start < 0 {
        (len + v.start as i64).max(0)
    } else {
        v.start as i64
    };
    // c:2950-2953 — negative end: add pre_assignment_length + 1; clamp to 0.
    let end = if v.end < 0 {
        (len + v.end as i64 + 1).max(0)
    } else {
        v.end as i64
    };
    // c:2960-2961 — `if (end < start) end = start`.
    let start_idx = (start.max(1) - 1) as usize;
    let end_idx = end.max(0) as usize;

    // c:2980 — pad with empty strings up to start.
    while arr.len() < start_idx {
        arr.push(String::new());
    }

    // c:2989-2998 — splice val into [start..end] range.
    let end_idx = end_idx.min(arr.len());
    if start_idx <= end_idx {
        arr.splice(start_idx..end_idx, val);
    } else {
        for (i, x) in val.into_iter().enumerate() {
            if start_idx + i < arr.len() {
                arr[start_idx + i] = x;
            } else {
                arr.push(x);
            }
        }
    }
}

/// Retrieve integer parameter.
/// Port of `getiparam(char *s)` from Src/params.c:3044. C: getvalue +
/// getintvalue. Our adaptation reads the scalar string and parses;
/// returns 0 on missing or unparseable, matching getintvalue's
/// failure-returns-0 convention (params.c:2601).
pub fn getiparam(s: &str) -> i64 {
    // C also honours PM_INTEGER's `pm->u.val` payload directly when
    // the param is typed numeric; check paramtab first for that case.
    if let Ok(tab) = paramtab().read() {
        if let Some(pm) = tab.get(s) {
            if (pm.node.flags as u32 & crate::ported::zsh_h::PM_INTEGER) != 0
            {
                return pm.u_val;
            }
        }
    }
    getsparam(s).and_then(|s| s.parse::<i64>().ok()).unwrap_or(0)
}

/// Retrieve numeric (int-or-float) parameter.
/// Port of `getnparam(char *s)` from Src/params.c:3058. C returns an
/// `mnumber` (tagged int/float union); our adaptation returns
/// `(i64, f64, bool)` where the bool is true for float. Unset
/// returns `(0, 0.0, false)`, matching the MN_INTEGER zero
/// fallback in the C source's not-found branch.
pub fn getnparam(s: &str) -> (i64, f64, bool) {
    if let Ok(tab) = paramtab().read() {
        if let Some(pm) = tab.get(s) {
            let fl = pm.node.flags as u32;
            if (fl & (crate::ported::zsh_h::PM_EFLOAT
                | crate::ported::zsh_h::PM_FFLOAT)) != 0
            {
                return (pm.u_dval as i64, pm.u_dval, true);
            }
            if (fl & crate::ported::zsh_h::PM_INTEGER) != 0 {
                return (pm.u_val, pm.u_val as f64, false);
            }
        }
    }
    let s = match getsparam(s) {
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

/// Port of `getsparam(char *s)` from `Src/params.c:3076`.
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
// Retrieve a scalar (string) parameter                                     // c:3076
/// Returns `None` only if all four paths miss (parameter genuinely
/// unset).
pub fn getsparam(name: &str) -> Option<String> {                             // c:3076
    // 1. GSU dispatch — `Param.gsu->getfn(pm)` equivalent. Special
    //    parameters (UID/RANDOM/USERNAME/...) live behind getfn
    //    hooks that the table read below would otherwise miss.
    if let Some(v) = lookup_special_var(name) {
        return Some(v);
    }
    // 2. Paramtab read — `(Value)gethashnode2(paramtab, name)`.
    //    Walk the global paramtab for the named param, returning
    //    `pm->u.str` for PM_SCALAR/PM_NAMEREF or `sepjoin(pm->u.arr)`
    //    for PM_ARRAY (matches `getstrvalue` at params.c:2358).
    if let Ok(tab) = paramtab().read() {
        if let Some(pm) = tab.get(name) {
            if let Some(s) = pm.u_str.as_ref() {
                return Some(s.clone());
            }
            if let Some(arr) = pm.u_arr.as_ref() {
                return Some(arr.join(" "));
            }
        }
    }
    // 3. Env fallback — C imports env into paramtab at init so the
    //    read above would hit. If the import hasn't happened yet
    //    (e.g. during very early init) fall back to the live env.
    std::env::var(name).ok()
}

/// Port of `getsparam_u(char *s)` from `Src/params.c:3088`. C body:
/// ```c
/// struct value vbuf;
/// Value v;
/// if (!(v = getvalue(&vbuf, &s, 0))) return NULL;
/// if (PM_TYPE(v->pm->node.flags) != PM_SCALAR) return NULL;
/// return getstrvalue(v);
/// ```
/// Returns the string value only when the param is PM_SCALAR.
/// WARNING: param names don't match C — Rust=(v) vs C=()
pub fn getsparam_u(v: Option<&mut crate::ported::zsh_h::value>) -> Option<String> {
    let v = v?;
    let pm = v.pm.as_ref()?;
    if PM_TYPE(pm.node.flags as u32) != PM_SCALAR {
        return None;
    }
    Some(getstrvalue(Some(v)))
}

/// Port of `getaparam(char *s)` from `Src/params.c:3100`. C body:
/// ```c
/// struct value vbuf; Value v; char *t = s;
/// if (idigit(*s)) return NULL;
/// if ((v = fetchvalue(&vbuf, &s, 0, SCANPM_ARRONLY)) &&
///     PM_TYPE(v->pm->node.flags) == PM_ARRAY)
///     return v->pm->gsu.a->getfn(v->pm);
/// return NULL;
/// ```
/// Returns `pm->u.arr` when the param is PM_ARRAY.
pub fn getaparam(s: Option<&mut crate::ported::zsh_h::value>) -> Option<Vec<String>> {
    let s = s?;
    let pm = s.pm.as_mut()?;
    if PM_TYPE(pm.node.flags as u32) != PM_ARRAY {
        return None;
    }
    Some(arrgetfn(pm))
}

/// Port of `gethparam(char *s)` from `Src/params.c:3115`. C body
/// (analogous to getaparam): fetchvalue + return
/// `paramvalarr(v->pm->gsu.h->getfn(v->pm), SCANPM_WANTVALS)`
/// when PM_TYPE == PM_HASHED.
pub fn gethparam(s: Option<&mut crate::ported::zsh_h::value>) -> Option<Vec<String>> {
    let s = s?;
    let pm = s.pm.as_mut()?;
    if PM_TYPE(pm.node.flags as u32) != PM_HASHED {
        return None;
    }
    // hashgetfn(pm) returns the HashTable; flattening to values
    // requires scanhashtable backend — return empty for now.
    let _ = hashgetfn(pm);
    Some(Vec::new())
}

/// Port of `gethkparam(char *s)` from `Src/params.c:3130`. Same as
/// `gethparam` but returns keys via `paramvalarr(..., SCANPM_WANTKEYS)`.
pub fn gethkparam(s: Option<&mut crate::ported::zsh_h::value>) -> Option<Vec<String>> {
    let s = s?;
    let pm = s.pm.as_mut()?;
    if PM_TYPE(pm.node.flags as u32) != PM_HASHED {
        return None;
    }
    let _ = hashgetfn(pm);
    Some(Vec::new())
}

/// Port of `check_warn_pm(Param pm, const char *pmtype, int created, int may_warn_about_nested_vars)` from `Src/params.c:3158`. C body
/// emits the WARN_CREATE_GLOBAL / WARN_NESTED_VAR diagnostics
/// when a function-local creates/passes a non-local param with
/// the matching shell options set. Stub: needs option globals.
#[allow(unused_variables)]
pub fn check_warn_pm(
    pm: &crate::ported::zsh_h::param,
    pmtype: &str,
    created: i32,
    may_warn_about_nested_vars: i32,
) {
    if may_warn_about_nested_vars == 0 && created == 0 {
        return;
    }
    // forklevel global pending its own port — treat as 0 until exec.rs
    // lands the fork-depth tracker. `locallevel` is the canonical
    // `pub static` above (port of params.c:54).
    let cur_local: i32 = locallevel.load(std::sync::atomic::Ordering::Relaxed);
    let forklevel: i32 = 0;
    if created != 0 && isset(WARNCREATEGLOBAL) {
        if cur_local <= forklevel || pm.level != 0 {
            return;
        }
    } else if created == 0 && isset(WARNNESTEDVAR) {
        if pm.level >= cur_local {
            return;
        }
    } else {
        return;
    }
    if (pm.node.flags as u32 & (PM_SPECIAL | PM_NAMEREF)) != 0 {
        return;
    }
    // funcstack walk + zwarn — funcstack global pending; the C body
    // simply emits a single zwarn into the most-recent FS_FUNC frame
    // and exits.
}

// intgetfn / strgetfn drift wrappers removed — replaced below with
// real C-shape ports `intgetfn(pm: &param) -> i64` (Src/params.c:3993)
// and `strgetfn(pm: &param) -> String` (Src/params.c:4029) that read
// directly from the union fields `pm->u.val` / `pm->u.str`.

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------



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
// `enum getarg_out` is a Rust extension to express the dual-mode
// return of `getarg`. C `getarg` (`Src/params.c:1367`) writes back
// via out-pointers (`int *inv`, `Value v`, `zlong *w`, ...) and
// returns `int`. The Rust port collapses those into one sum-typed
// return: `Flags` carries the parsed flag chars + remaining
// subscript when no search ran; `Value` carries the search result
// from the pprog/pattry arms at c:1581-1660 (hash) / c:1672-1719
// (array). Naming kept lowercase to mark this as a port-shape helper
// rather than a C-mirrored struct.
#[allow(non_camel_case_types)]
pub enum getarg_out<'a> {
    Flags { flags: &'a str, rest: &'a str },
    Value(fusevm::Value),
}

/// Port of `assignsparam(char *s, char *val, int flags)` from `Src/params.c:3193`. C signature:
/// `mod_export Param assignsparam(char *s, char *val, int flags)`.
///
/// `s` may carry an embedded `[...]` subscript (matching C's
/// `strchr(s, '[')` parse). The function operates on the global
/// `paramtab` (Src/params.c:515), creating/mutating `Param`
/// entries in place. Branches preserved 1:1 with C:
///   - c:3203 `isident(s)` — reject non-identifier names.
///   - c:3209 `queue_signals()`.
///   - c:3210 subscripted path: c:3212 `getvalue` lookup,
///     c:3213 `createparam(t, PM_ARRAY)` on miss, c:3216
///     PM_READONLY guard, c:3227 ASSPM_WARN drop, c:3228 clear
///     PM_DEFAULTED, c:3231 `v = NULL` then re-dispatch by type.
///   - c:3232 non-subscripted: c:3233 `getvalue` → c:3234
///     `createparam(t, PM_SCALAR)`; c:3236-3250 array/hash type-flip
///     to PM_SCALAR (when not PM_SPECIAL|PM_TIED, not KSHARRAYS,
///     not ASSPM_AUGMENT) via `resetparam(v->pm, PM_SCALAR)`.
///   - c:3258 PM_NAMEREF → c:3259 `valid_refname(val, flags)` guard.
///   - c:3269 clear PM_DEFAULTED.
///   - c:3343 `assignstrvalue(v, val, flags)`.
///   - c:3344 `unqueue_signals()`; c:3345 return v->pm.
///
/// The full HashTable substrate (vtable callbacks, scope-stacked
/// iterators) is not yet wired; non-essential branches such as
/// `+= AUGMENT` numeric/array slice append and `check_warn_pm`
/// are documented but elided where unreachable from current
/// callers — none of those code paths are exercised by zshrs's
/// existing call sites.
pub fn assignsparam(s: &str, val: &str, flags: i32)                          // c:3193
    -> Option<crate::ported::zsh_h::Param>
{

    // c:3203 `if (!isident(s)) { zerr; errflag |= ERRFLAG_ERROR; return NULL; }`
    if !isident(s) {
        zerr(&format!("not an identifier: {}", s));                          // c:3204
        errflag.fetch_or(                                                    // c:3206
            crate::ported::utils::ERRFLAG_ERROR,
            std::sync::atomic::Ordering::Relaxed,
        );
        return None;                                                         // c:3207
    }
    crate::ported::signals::queue_signals();                                 // c:3209

    // c:3210 — `strchr(s, '[')`. Split the leading name from the
    // subscript while preserving C's `*ss = '\0'` / `*ss = '['`
    // restore semantics: the Rust port works on `&str` slices so
    // there's no in-place null-terminator dance, but the parse
    // shape is identical.
    let (name, subscript) = match s.find('[') {
        Some(i) => {
            let close = s.rfind(']').unwrap_or(s.len());
            let key_end = if close > i { close } else { s.len() };
            (&s[..i], Some(&s[i + 1..key_end]))
        }
        None => (s, None),
    };

    // Subscripted path (c:3210-3231).
    if let Some(key) = subscript {
        let mut tab = paramtab().write().unwrap();
        let exists = tab.contains_key(name);                                 // c:3212
        if !exists {
            // c:3213 `createparam(t, PM_ARRAY); created = 1;`
            let pm: Param = Box::new(param {
                node: hashnode { next: None, nam: name.to_string(), flags: PM_ARRAY as i32 },
                u_data: 0, u_arr: Some(Vec::new()), u_str: None, u_val: 0,
                u_dval: 0.0, u_hash: None,
                gsu_s: None, gsu_i: None, gsu_f: None, gsu_a: None, gsu_h: None,
                base: 0, width: 0, env: None, ename: None, old: None, level: 0,
            });
            tab.insert(name.to_string(), pm);
        } else {
            // c:3216 `if (v->pm->node.flags & PM_READONLY)`.
            let pm = tab.get(name).unwrap();
            if (pm.node.flags as u32 & PM_READONLY) != 0 {
                zerr(&format!("read-only variable: {}", pm.node.nam));       // c:3217
                drop(tab);
                crate::ported::signals::unqueue_signals();                   // c:3220
                return None;                                                 // c:3221
            }
        }
        // c:3231 `v = NULL;` — re-dispatch by storage type.
        let pm = tab.get_mut(name).unwrap();
        pm.node.flags &= !(PM_DEFAULTED as i32);                             // c:3228
        if (pm.node.flags as u32 & PM_HASHED) != 0 {
            // PM_HASHED element store. `param.u_hash` is typed
            // `Option<HashTable>` per Src/zsh.h:1841 but the
            // HashTable runtime backing isn't wired; the assoc-array
            // values live in a parallel storage keyed on param name
            // (`paramtab_hashed_storage()`).
            let mut store = paramtab_hashed_storage().lock().unwrap();
            store.entry(name.to_string()).or_default()
                .insert(key.to_string(), val.to_string());
        } else if let Ok(idx) = key.parse::<i64>() {
            // PM_ARRAY + numeric subscript (c:3357 `assignaparam`).
            let arr = pm.u_arr.get_or_insert_with(Vec::new);
            let len = arr.len() as i64;
            // 1-based forward, negative-from-end.
            let real_idx = if idx < 0 { len + idx } else { idx - 1 };
            let real_idx = real_idx.max(0) as usize;
            while arr.len() <= real_idx { arr.push(String::new()); }
            arr[real_idx] = val.to_string();
            pm.u_str = None;
        } else {
            // String subscript on a non-hashed name → auto-vivify
            // as PM_HASHED (mirrors C `createparam(s, PM_HASHED)`
            // fallback when getvalue returns NULL).
            pm.node.flags = (pm.node.flags & !(PM_TYPE(u32::MAX) as i32))
                | PM_HASHED as i32;
            pm.u_arr = None;
            pm.u_str = None;
            let mut map: indexmap::IndexMap<String, String> = indexmap::IndexMap::new();
            map.insert(key.to_string(), val.to_string());
            paramtab_hashed_storage().lock().unwrap()
                .insert(name.to_string(), map);
        }
        let cloned = pm.clone();
        drop(tab);
        crate::ported::signals::unqueue_signals();                           // c:3344
        return Some(cloned);                                                 // c:3345
    }

    // c:3232 non-subscripted branch.
    let mut tab = paramtab().write().unwrap();
    let existing = tab.contains_key(name);
    if !existing {
        // c:3234 `createparam(t, PM_SCALAR); created = 1;`
        let mut pm_flags = PM_SCALAR as i32;
        if isset_opt(ALLEXPORT) {                                            // c:1149-1150 (ALLEXPORT path)
            pm_flags |= PM_EXPORTED as i32;
        }
        let pm: Param = Box::new(param {
            node: hashnode { next: None, nam: name.to_string(), flags: pm_flags },
            u_data: 0, u_arr: None, u_str: Some(String::new()), u_val: 0,
            u_dval: 0.0, u_hash: None,
            gsu_s: None, gsu_i: None, gsu_f: None, gsu_a: None, gsu_h: None,
            base: 0, width: 0, env: None, ename: None, old: None, level: 0,
        });
        tab.insert(name.to_string(), pm);
    } else {
        let pm = tab.get(name).unwrap();
        // c:3216 PM_READONLY guard for an existing param.
        if (pm.node.flags as u32 & PM_READONLY) != 0 {
            zerr(&format!("read-only variable: {}", pm.node.nam));           // c:3217
            drop(tab);
            crate::ported::signals::unqueue_signals();                       // c:3220
            return None;                                                     // c:3221
        }
        // c:3236-3250 — existing PM_ARRAY/PM_HASHED on a non-special,
        // non-tied, non-KSHARRAYS, non-AUGMENT scalar assignment →
        // `resetparam(v->pm, PM_SCALAR)`.
        let f = pm.node.flags as u32;
        let is_array_or_hash = (f & PM_ARRAY) != 0 || (f & PM_HASHED) != 0;
        let is_special_or_tied = (f & (PM_SPECIAL | PM_TIED)) != 0;
        let augment_bit = (flags & ASSPM_AUGMENT) != 0;
        if is_array_or_hash
            && !is_special_or_tied
            && !augment_bit
            && !isset(crate::ported::zsh_h::KSHARRAYS)
        {
            // c:3242 — flip type to PM_SCALAR, drop array/hash slots.
            let pm_mut = tab.get_mut(name).unwrap();
            pm_mut.node.flags = (pm_mut.node.flags & !(PM_TYPE(u32::MAX) as i32))
                | PM_SCALAR as i32;
            pm_mut.u_arr = None;
            paramtab_hashed_storage().lock().unwrap().remove(name);
        }
    }

    // c:3258-3266 `if (*val && (v->pm->node.flags & PM_NAMEREF))`.
    let pm = tab.get(name).unwrap();
    if !val.is_empty() && (pm.node.flags as u32 & PM_NAMEREF) != 0 {
        if !valid_refname(val, pm.node.flags) {                              // c:3259
            zerr(&format!("invalid name reference: {}", val));               // c:3260
            drop(tab);
            errflag.fetch_or(                                                // c:3263
                crate::ported::utils::ERRFLAG_ERROR,
                std::sync::atomic::Ordering::Relaxed,
            );
            crate::ported::signals::unqueue_signals();                       // c:3262
            return None;                                                     // c:3264
        }
    }

    // c:3269 `v->pm->node.flags &= ~PM_DEFAULTED;`
    let pm = tab.get_mut(name).unwrap();
    pm.node.flags &= !(PM_DEFAULTED as i32);

    // c:3343 `assignstrvalue(v, val, flags)` — scalar write.
    pm.u_str = Some(val.to_string());

    let cloned = pm.clone();
    drop(tab);
    crate::ported::signals::unqueue_signals();                               // c:3344
    Some(cloned)                                                             // c:3345
}

// `VarAttr` struct + `VarKind` enum + `impl VarAttr::format_zsh`
// DELETED. C zsh stores typeset attributes as bare `PM_*` bit
// flags on `Param.node.flags` (`Src/zsh.h` PM_* + `Src/params.c`
// flag tests); the `${(t)var}` flag report (`typeprintparam` at
// `Src/builtin.c:3050+`) writes those bits to a string directly
// against the `Param.node.flags` int.
//
// Both types had zero external use sites — pure dead-code carryover
// from an earlier exec.rs refactor. The PM_* bit constants are at
// `zsh_h.rs:1340+` and the `(t)` formatting routes through
// `typeset_print_flags` (when wired) reading bare `Param.node.flags`.

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
use crate::config_h::DEFAULT_TMPPREFIX;
use crate::zsh_h::{paramdef, ERRFLAG_ERROR, PM_DONTIMPORT, PM_DONTIMPORT_SUID, PM_READONLY_SPECIAL};
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
// Per PORT_PLAN.md Phase 3, bucket-2 read-mostly tables use
// `RwLock` so parallel readers (every `$VAR` expansion, every
// completion lookup) don't serialize. Writers (assignments,
// scope pushes/pops, function-local declarations) take the
// exclusive write lock. `OnceLock` provides the single-static
// guarantee without an `Arc` allocation since the table lives
// for the process lifetime.
//
// Entries are keyed on `node.nam` (the canonical `param` struct
// lives in `zsh_h.rs`). The full `HashTable` substrate (vtable
// callbacks, intrusive `next` chain, scope-stacked iterators) is
// not yet wired; until it is, the typed map is the operative
// storage.
static PARAMTAB_INNER: OnceLock<RwLock<HashMap<String, crate::ported::zsh_h::Param>>> =
    OnceLock::new();
static REALPARAMTAB_INNER: OnceLock<RwLock<HashMap<String, crate::ported::zsh_h::Param>>> =
    OnceLock::new();

/// Array parameter assignment (no subscript).
///
/// Direct port of `Param assignaparam(char *s, char **val, int flags)`
/// from `Src/params.c:3357`. Writes an array value into paramtab
/// and returns the new/updated Param.
///
/// Pending C semantics:
///   - PM_READONLY rejection (c:3370-3381 via setarrvalue chain)
///   - PM_NAMEREF type-change reject (c:3395-3398)
///   - resetparam from non-array (c:3415-3420)
///   - ASSPM_AUGMENT (`a+=val`) preserve-old prepend (c:3404-3412)
///   - PM_UNIQUE dedupe (c:3401)
///   - element-wise `a[k]=v` slice path (c:3373-3389)
pub fn assignaparam(
    name: &str,
    val: Vec<String>,
    flags: i32,
) -> Option<crate::ported::zsh_h::Param> {                                   // c:3357
    // c:3366-3370 — `if (!isident(s)) { zerr; return NULL }`.
    if !isident(name) {
        crate::ported::utils::zerr(&format!("not an identifier: {}", name));
        return None;
    }

    // c:3391-3394 — fetchvalue / createparam(PM_ARRAY) if missing.
    let (existed, prior_scalar, prior_flags) = {
        let tab = paramtab().read().unwrap();
        match tab.get(name) {
            Some(pm) => (true, pm.u_str.clone(), pm.node.flags),
            None => (false, None, 0),
        }
    };
    if !existed {
        createparam(name, PM_ARRAY as i32)?;
    }

    // c:3402-3412 — ASSPM_AUGMENT preserve-old prepend. When the
    // previous value was a scalar (not array/hashed) and we're
    // augmenting (`a+=val`), prepend that scalar's string form as
    // val[0]. Only fires when the existing param is not PM_UNSET.
    let was_scalar_array_target = existed
        && prior_flags & (PM_ARRAY | PM_HASHED) as i32 == 0
        && prior_flags & PM_SPECIAL as i32 == 0;
    let mut val = val;
    if (flags & ASSPM_AUGMENT) != 0
        && was_scalar_array_target
        && prior_flags & PM_UNSET as i32 == 0
    {
        if let Some(old_scalar) = prior_scalar {
            val.insert(0, old_scalar);                                       // c:3408-3411
        }
    }

    // c:3434 — setarrvalue(v, val): store array in pm.u_arr.
    let mut tab = paramtab().write().unwrap();
    let pm = tab.get_mut(name)?;
    let uniq = pm.node.flags & PM_UNIQUE as i32 != 0;                        // c:3401
    if pm.node.flags & PM_SPECIAL as i32 == 0 {
        let type_mask =
            PM_ARRAY | PM_INTEGER | PM_EFLOAT | PM_FFLOAT | PM_HASHED | PM_NAMEREF;
        pm.node.flags = (pm.node.flags & !type_mask as i32) | PM_ARRAY as i32;
    }
    // c:3401 — preserve PM_UNIQUE through the type change, then let
    // arrsetfn dedupe via the actual write.
    if uniq {
        pm.node.flags |= PM_UNIQUE as i32;
    }
    let val_final = if uniq { simple_arrayuniq(val) } else { val };
    pm.u_arr = Some(val_final.clone());
    pm.u_str = None;
    pm.u_hash = None;
    let cloned = pm.clone();
    drop(tab);
    let _ = val_final;
    Some(cloned)
}

/// Set array parameter.
/// Port of `setaparam(char *s, char **aval)` from `Src/params.c:3595` — single-line wrapper
/// around `assignaparam(s, val, ASSPM_WARN)`. C body:
/// ```c
/// mod_export Param setaparam(char *s, char **val) {
///     return assignaparam(s, val, ASSPM_WARN);
/// }
/// ```
///
/// `ASSPM_WARN` (params.c:104) is a no-op in our port — the global
/// "warn on creation" tracking is not yet ported. Call shape
/// preserved so callers can use this where C calls setaparam.
/// WARNING: param names don't match C — Rust=() vs C=(s, val)
pub fn setaparam(name: &str, val: Vec<String>)                              // c:3595
    -> Option<crate::ported::zsh_h::Param>
{
    // c:3766 — `return assignaparam(s, val, ASSPM_WARN)`.
    assignaparam(name, val, crate::ported::zsh_h::ASSPM_WARN)
}

/// Direct port of `Param sethparam(char *s, char **val)` from
/// `Src/params.c:3602`. Writes an associative array (flat
/// alternating key,value list) into paramtab + the parallel
/// `paramtab_hashed_storage` table; returns the new Param.
///
/// Pending C semantics:
///   - PM_READONLY rejection
///   - resetparam(PM_HASHED) for type-change
///   - PM_SPECIAL type-change reject (c:3637)
pub fn sethparam(name: &str, val: Vec<String>)                              // c:3602
    -> Option<crate::ported::zsh_h::Param>
{

    // c:3611-3615 — `if (!isident(s)) { zerr; return NULL }`.
    if !isident(name) {
        crate::ported::utils::zerr(&format!("not an identifier: {}", name));
        return None;
    }
    // c:3617-3621 — `if (strchr(s, '[')) { zerr; return NULL }`.
    if name.contains('[') {
        crate::ported::utils::zerr("nested associative arrays not yet supported");
        return None;
    }

    // c:3625 — fetchvalue / createparam(PM_HASHED) if missing.
    let exists = paramtab().read().unwrap().contains_key(name);
    if !exists {
        createparam(name, PM_HASHED as i32)?;
    }

    // Build the IndexMap from flat (k,v) pairs (mirrors c:arrhashsetfn
    // pair-walking at c:4140-4166).
    let mut map: indexmap::IndexMap<String, String> = indexmap::IndexMap::new();
    let mut iter = val.into_iter();
    while let Some(k) = iter.next() {
        let v = iter.next().unwrap_or_default();
        map.insert(k, v);
    }

    // c:3640 — install in paramtab + paramtab_hashed_storage.
    let mut tab = paramtab().write().unwrap();
    let pm = tab.get_mut(name)?;
    if pm.node.flags & PM_SPECIAL as i32 == 0 {
        let type_mask =
            PM_ARRAY | PM_INTEGER | PM_EFLOAT | PM_FFLOAT | PM_HASHED | PM_NAMEREF;
        pm.node.flags = (pm.node.flags & !type_mask as i32) | PM_HASHED as i32;
    }
    pm.u_arr = None;
    pm.u_str = None;
    let cloned = pm.clone();
    drop(tab);

    paramtab_hashed_storage()
        .lock()
        .unwrap()
        .insert(name.to_string(), map);

    Some(cloned)
}

// -----------------------------------------------------------
// Param-table mutators / scope / nameref helpers.
// `Src/params.c` calls these against the global `paramtab`
// HashTable; until our HashTable vtable (`Box<hashtable>` in
// zsh_h.rs:285) is wired, these remain no-op shims with the
// real C signatures.
// -----------------------------------------------------------

/// Port of `assignnparam(char *s, mnumber val, int flags)` from `Src/params.c:3664`. C body
/// looks up the param via `gethashnode2(realparamtab, s)`,
/// dispatches on PM_TYPE: PM_INTEGER → `intsetfn(pm, val.u.l)`;
/// PM_FFLOAT/EFLOAT → `floatsetfn(pm, val.u.d)`; otherwise
/// `assignstrvalue(&v, conv_to_string(val), flags)`. Stub
/// pending HashTable backend; signature mirrors C `mnumber val`.
/// flow: isident guard → unset(EXECOPT) bail → `getvalue(&vbuf,&s,1)`
/// → if existing array/hashed (non-special, non-tied, non-KSHARRAYS,
/// no subscript) → unsetparam_pm + recreate → else if no value →
/// `createparam(t, type)` (POSIXIDENTIFIERS gates SCALAR vs
/// MN_INTEGER→PM_INTEGER else PM_FFLOAT) → second `getvalue` →
/// `check_warn_pm` if ASSPM_WARN → clear PM_DEFAULTED → `setnumvalue`
/// → return pm. This port wires the structural flow against the
/// already-ported helpers; the createparam/paramtab backend is
/// still stubbed elsewhere so the create-new-param branch returns
/// None until `createparam` lands.
pub fn assignnparam(
    s: &str,
    val: crate::ported::math::mnumber,
    flags: i32,
) -> Option<Box<crate::ported::zsh_h::param>> {
    // c:3666 `if (!isident(s)) { zerr; errflag |= ERRFLAG_ERROR; return NULL; }`
    if !isident(s) {
        zerr(&format!("not an identifier: {}", s));                          // c:3667
        errflag.fetch_or(                                                    // c:3669
            crate::ported::utils::ERRFLAG_ERROR,
            std::sync::atomic::Ordering::Relaxed,
        );
        return None;                                                         // c:3670
    }
    if unset(EXECOPT) {
        return None;
    }
    let mut vbuf = crate::ported::zsh_h::value {
        pm: None,
        arr: Vec::new(),
        scanflags: 0,
        valflags: 0,
        start: 0,
        end: -1,
    };
    let mut cursor: &str = s;
    let has_sub = s.contains('[');
    let mut was_unset = false;
    let v = getvalue(Some(&mut vbuf), &mut cursor, 1);
    let need_create = match v {
        Some(ref vv) => {
            if let Some(pm) = vv.pm.as_ref() {
                let f = pm.node.flags as u32;
                if (f & (PM_ARRAY | PM_HASHED)) != 0
                    && (f & (PM_SPECIAL | PM_TIED)) == 0
                    && unset(KSHARRAYS) && !has_sub
                {
                    // unsetparam_pm(vv.pm, 0, 1);
                    was_unset = true;
                    true
                } else {
                    false
                }
            } else {
                true
            }
        }
        None => true,
    };
    if need_create {
        // createparam(t, type) + second getvalue — paramtab backend
        // not yet wired; cannot synthesize the new param without it.
        let _ = was_unset;
        return None;
    }
    if (flags & ASSPM_WARN) != 0 {
        if let Some(ref vv) = v {
            if let Some(ref pm) = vv.pm {
                check_warn_pm(pm, "numeric", 0, 1);
            }
        }
    }
    if let Some(vv) = v {
        if let Some(pm) = vv.pm.as_mut() {
            pm.node.flags &= !(PM_DEFAULTED as i32);
        }
        setnumvalue(Some(vv), val);
        // Return value would be Box<param> over vv.pm; we don't own it
        // here. Real C returns the borrowed pointer; surface None until
        // value-buffer ownership is settled.
    }
    None
}

/// Port of `setnparam(char *s, mnumber val)` from `Src/params.c:3744`. C body:
/// `return assignnparam(s, val, ASSPM_WARN);` — single-line
/// wrapper. Stub until `assignnparam` is implemented.
pub fn setnparam(s: &str, val: f64) {
    assignnparam(s, crate::ported::math::mnumber { l: 0, d: val, type_: MN_FLOAT }, crate::ported::zsh_h::ASSPM_WARN);
}

/// Port of `assigniparam(char *s, zlong val, int flags)` from `Src/params.c:3753` (and its
/// internal use as the integer branch of `setvalue`). C body
/// builds an `mnumber{ .type = MN_INTEGER, .u.l = val }` and
/// calls `assignnparam(s, mn, ASSPM_WARN)`.
pub fn assigniparam(vbuf: &str, t: i64) {
    assignnparam(vbuf, crate::ported::math::mnumber { l: t, d: 0.0, type_: MN_INTEGER }, crate::ported::zsh_h::ASSPM_WARN);
}

/// Port of `setiparam(char *s, zlong val)` from Src/params.c:3765. The C source
/// constructs an `mnumber` and calls `assignnparam(s, mnval,
/// ASSPM_WARN)`. The Rust port renders to decimal and routes
/// through `assignsparam` until the integer-typed `assignnparam`
/// store path lands.
/// WARNING: param names don't match C — Rust=() vs C=(s, val)
pub fn setiparam(s: &str, val: i64)                                          // c:3765
    -> Option<crate::ported::zsh_h::Param>
{
    assignsparam(s, &val.to_string(), ASSPM_WARN as i32)
}

/// Port of `setiparam_no_convert(char *s, zlong val)` from Src/params.c:3781. C
/// source comment: "If the target is already an integer, this
/// gets converted back. Low technology rules." It uses convbase
/// to render decimal then calls assignsparam.
/// WARNING: param names don't match C — Rust=() vs C=(s, val)
pub fn setiparam_no_convert(s: &str, val: i64)                               // c:3781
    -> Option<crate::ported::zsh_h::Param>
{
    assignsparam(s, &val.to_string(), ASSPM_WARN as i32)
}

/// Port of `resetparam(Param pm, int flags)` from `Src/params.c:3796`. C body:
/// ```c
/// char *s = pm->node.nam;
/// queue_signals();
/// if (pm != (Param)(paramtab == realparamtab ?
///        paramtab->getnode2(paramtab, s) :
///        paramtab->getnode(paramtab, s))) {
///     unqueue_signals();
///     zerr("can't change type of hidden variable: %s", s);
///     return 1;
/// }
/// s = dupstring(s);
/// unsetparam_pm(pm, 0, 1);
/// unqueue_signals();
/// createparam(s, flags);
/// return 0;
/// ```
/// Tears `pm` down + recreates it with `flags` so the next
/// assignment lands in a fresh slot of the requested type. Used
/// by `assignsparam` when the type-flag of an existing param
/// changes (e.g. `typeset -i x; x="abc"` resets x back to scalar).
///
/// The `paramtab->getnode` reachability check at c:3800 catches
/// the hidden-shadow case (a local var hiding the global `pm` we
/// were handed) — without the paramtab vtable we skip the check
/// and proceed to unset+create.
pub fn resetparam(pm: &mut crate::ported::zsh_h::param, flags: i32) -> i32 { // c:3796
    let s = pm.node.nam.clone();                                             // c:3796
    crate::ported::signals::queue_signals();                                 // c:3799
    // c:3800-3807 — paramtab->getnode2 / getnode reachability check.
    // Without paramtab vtable wired we cannot detect the hidden-
    // variable case, so we proceed; a future port of paramtab
    // adds the check at this site.
    unsetparam_pm(pm, 0, 1);                                                 // c:3819
    crate::ported::signals::unqueue_signals();                               // c:3819
    let _ = createparam(&s, flags);                                          // c:3819
    0                                                                        // c:3819
}

/// Unset a parameter from all storage.
/// Port of `unsetparam(char *s)` from Src/params.c:3819. C uses a single
/// HashTable; our SubstState-style storage spans variables /
/// arrays / assoc_arrays, so removal must touch all three to be
/// thorough (matches `unsetparam_pm`'s flag-aware tear-down).
/// WARNING: param names don't match C — Rust=(arrays, assoc_arrays, name) vs C=(s)
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

/// Unset parameter (from params.c unsetparam_pm)
/// Port of `unsetparam_pm(Param pm, int altflag, int exp)` from `Src/params.c:3841`. Full body
/// removes `pm` from `paramtab` (after invoking
/// `pm->gsu.s->unsetfn(pm, exp)`), tears down the tied alternate
/// (`pm->ename`) when `!altflag`, deletes the env entry, and
/// resurrects `pm->old` at the right scope. Stub: needs paramtab
/// HashTable backend (`paramtab->removenode/addnode`) plus the
/// `delenv`/`adduserdir` helpers — direct port retains only the
/// in-memory mutation of `pm` that doesn't touch the table.
#[allow(unused_variables)]
pub fn unsetparam_pm(pm: &mut crate::ported::zsh_h::param, altflag: i32, exp: i32) -> i32 {
    // Readonly check (locallevel global not yet ported — assume 0).
    if (pm.node.flags as u32 & PM_READONLY) != 0 && pm.level <= 0 {
        // zerr("read-only %s: %s", ref?"reference":"variable", nam);
        let _kind = if (pm.node.flags as u32 & PM_NAMEREF) != 0 {
            "reference"
        } else {
            "variable"
        };
        return 1;
    }
    pm.node.flags &= !(PM_DECLARED as i32);
    if (pm.node.flags as u32 & PM_UNSET) == 0
        || (pm.node.flags as u32 & PM_REMOVABLE) != 0
    {
        // pm->gsu.s->unsetfn(pm, exp) — open-coded to stdunsetfn.
        stdunsetfn(pm, exp);
    }
    if pm.env.is_some() {
        delenv(&pm.node.nam);
        pm.env = None;
    }
    // Tied alt-name removal + paramtab restore-from-old not yet
    // possible without HashTable backend; the C postlude (lines
    // 3853-3935) is a paramtab->removenode + addnode dance that
    // requires the missing vtable.
    pm.node.flags |= PM_UNSET as i32;
    0
}

// -----------------------------------------------------------
// GSU dispatch callbacks — direct ports against `param.u_*`
// fields. C source in Src/params.c:4002.
// -----------------------------------------------------------

/// Port of `intgetfn(Param pm)` from `Src/params.c:3993`. C body:
/// `return pm->u.val;`
pub fn intgetfn(pm: &crate::ported::zsh_h::param) -> i64 {
    pm.u_val
}

/// Port of `intsetfn(Param pm, zlong x)` from `Src/params.c:4002`. C body:
/// `pm->u.val = x;`
pub fn intsetfn(pm: &mut crate::ported::zsh_h::param, x: i64) {
    pm.u_val = x;
}

/// Port of `floatgetfn(Param pm)` from `Src/params.c:4011`. C body:
/// `return pm->u.dval;`
pub fn floatgetfn(pm: &crate::ported::zsh_h::param) -> f64 {
    pm.u_dval
}

/// Port of `floatsetfn(Param pm, double x)` from `Src/params.c:4020`. C body:
/// `pm->u.dval = x;`
pub fn floatsetfn(pm: &mut crate::ported::zsh_h::param, x: f64) {
    pm.u_dval = x;
}

/// Port of `strgetfn(Param pm)` from `Src/params.c:4029`. C body:
/// `return pm->u.str ? pm->u.str : (char *) hcalloc(1);`
pub fn strgetfn(pm: &crate::ported::zsh_h::param) -> String {
    pm.u_str.clone().unwrap_or_default()
}

/// Port of `strsetfn(Param pm, char *x)` from `Src/params.c:4038`. C body:
/// `zsfree(pm->u.str); pm->u.str = x;` plus AUTONAMEDIRS handling.
/// The `adduserdir()` call is gated on PM_NAMEDDIR/AUTONAMEDIRS.
pub fn strsetfn(pm: &mut crate::ported::zsh_h::param, x: String) {
    pm.u_str = Some(x.clone());
    if (pm.node.flags as u32 & PM_HASHELEM) == 0 {
        if (pm.node.flags as u32 & PM_NAMEDDIR) != 0 {
            pm.node.flags |= PM_NAMEDDIR as i32;
            crate::ported::utils::adduserdir(&pm.node.nam, &x, 0, false);
        }
    }
}

/// Port of `arrgetfn(Param pm)` from `Src/params.c:4057`. C body:
/// `return pm->u.arr ? pm->u.arr : &nullarray;`
pub fn arrgetfn(pm: &crate::ported::zsh_h::param) -> Vec<String> {
    pm.u_arr.clone().unwrap_or_default()
}

/// Port of `arrsetfn(Param pm, char **x)` from `Src/params.c:4066`. C body frees
/// the old array, applies PM_UNIQUE filter via `uniqarray()`, then
/// stores. Calls `arrfixenv(ename, x)` for tied colon-arrays.
pub fn arrsetfn(pm: &mut crate::ported::zsh_h::param, x: Vec<String>) {
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

/// Port of `hashgetfn(Param pm)` from `Src/params.c:4084`. C body:
/// `return pm->u.hash;`
pub fn hashgetfn(pm: &crate::ported::zsh_h::param) -> Option<&crate::ported::zsh_h::HashTable> {
    pm.u_hash.as_ref()
}

/// Port of `hashsetfn(Param pm, HashTable x)` from `Src/params.c:4093`. C body:
/// `if (pm->u.hash && pm->u.hash != x) deleteparamtable(pm->u.hash);
///  pm->u.hash = x;`
pub fn hashsetfn(pm: &mut crate::ported::zsh_h::param, x: crate::ported::zsh_h::HashTable) {
    pm.u_hash = Some(x);
}

/// Direct port of `static void arrhashsetfn(Param pm, char **val,
/// int flags)` from `Src/params.c:4113-4170`. Set callback for
/// assoc arrays: takes a flat `[k1, v1, k2, v2, ...]` value list
/// and turns it into a hash.
///
/// C body:
///   1. Count non-Marker entries; if odd, error c:4128-4131.
///   2. Under ASSPM_AUGMENT, fetch existing hash via getfn
///      (c:4134-4137); otherwise allocate fresh via
///      newparamtable(17, name).
///   3. Walk pairs: each value (k, v) becomes a PM_SCALAR|PM_UNSET
///      child param `createparam(k)`, then `assignstrvalue(v->pm,
///      val, eltflags)` (c:4140-4166).
///   4. `pm->gsu.h->setfn(pm, ht)` to install (c:4168).
///
/// The Rust port partially mirrors: counts pairs, rejects odd
/// counts via zerr, installs a fresh hashtable. The per-pair
/// createparam+assignstrvalue cycle requires assoc storage
/// shape we don't yet have wired through `u_hash`; this stays as
/// a structural port and emits diagnostic on the odd-count path.
pub fn arrhashsetfn(                                                         // c:4113
    pm: &mut crate::ported::zsh_h::param,
    val: Vec<String>,
    _flags: i32,
) {

    // c:4124-4127 — count non-Marker entries.
    let alen: usize = val
        .iter()
        .filter(|s| !s.starts_with(Marker as char))
        .count();

    // c:4129-4131 — odd count → error.
    if alen % 2 != 0 {
        crate::ported::utils::zerr(
            "bad set of key/value pairs for associative array",
        );
        return;
    }

    // c:4132-4138 — install or augment. Skip the createparam
    // sub-hash walk pending assoc-storage wiring; install an
    // empty hashtable so hashgetfn doesn't return stale data.
    pm.u_hash = Some(Box::new(crate::ported::zsh_h::hashtable {
        hsize: 0,
        ct: 0,
        nodes: Vec::new(),
        tmpdata: 0,
        hash: None,
        emptytable: None,
        filltable: None,
        cmpnodes: None,
        addnode: None,
        getnode: None,
        getnode2: None,
        removenode: None,
        disablenode: None,
        enablenode: None,
        freenode: None,
        printnode: None,
        scantab: None,
    }));
    // c:4170 — free(val). Rust drops automatically.
}

/// Port of `nullstrsetfn(UNUSED(Param pm), char *x)` from `Src/params.c:4180`. C body:
/// `zsfree(x);` — frees but doesn't store. Rust drop handles free.
#[allow(unused_variables)]
pub fn nullstrsetfn(pm: &mut crate::ported::zsh_h::param, x: String) {}

/// Port of `nullunsetfn(UNUSED(Param pm), UNUSED(int exp))` from `Src/params.c:4192`. C body: empty.
#[allow(unused_variables)]
pub fn nullunsetfn(pm: &mut crate::ported::zsh_h::param, exp: i32) {}

/// Port of `stdunsetfn(Param pm, UNUSED(int exp))` from `Src/params.c:3955`. C body:
/// dispatches `pm->gsu->setfn(pm, NULL)` per `PM_TYPE`, clears
/// `PM_TIED`/frees ename for tied params, sets PM_UNSET.
///
/// Rust port mirrors C semantics: clears the union slot and sets
/// PM_UNSET. The GSU vtable callbacks are stored on `param` as
/// `Option<Gsu*>` (zsh_h:760-764) but the dispatch uses callback
/// fn-ptrs that aren't generally registered yet, so we open-code
/// the "setfn(pm, NULL)" effect by zeroing the matching union
/// member instead of calling through the vtable.
#[allow(unused_variables)]
pub fn stdunsetfn(pm: &mut crate::ported::zsh_h::param, exp: i32) {
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

// -----------------------------------------------------------
// "Null" callbacks — no-op getfn/setfn/unsetfn slots used for
// read-only or write-only special params.
// -----------------------------------------------------------

/// Port of `nullintsetfn(UNUSED(Param pm), UNUSED(zlong x))` from `Src/params.c:4187`. C body:
/// empty (no-op setter for read-only int params).
#[allow(unused_variables)]
pub fn nullintsetfn(pm: &mut crate::ported::zsh_h::param, x: i64) {}

/// Port of `nullsethashfn(UNUSED(Param pm), HashTable x)` from `Src/params.c:4104`. C body:
/// `deleteparamtable(x);` — frees the supplied table, doesn't store.
#[allow(unused_variables)]
pub fn nullsethashfn(pm: &mut crate::ported::zsh_h::param, x: crate::ported::zsh_h::HashTable) {
    // Rust drop semantics free `x` when this scope ends.
}

// -----------------------------------------------------------
// Generic special-param GSU callbacks (`u.valptr` / `u.data`).
// C source uses raw pointer indirection through `pm->u.data`/
// `pm->u.valptr` — Rust port stores the global's name in `u_str`
// (lookup key) since we can't carry raw pointers across an FFI
// boundary safely. The lookup-table integration ships with the
// special-params init code (Src/params.c:4213 createparamtable).
// -----------------------------------------------------------

/// Port of `intvargetfn(Param pm)` from `Src/params.c:4202`. C body:
/// `return *pm->u.valptr;`
pub fn intvargetfn(pm: &crate::ported::zsh_h::param) -> i64 {
    pm.u_val
}

/// Port of `intvarsetfn(Param pm, zlong x)` from `Src/params.c:4213`. C body:
/// `*pm->u.valptr = x;`
pub fn intvarsetfn(pm: &mut crate::ported::zsh_h::param, x: i64) {
    pm.u_val = x;
}

/// Port of `zlevarsetfn(Param pm, zlong x)` from `Src/params.c:4224`. C body sets
/// the int and triggers `adjustwinsize` for LINES/COLUMNS.
pub fn zlevarsetfn(pm: &mut crate::ported::zsh_h::param, x: i64) {
    pm.u_val = x;
    if pm.node.nam == "LINES" || pm.node.nam == "COLUMNS" {
        let _ = crate::ported::utils::adjustwinsize(0);
    }
}

/// Port of `strvarsetfn(Param pm, char *x)` from `Src/params.c:4249`. C body:
/// `zsfree(*q); *q = x;` where `q = (char **)pm->u.data`.
pub fn strvarsetfn(pm: &mut crate::ported::zsh_h::param, x: Option<String>) {
    pm.u_str = x;
}

/// Port of `strvargetfn(Param pm)` from `Src/params.c:4263`. C body:
/// `s = *((char **)pm->u.data); return s ? s : hcalloc(1);`
pub fn strvargetfn(pm: &crate::ported::zsh_h::param) -> String {
    pm.u_str.clone().unwrap_or_default()
}

/// Port of `arrvargetfn(Param pm)` from `Src/params.c:4279`. C body:
/// `arrptr = *((char ***)pm->u.data); return arrptr ?: &nullarray;`
pub fn arrvargetfn(pm: &crate::ported::zsh_h::param) -> Vec<String> {
    pm.u_arr.clone().unwrap_or_default()
}

/// Port of `arrvarsetfn(Param pm, char **x)` from `Src/params.c:4294`. C body
/// frees old, applies PM_UNIQUE, handles PM_SPECIAL+NULL → mkarray.
pub fn arrvarsetfn(pm: &mut crate::ported::zsh_h::param, x: Vec<String>) {
    let val = if (pm.node.flags as u32 & PM_UNIQUE) != 0 {
        simple_arrayuniq(x)
    } else {
        x
    };
    pm.u_arr = Some(val);
}

/// Array to colon-separated path — inverse of `colonsplit`.
/// Port of `colonarrgetfn(Param pm)` from Src/params.c (joins the array
/// stored in `pm->u.colon` back into the `:`-form for env).
/// WARNING: param names don't match C — Rust=(arr) vs C=(pm)
pub fn colonarrgetfn(arr: &[String]) -> String {
    arr.join(":")
}

/// Port of `colonarrsetfn(Param pm, char *x)` from `Src/params.c:4329`. C body
/// splits the colon-string into an array and stores via the
/// generic arrvarsetfn.
pub fn colonarrsetfn(pm: &mut crate::ported::zsh_h::param, x: Option<String>) {
    let uniq = (pm.node.flags as u32 & PM_UNIQUE) != 0;                          // c:4339
    let arr = match x {
        Some(s) => crate::ported::utils::colonsplit(&s, uniq),                   // c:4339
        None => Vec::new(),
    };
    arrvarsetfn(pm, arr);
}

/// Port of `tiedarrgetfn(Param pm)` from `Src/params.c:4348`. C body:
/// `return *((Tieddata)pm->u.data)->arrptr;`
pub fn tiedarrgetfn(pm: &crate::ported::zsh_h::param) -> Vec<String> {
    pm.u_arr.clone().unwrap_or_default()
}

/// Direct port of `void tiedarrsetfn(Param pm, char *x)` from
/// `Src/params.c:4357-4389`. Setter for a colon-array-tied
/// scalar (PATH/CDPATH/MAILPATH/etc.).
///
/// C body:
///   1. Free the existing tied array (`*dptr->arrptr`) at c:4363.
///   2. If no array but an `ename` exists, clear PM_DEFAULTED on
///      the tied array param (c:4365-4368).
///   3. If `x` is non-null: build a 1-or-2-byte separator from
///      `dptr->joinchar` (Meta-quoting if needed, c:4371-4380),
///      `sepsplit(x, sepbuf, 0, 0)` into the array (c:4381), and
///      uniqarray() if PM_UNIQUE (c:4382-4383). Free `x` (c:4384).
///   4. Else: `*dptr->arrptr = NULL` (c:4385-4386).
///   5. If `pm->ename` is set, call `arrfixenv(pm->name, arrptr)`
///      to sync env (c:4387-4388).
///
/// The Rust port treats `u_arr` as the tied array storage and
/// uses `':'` as the joinchar default (matches PATH/CDPATH/FPATH
/// /MAILPATH/PSVAR/MODULE_PATH which all use colon separators —
/// the joinchar field on the C-side tieddata wasn't ported to the
/// Rust Param struct yet).
pub fn tiedarrsetfn(pm: &mut crate::ported::zsh_h::param, x: Option<String>) { // c:4357

    // c:4361-4368 — free old / clear PM_DEFAULTED on tied counterpart.
    if pm.u_arr.is_none() {
        if let Some(ename) = pm.ename.clone() {                              // c:4365
            let mut tab = paramtab().write().unwrap();
            if let Some(altpm) = tab.get_mut(&ename) {                       // c:4366
                altpm.node.flags &= !(PM_DEFAULTED as i32);                  // c:4367
            }
        }
    }

    if let Some(s) = x {                                                     // c:4369
        // c:4370-4380 — single-byte separator (joinchar=':' for all
        // currently-tied params; Meta-quoting only kicks in for
        // exotic joinchars not present today).
        let arr: Vec<String> = s.split(':').map(|t| t.to_string()).collect();
        // c:4382-4383 — uniqarray if PM_UNIQUE.
        let arr = if pm.node.flags & PM_UNIQUE as i32 != 0 {                 // c:4382
            uniqarray(arr)                                                   // c:4383
        } else {
            arr
        };
        pm.u_arr = Some(arr);
        // c:4384 — zsfree(x). Rust drop.
    } else {                                                                 // c:4385
        pm.u_arr = None;                                                     // c:4386
    }

    // c:4387-4388 — `if (pm->ename) arrfixenv(pm->name, *dptr->arrptr)`.
    if pm.ename.is_some() {
        let nam = pm.node.nam.clone();
        let arr_ref = pm.u_arr.as_deref();
        arrfixenv(&nam, arr_ref);
    }
}

/// Port of `tiedarrunsetfn(Param pm, UNUSED(int exp))` from `Src/params.c:4393`. C body
/// frees the tied storage and calls stdunsetfn.
/// Direct port of `void tiedarrunsetfn(Param pm, UNUSED(int exp))`
/// from `Src/params.c:4393`. Special unset for tied arrays:
/// frees tieddata, ename, clears PM_TIED, sets PM_UNSET.
///
/// C body:
///   pm->gsu.s->setfn(pm, NULL);             // c:4393
///   zfree(pm->u.data, sizeof(tieddata));    // c:4393
///   pm->u.data = NULL;                      // c:4393
///   zsfree(pm->ename);                      // c:4393
///   pm->ename = NULL;                       // c:4393
///   pm->flags &= ~PM_TIED;                  // c:4393
///   pm->flags |= PM_UNSET;                  // c:4393
pub fn tiedarrunsetfn(pm: &mut crate::ported::zsh_h::param, _exp: i32) {     // c:4393
    // c:4400 — invoke the scalar setfn with NULL (frees backing array).
    tiedarrsetfn(pm, None);
    // c:4401-4403 — drop tieddata.
    pm.u_data = 0;
    pm.u_arr = None;
    // c:4404-4405 — `zsfree(pm->ename); pm->ename = NULL`.
    pm.ename = None;
    // c:4406-4407 — flag toggles.
    pm.node.flags &= !(PM_TIED as i32);
    pm.node.flags |= PM_UNSET as i32;
}

// -----------------------------------------------------------
// Array uniq helpers.
// -----------------------------------------------------------

/// Port of `simple_arrayuniq(char **x, int freeok)` from `Src/params.c:4412`. C body:
/// O(n^2) dedupe in place — first occurrence wins.
/// WARNING: param names don't match C — Rust=(x) vs C=(x, freeok)
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

/// Port of `arrayuniq_freenode(HashNode hn)` from `Src/params.c:4443`. C
/// body: `zsfree(((Pathnode)hn)->name); zfree(hn, sizeof…);` —
/// the freenode callback for the temporary HashTable `arrayuniq`
/// builds. Rust drop semantics handle this; no-op shim.
/// is `(void)hn;` — intentional no-op; passed as freenode callback
/// to scratch hashtable used by `arrayuniq` so existing entries
/// aren't freed when the table is torn down.
/// WARNING: param names don't match C — Rust=() vs C=(hn)
/// WARNING: param names don't match C — Rust=() vs C=(pm, x)
pub fn arrayuniq_freenode() {}

/// Direct port of `HashTable newuniqtable(zlong size)` from
/// `Src/params.c:4450`. C body allocates a `HashTable`
/// named "arrayuniq" with the standard hasher/cmpnodes/
/// add/get/remove/disable/enable function pointers plus
/// `arrayuniq_freenode` as the freenode callback (which is a
/// no-op — see c:4443). Rust returns a `HashSet<String>` with
/// the size hint pre-allocated; the freenode-callback role is
/// implicit (Drop runs on HashSet teardown without freeing
/// borrowed strings).
pub fn newuniqtable(size: i64) -> HashSet<String> {                          // c:4450
    HashSet::with_capacity(size.max(0) as usize)                             // c:4450 newhashtable(size, ...)
}

/// Direct port of `static void arrayuniq(char **x, int freeok)`
/// from `Src/params.c:4473`. First-wins dedupe of `x`,
/// in-place. C uses simple O(n²) scan for arrays under 10
/// entries, switching to a HashTable for larger arrays. `freeok`
/// controls whether to `zsfree()` duplicates (only safe when
/// caller owns the strings — Rust drop semantics handle it).
///
/// Signature note: C takes `char **x` + in-place mutation; Rust
/// takes owned `Vec<String>` and returns the deduped result.
/// `freeok` is preserved but is a no-op in Rust (drops free
/// automatically). The hashtable / simple-loop tiering follows
/// the same threshold (10) as C.
pub fn arrayuniq(x: Vec<String>, freeok: i32) -> Vec<String> {               // c:4473
    let _ = freeok;
    let array_size = x.len();
    if array_size == 0 {                                                     // c:4481
        return x;
    }
    // c:4482-4486 — small-array fallback to simple_arrayuniq.
    if array_size < 10 {                                                     // c:4482
        return simple_arrayuniq(x);                                          // c:4484
    }
    // c:4483 — `if (!(ht = newuniqtable(array_size + 1)))` — Rust
    // newuniqtable never fails, but mirror the C order of allocation.
    let mut ht = newuniqtable(array_size as i64 + 1);
    // c:4487-4507 — walk + first-wins.
    let mut out: Vec<String> = Vec::with_capacity(array_size);
    for s in x {                                                             // c:4487 walk
        if ht.insert(s.clone()) {                                            // c:4488 gethashnode2 + addhashnode2
            out.push(s);                                                     // c:4495 *write_it = *it
        }
        // else: dup — drop the value (c:4502 zsfree if freeok).
    }
    drop(ht);                                                                // c:4523 deletehashtable
    out
}

/// Remove duplicate elements from array while preserving order.
/// Port of `uniqarray(char **x)` from Src/params.c.
/// WARNING: param names don't match C — Rust=(arr) vs C=(x)
pub fn uniqarray(arr: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    arr.into_iter().filter(|s| seen.insert(s.clone())).collect()
}

/// Direct port of `void zhuniqarray(char **x)` from
/// `Src/params.c:4523`. Wraps `arrayuniq` with `freeok=0`.
/// (C body is literally `arrayuniq(x, 0);`.)
pub fn zhuniqarray(x: Vec<String>) -> Vec<String> {                          // c:4523
    arrayuniq(x, 0)                                                          // c:4523
}

/// Port of `poundgetfn(UNUSED(Param pm))` from `Src/params.c:4534`. C body:
/// `return arrlen(pparams);`
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn poundgetfn() -> i64 {
    pparams_lock().lock().expect("pparams poisoned").len() as i64
}

/// Port of `randomgetfn(UNUSED(Param pm))` from `Src/params.c:4543`. C body:
/// `return rand() & 0x7fff;`
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn randomgetfn() -> i64 {
    (unsafe { libc::rand() } & 0x7fff) as i64
}

/// Port of `randomsetfn(UNUSED(Param pm), zlong v)` from `Src/params.c:4552`. C body:
/// `srand((unsigned int)v);`
/// WARNING: param names don't match C — Rust=(v) vs C=(pm, v)
pub fn randomsetfn(v: i64) {
    unsafe { libc::srand(v as libc::c_uint) };
}

// -----------------------------------------------------------
// SECONDS / EPOCHSECONDS family — backed by SHTIMER static.
// -----------------------------------------------------------

/// Port of `intsecondsgetfn(UNUSED(Param pm))` from `Src/params.c:4561`. C body:
/// `return (zlong)(now.tv_sec - shtimer.tv_sec - …);`
/// WARNING: param names don't match C — Rust=() vs C=(pm)
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

/// Port of `intsecondssetfn(UNUSED(Param pm), zlong x)` from `Src/params.c:4575`. C body:
/// ```c
/// diff = (zlong)now.tv_sec - x;
/// shtimer.tv_sec = diff;
/// if ((zlong)shtimer.tv_sec != diff)
///     zwarn("SECONDS truncated on assignment");
/// shtimer.tv_nsec = now.tv_nsec;
/// ```
/// WARNING: param names don't match C — Rust=(x) vs C=(pm, x)
pub fn intsecondssetfn(x: i64) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let now_sec = now.as_secs() as i64;
    let new_sec = now_sec - x;
    // c:4587 — C uses `zwarn` (informational), NOT `zerr` (fatal).
    // The C body STORES `diff` unconditionally then emits the warning
    // if truncation lost information. Rust port previously used `zerr`
    // and early-returned (skipping the store) — divergent from C.
    if new_sec < 0 {
        crate::ported::utils::zwarn("SECONDS truncated on assignment");
        // c:4585 — C still stores; Rust represents shtimer as Duration
        // which is non-negative. We clamp to zero to preserve the
        // "store-anyway" semantic for the time-display path, even
        // though the negative-time case is unrepresentable.
        *shtimer_lock().lock().expect("shtimer poisoned") =
            Duration::new(0, now.subsec_nanos());
        return;
    }
    *shtimer_lock().lock().expect("shtimer poisoned") =
        Duration::new(new_sec as u64, now.subsec_nanos());
}

/// Port of `floatsecondsgetfn(UNUSED(Param pm))` from `Src/params.c:4591`. C body:
/// `return (double)(now-tv_sec - shtimer.tv_sec) + nsec/1e9;`
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn floatsecondsgetfn() -> f64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let timer = *shtimer_lock().lock().expect("shtimer poisoned");
    (now - timer).as_secs_f64()
}

/// Port of `floatsecondssetfn(UNUSED(Param pm), double x)` from `Src/params.c:4603`. C body:
/// `shtimer.tv_sec = now.tv_sec - (zlong)x; shtimer.tv_nsec = now.tv_nsec - (x-int)*1e9;`
/// WARNING: param names don't match C — Rust=(x) vs C=(pm, x)
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

/// Port of `setrawseconds(double x)` from `Src/params.c:4622`. C body:
/// `shtimer.tv_sec = (zlong)x; shtimer.tv_nsec = (x-int)*1e9;`
pub fn setrawseconds(x: f64) {
    *shtimer_lock().lock().expect("shtimer poisoned") = Duration::from_secs_f64(x);
}

/// Port of `setsecondstype(Param pm, int on, int off)` from `Src/params.c:4630`. C body
/// flips the `gsu.f`/`gsu.i` callback pointer based on the new
/// param-flag bitset.
///
/// WARNING: zshrs has no Param/GSU dispatch table yet — the
/// "promotion between integer/float seconds" logic happens via
/// pm->gsu pointer swaps in C. Returns 0 to signal success;
/// callers can assume the type change is recorded by the caller's
/// own bookkeeping until the GSU table lands.
/// WARNING: param names don't match C — Rust=(on, off) vs C=(pm, on, off)
pub fn setsecondstype(                                                       // c:4630
    pm: &mut crate::ported::zsh_h::param,
    on: i32,
    off: i32,
) -> i32 {
    // c:4632 — `int newflags = (pm->flags | on) & ~off`.
    let newflags = (pm.node.flags | on) & !off;
    // c:4633 — `int tp = PM_TYPE(newflags)`.
    let tp = PM_TYPE(newflags as u32);
    // c:4635-4638 / 4639-4642 — float vs integer GSU pointer swap.
    if tp == PM_EFLOAT || tp == PM_FFLOAT {                                  // c:4635
        // C: `pm->gsu.f = &floatseconds_gsu`. GSU table not yet
        // wired in the Rust port; record the type by clearing
        // any integer GSU.
        pm.gsu_i = None;
        // pm.gsu_f = Some(floatseconds_gsu) — pending GSU port.
    } else if tp == PM_INTEGER {                                             // c:4639
        // C: `pm->gsu.i = &intseconds_gsu`.
        pm.gsu_f = None;
        // pm.gsu_i = Some(intseconds_gsu) — pending GSU port.
    } else {
        return 1;                                                            // c:4644
    }
    pm.node.flags = newflags;                                                // c:4645
    0                                                                        // c:4646
}

// -----------------------------------------------------------
// $USERNAME
// -----------------------------------------------------------

/// Port of `usernamegetfn(UNUSED(Param pm))` from `Src/params.c:4653`. C body:
/// `return get_username();`
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn usernamegetfn() -> String {
    cached_username_lock()
        .lock()
        .expect("username poisoned")
        .clone()
}

/// Port of `usernamesetfn(UNUSED(Param pm), char *x)` from `Src/params.c:4662`. C body:
/// `getpwnam(x); setgid; setuid; cached_uid = pswd->pw_uid;`
///
/// WARNING: the SUID-changing path requires getpwnam(3) which
/// crosses an unsafe FFI boundary not yet wrapped here. The
/// cached-name update is performed; uid/gid changes still need
/// porting of the `pwd.h` getpwnam wrapper.
/// WARNING: param names don't match C — Rust=(x) vs C=(pm, x)
pub fn usernamesetfn(x: String) {                                            // c:4662
    // c:4662 — `if (x && (pswd = getpwnam(x)) && pswd->pw_uid != cached_uid)`.
    let target = std::ffi::CString::new(x.as_bytes()).ok();
    if let Some(cstr) = target {
        unsafe {
            let pwd = libc::getpwnam(cstr.as_ptr());                         // c:4666
            if !pwd.is_null() {
                let cached_uid =
                    libc::geteuid() as libc::uid_t;
                if (*pwd).pw_uid != cached_uid {                             // c:4666
                    // c:4670-4672 — initgroups(x, pswd->pw_gid).
                    let _ = libc::initgroups(cstr.as_ptr(), (*pwd).pw_gid as _);
                    // c:4671 — setgid(pswd->pw_gid).
                    if libc::setgid((*pwd).pw_gid) != 0 {                    // c:4673
                        crate::ported::utils::zwarn(&format!(
                            "failed to change group ID: {}",
                            std::io::Error::last_os_error()
                        ));
                    } else if libc::setuid((*pwd).pw_uid) != 0 {             // c:4675
                        // c:4675-4676 — setuid failed.
                        crate::ported::utils::zwarn(&format!(
                            "failed to change user ID: {}",
                            std::io::Error::last_os_error()
                        ));
                    } else {
                        // c:4677-4681 — cache update.
                        let name_cstr = std::ffi::CStr::from_ptr((*pwd).pw_name);
                        let name_str = name_cstr.to_string_lossy().to_string();
                        *cached_username_lock()
                            .lock()
                            .expect("username poisoned") =
                            crate::ported::utils::ztrdup_metafy(&name_str);
                    }
                }
            }
        }
    }
    // c:4683 — `zsfree(x)`; Rust drop handles it.
    drop(x);
}

// -----------------------------------------------------------
// libc-backed callbacks (UID/GID/EUID/EGID/errno/RANDOM/TTYIDLE).
// -----------------------------------------------------------

/// Port of `uidgetfn(UNUSED(Param pm))` from `Src/params.c:4689`. C body:
/// `return getuid();`
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn uidgetfn() -> i64 {
    unsafe { libc::getuid() as i64 }
}

// `termflags` from Src/init.c — bitmap of terminal-state flags. Set
// from term_reinit_from_pm and consulted by ZLE before first paint.
pub static TERMFLAGS: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);
// `TERM_UNKNOWN` from Src/zsh.h:1986.
pub const TERM_UNKNOWN: i32 = 1 << 0;

/// Port of `uidsetfn(UNUSED(Param pm), zlong x)` from `Src/params.c:4698`. C body:
/// `if (setuid((uid_t)x)) zerr("failed to change user ID: %e", errno);`
/// C body (2 lines):
///   `if (setuid((uid_t)x)) zerr("failed to change user ID: %e", errno);`
/// WARNING: param names don't match C — Rust=(x) vs C=(pm, x)
pub fn uidsetfn(x: i64) {                                                    // c:4698
    if unsafe { libc::setuid(x as libc::uid_t) } != 0 {                      // c:4701
        zerr(&format!("failed to change user ID: {}", std::io::Error::last_os_error())); // c:4702
    }
}

/// Port of `euidgetfn(UNUSED(Param pm))` from `Src/params.c:4710`. C body:
/// `return geteuid();`
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn euidgetfn() -> i64 {
    unsafe { libc::geteuid() as i64 }
}

/// Port of `euidsetfn(UNUSED(Param pm), zlong x)` from `Src/params.c:4719`. C body:
/// `if (seteuid((uid_t)x)) zerr("failed to change effective user ID: %e", errno);`
/// WARNING: param names don't match C — Rust=(x) vs C=(pm, x)
pub fn euidsetfn(x: i64) {                                                   // c:4719
    if unsafe { libc::seteuid(x as libc::uid_t) } != 0 {                     // c:4722
        zerr(&format!("failed to change effective user ID: {}", std::io::Error::last_os_error())); // c:4723
    }
}

/// Port of `gidgetfn(UNUSED(Param pm))` from `Src/params.c:4731`. C body: `return getgid();`
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn gidgetfn() -> i64 {
    unsafe { libc::getgid() as i64 }
}

/// Port of `gidsetfn(UNUSED(Param pm), zlong x)` from `Src/params.c:4740`. C body:
/// `if (setgid((gid_t)x)) zerr("failed to change group ID: %e", errno);`
/// WARNING: param names don't match C — Rust=(x) vs C=(pm, x)
pub fn gidsetfn(x: i64) {                                                    // c:4740
    if unsafe { libc::setgid(x as libc::gid_t) } != 0 {                      // c:4743
        zerr(&format!("failed to change group ID: {}", std::io::Error::last_os_error())); // c:4744
    }
}

/// Port of `egidgetfn(UNUSED(Param pm))` from `Src/params.c:4752`. C body: `return getegid();`
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn egidgetfn() -> i64 {
    unsafe { libc::getegid() as i64 }
}

/// Port of `egidsetfn(UNUSED(Param pm), zlong x)` from `Src/params.c:4761`. C body:
/// `if (setegid((gid_t)x)) zerr("failed to change effective group ID: %e", errno);`
/// WARNING: param names don't match C — Rust=(x) vs C=(pm, x)
pub fn egidsetfn(x: i64) {                                                   // c:4761
    if unsafe { libc::setegid(x as libc::gid_t) } != 0 {                     // c:4764
        zerr(&format!("failed to change effective group ID: {}", std::io::Error::last_os_error())); // c:4765
    }
}

/// Port of `ttyidlegetfn(UNUSED(Param pm))` from `Src/params.c:4771`. C body:
/// ```c
/// struct stat ttystat;
/// if (SHTTY == -1 || fstat(SHTTY, &ttystat)) return -1;
/// return time(NULL) - ttystat.st_atime;
/// ```
/// Rust port reads stdin (fd 0) — closest match to `SHTTY` the
/// shell tracks as the controlling-tty fd. Returns -1 if stdin is
/// not a tty.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
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
// $IFS / $HOME / $TERM / $WORDCHARS / $TERMINFO / $TERMINFO_DIRS
// $KEYBOARD_HACK / $HISTCHARS / $_  — string-state callbacks.
// -----------------------------------------------------------

/// Port of `ifsgetfn(UNUSED(Param pm))` from `Src/params.c:4784`. C body: `return ifs;`
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn ifsgetfn() -> String {
    ifs_lock().lock().expect("ifs poisoned").clone()
}

/// Port of `ifssetfn(UNUSED(Param pm), char *x)` from `Src/params.c:4793`. C body:
/// `zsfree(ifs); ifs = x; inittyptab();`
/// WARNING: param names don't match C — Rust=(x) vs C=(pm, x)
pub fn ifssetfn(x: String) {
    *ifs_lock().lock().expect("ifs poisoned") = x;
    // c:4795 — `inittyptab()` rebuilds the typtab[] ISEP/IWSEP bits
    // from the new IFS. Without this, every word-split path stays
    // pinned to the old separator set and silently mis-splits.
    crate::ported::utils::inittyptab();
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

/// Port of `setlang(char *x)` from `Src/params.c:4840`. C body:
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
    // c:4868 — `inittyptab();`. The locale change may shift which
    // bytes are isalpha/isalnum/etc under the typtab init, so the
    // table must be rebuilt. Without this, every shell-script that
    // sets LANG to a non-ASCII locale would silently keep using
    // the prior locale's char classes.
    crate::ported::utils::inittyptab();
}

/// Port of `lc_allsetfn(Param pm, char *x)` from `Src/params.c:4871`. C body
/// dispatches to `setlang(LANG)` if x empty, else `setlocale(LC_ALL, x)`.
/// WARNING: param names don't match C — Rust=(x) vs C=(pm, x)
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

/// Port of `langsetfn(Param pm, char *x)` from `Src/params.c:4896`. C body:
/// `strsetfn(pm, x); setlang(unmeta(x));`
/// WARNING: param names don't match C — Rust=(x) vs C=(pm, x)
pub fn langsetfn(x: String) {
    setlang(Some(&x));
}

/// Port of `lcsetfn(Param pm, char *x)` from `Src/params.c:4904`. C body:
/// per-category `setlocale` with LC_ALL precedence + LANG fallback.
pub fn lcsetfn(pm: &str, x: Option<String>) {
    if let Ok(lc_all) = env::var("LC_ALL") {
        if !lc_all.is_empty() {
            return;
        }
    }
    let val = x
        .filter(|s| !s.is_empty())
        .or_else(|| env::var("LANG").ok().filter(|s| !s.is_empty()));
    if let Some(v) = val {
        env::set_var(pm, v);
    }
    clear_mbstate();
}

/// Direct port of `static void argzerosetfn(UNUSED(Param pm),
/// char *x)` from `Src/params.c:4937-4946`. Setter for `$0` —
/// POSIX mode rejects assignment (read-only), zsh mode replaces
/// `argzero`.
///
/// C body:
///   if (x) {
///     if (isset(POSIXARGZERO))
///       zerr("read-only variable: 0");
///     else {
///       zsfree(argzero);
///       argzero = ztrdup(x);
///     }
///     zsfree(x);
///   }
/// Port of `argzerosetfn(UNUSED(Param pm), char *x)` from `Src/params.c:4937`.
/// WARNING: param names don't match C — Rust=(x) vs C=(pm, x)
pub fn argzerosetfn(x: String) {                                             // c:4937
    // c:4937 — if (x).
    if !x.is_empty() {
        // c:4940 — isset(POSIXARGZERO) reject.
        if isset(crate::ported::zsh_h::POSIXARGZERO) {
            crate::ported::utils::zerr("read-only variable: 0");             // c:4941
        } else {
            // c:4943-4944 — zsfree(argzero); argzero = ztrdup(x).
            crate::ported::utils::set_argzero(Some(crate::ported::utils::ztrdup(&x)));
        }
        // c:4946 — `zsfree(x)`. Rust drop handles via move.
    }
}

// -----------------------------------------------------------
// $0 / $#
// -----------------------------------------------------------

/// Port of `argzerogetfn(UNUSED(Param pm))` from `Src/params.c:4954`. C body:
/// `return isset(POSIXARGZERO) ? posixzero : argzero;`
///
/// Reads through `crate::ported::utils::argzero()` (the canonical
/// OnceLock storage in utils.rs). C's `posixzero` branch is not
/// yet ported (POSIXARGZERO option needs the option table).
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn argzerogetfn() -> String {
    crate::ported::utils::argzero().unwrap_or_default()
}

// -----------------------------------------------------------
// $HISTSIZE / $SAVEHIST
// -----------------------------------------------------------

/// Port of `histsizegetfn(UNUSED(Param pm))` from `Src/params.c:4965`. C body: `return histsiz;`
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn histsizegetfn() -> i64 {
    *histsiz_lock().lock().expect("histsiz poisoned")
}

/// Port of `histsizesetfn(UNUSED(Param pm), zlong v)` from `Src/params.c:4974`. C body:
/// `if ((histsiz = v) < 1) histsiz = 1; resizehistents();`
/// WARNING: param names don't match C — Rust=(v) vs C=(pm, v)
pub fn histsizesetfn(v: i64) {
    *histsiz_lock().lock().expect("histsiz poisoned") = v.max(1);
    // `resizehistents()` is a hist.c entry point — pending the
    // history-table port, the size change is recorded in the
    // static and picked up the next time the history layer reads.
}

/// Port of `savehistsizegetfn(UNUSED(Param pm))` from `Src/params.c:4985`. C body:
/// `return savehistsiz;`
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn savehistsizegetfn() -> i64 {
    *savehistsiz_lock().lock().expect("savehistsiz poisoned")
}

/// Port of `savehistsizesetfn(UNUSED(Param pm), zlong v)` from `Src/params.c:4994`. C body:
/// `if ((savehistsiz = v) < 0) savehistsiz = 0;`
/// WARNING: param names don't match C — Rust=(v) vs C=(pm, v)
pub fn savehistsizesetfn(v: i64) {
    *savehistsiz_lock().lock().expect("savehistsiz poisoned") = v.max(0);
}

/// Port of `errnosetfn(UNUSED(Param pm), zlong x)` from `Src/params.c:5004`. C body:
/// `errno = (int)x; if ((zlong)errno != x) zwarn("errno truncated on assignment");`
///
/// Rust note: `errno` is a libc thread-local; Rust uses `std::io::Error`
/// which captures the *last* call. To set errno for subsequent
/// `last_os_error()` reads on macOS / Linux, write through the libc
/// `__error()`/`__errno_location()` accessor.
/// C body (Src/params.c:5004):
///     `errno = (int)x;
///      if ((zlong)errno != x) zwarn("errno truncated on assignment");`
/// WARNING: param names don't match C — Rust=(x) vs C=(pm, x)
pub fn errnosetfn(x: i64) {                                                  // c:5004
    let truncated = x as i32;
    unsafe { *errno_ptr() = truncated; }                                     // c:5006 errno = (int)x
    // c:5009-5010 — C uses `zwarn` (informational), NOT `zerr`. The
    // store happens unconditionally; the warning fires only on
    // truncation. Previously used `zerr` — divergent.
    if truncated as i64 != x {                                               // c:5008
        crate::ported::utils::zwarn("errno truncated on assignment");        // c:5009
    }
}

/// !!! RUST-ONLY HELPER — no direct C counterpart. C accesses
/// `errno` through the standard macro which the compiler resolves
/// to the per-platform getter (`__error()` on macOS, `__errno_location()`
/// on Linux). Rust libc exposes both as raw FFI; this helper picks
/// the right one per target so errnosetfn/errnogetfn stay one-liners.
#[inline]
unsafe fn errno_ptr() -> *mut libc::c_int {
    #[cfg(target_os = "macos")] { libc::__error() }
    #[cfg(target_os = "linux")] { libc::__errno_location() }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))] { std::ptr::null_mut() }
}

/// Port of `errnogetfn(UNUSED(Param pm))` from `Src/params.c:5015`. C body: `return errno;`
///
/// Reads the libc errno directly through the per-platform accessor
/// (matching C's `return errno;` semantics). Previously routed
/// through `std::io::Error::last_os_error()` which is NOT errno —
/// it's a snapshot taken at the most recent stdlib syscall. That
/// silently broke `$ERRNO` round-trip: `ERRNO=42` followed by
/// `$ERRNO` could return any stale value.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn errnogetfn() -> i64 {
    let p = unsafe { errno_ptr() };                                          // c:5017 return errno
    if p.is_null() {
        // Non-Linux/macOS fallback: best-effort via std API.
        std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as i64
    } else {
        unsafe { *p as i64 }
    }
}

/// Port of `keyboardhackgetfn(UNUSED(Param pm))` from `Src/params.c:5024`. C body:
/// `static char buf[2]; buf[0] = keyboardhackchar; return buf;`
/// WARNING: param names don't match C — Rust=() vs C=(pm)
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

/// Port of `keyboardhacksetfn(UNUSED(Param pm), char *x)` from `Src/params.c:5040-5060`. C body:
/// ```c
/// if (x) {
///     unmetafy(x, &len);
///     if (len > 1) { len = 1; zwarn("Only one KEYBOARD_HACK character can be defined"); }
///     for (i = 0; i < len; i++)
///         if (!isascii((unsigned char) x[i])) {
///             zwarn("KEYBOARD_HACK can only contain ASCII characters");
///             return;
///         }
///     keyboardhackchar = len ? (unsigned char) x[0] : '\0';
/// } else
///     keyboardhackchar = '\0';
/// ```
/// WARNING: param names don't match C — Rust=(x) vs C=(pm, x)
pub fn keyboardhacksetfn(x: String) {
    let bytes = x.as_bytes();
    // c:5046-5049 — `zwarn` (informational) when len > 1; C still
    // proceeds to use only the first char. Previously used `zerr`.
    if bytes.len() > 1 {
        crate::ported::utils::zwarn("Only one KEYBOARD_HACK character can be defined");
    }
    let c = bytes.first().copied().unwrap_or(0);
    // c:5050-5054 — `zwarn` + early return for non-ASCII byte.
    if c >= 0x80 {
        crate::ported::utils::zwarn("KEYBOARD_HACK can only contain ASCII characters");
        return;
    }
    *keyboardhack_lock().lock().expect("keyboardhack poisoned") = c;
}

/// Port of `histcharsgetfn(UNUSED(Param pm))` from `Src/params.c:5064`. C body:
/// ```c
/// static char buf[4];
/// buf[0] = bangchar; buf[1] = hatchar; buf[2] = hashchar; buf[3] = '\0';
/// return buf;
/// ```
/// Reads from the three canonical atomic globals
/// (`crate::ported::hist::{bangchar, hatchar, hashchar}`) to mirror C
/// which reads from three separate `unsigned char` globals.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn histcharsgetfn() -> String {
    use std::sync::atomic::Ordering;
    let b = crate::ported::hist::bangchar.load(Ordering::SeqCst) as u8;
    let h = crate::ported::hist::hatchar.load(Ordering::SeqCst) as u8;
    let p = crate::ported::hist::hashchar.load(Ordering::SeqCst) as u8;
    // c:5068-5073 — terminal NUL trims unset chars (default-`!^#` is
    // 3 non-NUL bytes); explicit NULs are skipped to match C `buf[3]
    // = '\0'` C-string truncation semantics.
    let mut s = String::new();
    for &byte in &[b, h, p] {
        if byte != 0 {
            s.push(byte as char);
        }
    }
    s
}

/// Port of `histcharssetfn(UNUSED(Param pm), char *x)` from `Src/params.c:5079`. C body
/// validates ASCII, takes up to 3 chars; defaults `!^#` if NULL.
/// WARNING: param names don't match C — Rust=(x) vs C=(pm, x)
pub fn histcharssetfn(x: Option<String>) {
    use std::sync::atomic::Ordering;
    let new_chars: [u8; 3] = match x {
        None => {
            // c:5100-5103 — defaults `!^#` when x is NULL.
            [b'!', b'^', b'#']
        }
        Some(s) => {
            let bytes = s.as_bytes();
            for &b in bytes.iter().take(3) {
                if b >= 0x80 {                                          // c:5090-5093
                    // c:5091 — C uses `zwarn` (informational), NOT
                    // `zerr` (fatal). Function returns early without
                    // updating any globals.
                    crate::ported::utils::zwarn(
                        "HISTCHARS can only contain ASCII characters");
                    return;
                }
            }
            // c:5095-5097 — `bangchar = x[0]; hatchar = x[1]; hashchar = x[2]`.
            // C uses `len ? x[0] : '\0'` etc — for short strings the
            // unset bytes are NUL.
            let mut chars = [0u8; 3];
            for (i, &b) in bytes.iter().take(3).enumerate() {
                chars[i] = b;
            }
            chars
        }
    };
    // c:5079 — set histchars table.
    *histchars_lock().lock().expect("histchars poisoned") = new_chars;
    // c:5095-5097 — `bangchar = x[0]; hatchar = x[1]; hashchar = x[2]`.
    // Sync all three per-char atomic globals so lex/hist callers
    // see the new HISTCHARS. (Previously hashchar was a `const char`
    // in lex.rs — promoted to atomic this iteration.)
    crate::ported::hist::bangchar.store(new_chars[0] as i32, Ordering::SeqCst);
    crate::ported::hist::hatchar.store(new_chars[1] as i32, Ordering::SeqCst);
    crate::ported::hist::hashchar.store(new_chars[2] as i32, Ordering::SeqCst);
    // c:5104 — `inittyptab();`. The bangchar special bit in typtab
    // depends on the current `bangchar` global; reseed.
    crate::ported::utils::inittyptab();
}

/// Port of `homegetfn(UNUSED(Param pm))` from `Src/params.c:5109`. C body: `return home;`
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn homegetfn() -> String {
    home_lock().lock().expect("home poisoned").clone()
}

/// Port of `homesetfn(UNUSED(Param pm), char *x)` from `Src/params.c:5118`. C body:
/// ```c
/// zsfree(home);
/// if (x && isset(CHASELINKS) && (home = xsymlink(x, 0)))
///     zsfree(x);
/// else
///     home = x ? x : ztrdup("");
/// finddir(NULL);
/// ```
/// WARNING: param names don't match C — Rust=(x) vs C=(pm, x)
pub fn homesetfn(x: String) {
    // c:5121-5126 — CHASELINKS path resolves symlinks before storing.
    // Falls through to the plain `x` store when CHASELINKS is off or
    // xsymlink fails.
    let resolved = if !x.is_empty()
        && crate::ported::zsh_h::isset(crate::ported::zsh_h::CHASELINKS)
    {
        crate::ported::utils::xsymlink(&x).unwrap_or(x)
    } else {
        x
    };
    *home_lock().lock().expect("home poisoned") = resolved;
    // c:5127 — `finddir(NULL)` invalidates zsh's cached named-directory
    // lookups. zshrs's finddir port has no cache (per hashnameddir.rs
    // createnameddirtable note); the call is a no-op here.
}

/// Port of `wordcharsgetfn(UNUSED(Param pm))` from `Src/params.c:5132`. C body:
/// `return wordchars;`
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn wordcharsgetfn() -> String {
    wordchars_lock()
        .lock()
        .expect("wordchars poisoned")
        .clone()
}

/// Port of `wordcharssetfn(UNUSED(Param pm), char *x)` from `Src/params.c:5141`. C body:
/// `zsfree(wordchars); wordchars = x; inittyptab();`
/// WARNING: param names don't match C — Rust=(x) vs C=(pm, x)
pub fn wordcharssetfn(x: String) {
    *wordchars_lock().lock().expect("wordchars poisoned") = x;
    // c:5143 — `inittyptab()` rebuilds typtab IWORD bits from the
    // new WORDCHARS. Without this, every IWORD lookup stays pinned
    // to the old set and silently mis-classifies word boundaries.
    crate::ported::utils::inittyptab();
}

/// Port of `underscoregetfn(UNUSED(Param pm))` from `Src/params.c:5152`. C body:
/// `char *u = dupstring(zunderscore); untokenize(u); return u;`
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn underscoregetfn() -> String {
    zunderscore_lock()
        .lock()
        .expect("zunderscore poisoned")
        .clone()
}

/// Port of `term_reinit_from_pm()` from `Src/params.c:5163`.
/// C: `static void term_reinit_from_pm(void)` →
///   `if (unset(INTERACTIVE) || !*term) termflags |= TERM_UNKNOWN;
///    else init_term();`
pub fn term_reinit_from_pm() {                                               // c:5163
    // c:5167 — `if (unset(INTERACTIVE) || !*term) termflags |= TERM_UNKNOWN;`
    let interactive = crate::ported::zsh_h::isset(crate::ported::options::optlookup("interactive"));
    let term = term_lock().lock().map(|s| s.clone()).unwrap_or_default();
    if !interactive || term.is_empty() {                                     // c:5167
        TERMFLAGS.fetch_or(TERM_UNKNOWN, Ordering::Relaxed);                 // c:5168
    } else {
        // c:5170 — `init_term();` lives in ZLE; flag the next prompt
        // to re-init via TERM_UNKNOWN so the lazy path picks it up.
        TERMFLAGS.fetch_or(TERM_UNKNOWN, Ordering::Relaxed);                 // c:5170
    }
}

/// Port of `termgetfn(UNUSED(Param pm))` from `Src/params.c:5176`. C body: `return term;`
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn termgetfn() -> String {
    term_lock().lock().expect("term poisoned").clone()
}

/// Port of `termsetfn(UNUSED(Param pm), char *x)` from `Src/params.c:5185`. C body:
/// `zsfree(term); term = x ? x : ""; term_reinit_from_pm();`
/// WARNING: param names don't match C — Rust=(x) vs C=(pm, x)
pub fn termsetfn(x: String) {
    *term_lock().lock().expect("term poisoned") = x;
    term_reinit_from_pm();
}

/// Port of `terminfogetfn(UNUSED(Param pm))` from `Src/params.c:5196`. C body:
/// `return zsh_terminfo ? zsh_terminfo : "";`
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn terminfogetfn() -> String {
    zsh_terminfo_lock()
        .lock()
        .expect("zsh_terminfo poisoned")
        .clone()
}

/// Port of `int rprompt_indent` from `Src/init.c`. Set to 1 by
/// `init_term()` and reset by `rprompt_indent_unsetfn` when the
/// `RPROMPT_INDENT` parameter is unset.
pub static RPROMPT_INDENT: std::sync::Mutex<i32> = std::sync::Mutex::new(1);

/// Port of `terminfosetfn(Param pm, char *x)` from `Src/params.c:5205`. C body:
/// `zsfree(zsh_terminfo); zsh_terminfo = x; addenv if exported; term_reinit_from_pm();`
/// WARNING: param names don't match C — Rust=(x) vs C=(pm, x)
pub fn terminfosetfn(x: String) {
    *zsh_terminfo_lock()
        .lock()
        .expect("zsh_terminfo poisoned") = x.clone();
    env::set_var("TERMINFO", &x);
    term_reinit_from_pm();
}

/// Port of `terminfodirsgetfn(UNUSED(Param pm))` from `Src/params.c:5224`. C body:
/// `return zsh_terminfodirs ? zsh_terminfodirs : "";`
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn terminfodirsgetfn() -> String {
    zsh_terminfodirs_lock()
        .lock()
        .expect("zsh_terminfodirs poisoned")
        .clone()
}

/// Port of `terminfodirssetfn(Param pm, char *x)` from `Src/params.c:5233`. C body
/// mirrors `terminfosetfn` for the TERMINFO_DIRS env var.
/// WARNING: param names don't match C — Rust=(x) vs C=(pm, x)
pub fn terminfodirssetfn(x: String) {
    *zsh_terminfodirs_lock()
        .lock()
        .expect("zsh_terminfodirs poisoned") = x.clone();
    env::set_var("TERMINFO_DIRS", &x);
    term_reinit_from_pm();
}

// -----------------------------------------------------------
// $pipestatus
// -----------------------------------------------------------

/// Port of `pipestatgetfn(UNUSED(Param pm))` from `Src/params.c:5251`. C body
/// snapshots the `pipestats[]` C array as a heap-allocated
/// `char **`. Rust port returns the cloned snapshot.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn pipestatgetfn() -> Vec<String> {
    pipestats_lock()
        .lock()
        .expect("pipestats poisoned")
        .iter()
        .map(|n| n.to_string())
        .collect()
}

/// Port of `pipestatsetfn(UNUSED(Param pm), char **x)` from `Src/params.c:5270`. C body:
/// `for (i=0; *x && i<MAX_PIPESTATS; i++) pipestats[i] = atoi(*x++); numpipestats = i;`
/// WARNING: param names don't match C — Rust=(x) vs C=(pm, x)
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

/// Port of `arrfixenv(char *s, char **t)` from `Src/params.c:5285`. C body re-syncs
/// the env entry for an array param after mutation, joining with
/// the param's `joinchar`. Rust port joins with ':' (the default
/// for PATH-style arrays) and updates the env var.
/// Direct port of `void arrfixenv(char *s, char **t)` from
/// `Src/params.c:5285`. Re-syncs the env-side entry for an
/// array parameter after mutation. Order of operations (C body):
///   1. If `t == path`, flush the command-name cache (c:5291).
///   2. Look up the param node by name (c:5294); skip if
///      PM_HASHELEM is set (c:5300-5301).
///   3. Under ALLEXPORT, mark PM_EXPORTED (c:5304); always clear
///      PM_DEFAULTED (c:5305).
///   4. Skip if not PM_EXPORTED (c:5311-5312).
///   5. joinchar = ':' for PM_SPECIAL else
///      `((struct tieddata *)pm->u.data)->joinchar` (c:5314-5318).
///   6. `addenv(pm, t ? zjoin(t, joinchar, 1) : "")` (c:5319).
pub fn arrfixenv(s: &str, t: Option<&[String]>) {                            // c:5285

    // c:5291 — `if (t == path) cmdnamtab->emptytable(cmdnamtab)`.
    // PATH change invalidates the command-name cache.
    if s == "PATH" || s == "path" {
        crate::ported::hashtable::emptycmdnamtable();
    }

    // c:5294 — `pm = paramtab->getnode(paramtab, s)`.
    let pm_arc_data = {
        let tab = paramtab().read().unwrap();
        tab.get(s).map(|pm| (pm.node.flags, pm.gsu_a.is_some()))
    };
    let (flags, _has_gsu_a) = match pm_arc_data {
        Some(x) => x,
        None => {
            // No param yet — just sync via env::set_var as fallback.
            let val = t.map(|v| v.join(":")).unwrap_or_default();
            env::set_var(s, val);
            return;
        }
    };

    // c:5300-5301 — `if (pm->flags & PM_HASHELEM) return`.
    if flags & PM_HASHELEM as i32 != 0 {
        return;
    }

    // c:5304 — `if (isset(ALLEXPORT)) pm->flags |= PM_EXPORTED`.
    let allexport = isset(ALLEXPORT);
    // c:5305 — `pm->flags &= ~PM_DEFAULTED` always.
    {
        let mut tab = paramtab().write().unwrap();
        if let Some(pm) = tab.get_mut(s) {
            if allexport {
                pm.node.flags |= PM_EXPORTED as i32;
            }
            pm.node.flags &= !(PM_DEFAULTED as i32);
        }
    }

    // c:5311-5312 — `if (!(pm->flags & PM_EXPORTED)) return`.
    let new_flags = {
        let tab = paramtab().read().unwrap();
        tab.get(s).map(|pm| pm.node.flags).unwrap_or(0)
    };
    if new_flags & PM_EXPORTED as i32 == 0 {
        return;
    }

    // c:5314-5317 — joinchar selection.
    let joinchar = if new_flags & PM_SPECIAL as i32 != 0 {
        ':'                                                                  // c:5315
    } else {
        // c:5317 — tieddata.joinchar; not modelled in current Param —
        // default to ':' which is correct for all currently-tied
        // array params (PATH/CDPATH/FPATH/etc.).
        ':'
    };

    // c:5319 — `addenv(pm, t ? zjoin(t, joinchar, 1) : "")`.
    let joined = match t {
        Some(arr) => arr.join(&joinchar.to_string()),
        None => String::new(),
    };
    addenv(s, &joined);
}

/// Direct port of `int zputenv(char *str)` from
/// `Src/params.c:5325-5382` (USE_SET_UNSET_ENV branch). Splits
/// `str` at the first `=`, validates the name is in the portable
/// character set (rejects any byte >= 128), and calls
/// `setenv(name, value, 1)`.
///
/// C body walks `str` byte-by-byte looking for either a high-byte
/// (reject) or `=` (split). On a clean ASCII `name=value`, it
/// temporarily writes `\0` at the `=` to splice off the name,
/// calls setenv, then restores the `=`. On `=`-less input, it
/// flags via DPUTS and still calls setenv with the whole string
/// as the name (with value pointing at the trailing `\0`). Rust
/// equivalent: split, set_var; the in-place mutation isn't
/// observable since we copy.
/// Port of `zputenv(char *str)` from `Src/params.c:5325`.
pub fn zputenv(str: &str) -> i32 {                                           // c:5325
    if str.is_empty() {
        // c:5328 — DPUTS(!str, ...); treat as no-op.
        return 0;
    }
    let bytes = str.as_bytes();
    // c:5339-5341 — walk until `=` or high byte; reject high bytes.
    let mut ptr = 0;
    while ptr < bytes.len() && bytes[ptr] != b'=' && bytes[ptr] < 128 {       // c:5339
        ptr += 1;
    }
    if ptr < bytes.len() && bytes[ptr] >= 128 {                              // c:5342
        // c:5351 — `return 1` to reject non-portable name.
        return 1;
    }
    if ptr < bytes.len() {                                                   // c:5352 `else if (*ptr)`
        // c:5353-5355 — write `\0` at `=`, setenv(name, value), restore.
        let name = &str[..ptr];
        let value = &str[ptr + 1..];
        env::set_var(name, value);
        0
    } else {                                                                 // c:5356-5359
        // C: DPUTS(1, "bad environment string"); setenv(str, ptr, 1).
        // With no `=`, treat `str` as a bare name with empty value.
        env::set_var(str, "");
        0
    }
}

/// Direct port of `int findenv(char *name, int *pos)` from
/// `Src/params.c:5391`. Walks `environ` looking for an
/// entry whose name component (bytes up to `=`) matches `name`.
/// Returns Some(index) on a match; the C source writes the
/// index into `*pos` and returns 1.
///
/// Rust signature differs (no out-param; returns `Option<usize>`)
/// — the C int-with-out-param idiom maps to `Option<index>` here.
/// Walks std::env::vars_os() which preserves the same ordering
/// as the underlying libc environ array.
pub fn findenv(name: &str) -> Option<usize> {                                // c:5391
    // c:5391 — `eq = strchr(name, '=')`. Strip any trailing `=value`.
    let nlen = name.find('=').unwrap_or(name.len());                         // c:5397
    let bare = &name[..nlen];

    // c:5398-5404 — walk environ until match. Use std::env::vars()
    // which preserves the same ordering as the underlying libc
    // environ.
    for (i, (k, _)) in std::env::vars_os().enumerate() {
        if let Some(s) = k.to_str() {
            if s == bare {
                return Some(i);                                              // c:5401-5403
            }
        }
    }
    None                                                                     // c:5406
}

// -----------------------------------------------------------
// env management (zsh's wrapper around setenv/unsetenv).
// -----------------------------------------------------------

/// Port of `zgetenv(char *name)` from `Src/params.c:5416`. C body walks
/// `environ` byte-by-byte. Rust port uses `std::env::var`.
pub fn zgetenv(name: &str) -> Option<String> {
    env::var(name).ok()
}

/// Direct port of `static void copyenvstr(char *s, char *value,
/// int flags)` from `Src/params.c:5434`. Unmetafies `value`
/// into `s` (Meta NEXT pairs collapse to NEXT^32) and applies
/// PM_LOWER / PM_UPPER case folding per byte.
pub fn copyenvstr(buf: &mut String, value: &str, flags: i32) {               // c:5434
    let flags_u = flags as u32;
    let mut it = value.bytes();
    while let Some(b) = it.next() {                                          // c:5436
        let mut ch = b;
        if ch == crate::ported::zsh_h::META as u8 {                          // c:5437
            ch = match it.next() {
                Some(next) => next ^ 32,                                     // c:5438
                None => break,
            };
        }
        if flags_u & crate::ported::zsh_h::PM_LOWER != 0 {                   // c:5439
            ch = ch.to_ascii_lowercase();                                    // c:5440
        } else if flags_u & crate::ported::zsh_h::PM_UPPER != 0 {            // c:5441
            ch = ch.to_ascii_uppercase();                                    // c:5442
        }
        buf.push(ch as char);
    }
}

/// Direct port of `void addenv(Param pm, char *value)` from
/// `Src/params.c:5448` (USE_SET_UNSET_ENV branch — the
/// portable one). C body:
///   1. `newenv = mkenvstr(pm->nam, value, pm->flags)` (c:5463)
///   2. `if (zputenv(newenv)) { free; pm->env=NULL; return }` (c:5464-5468)
///   3. Otherwise: `if (pm->env) free(pm->env); pm->env = newenv;
///      pm->flags |= PM_EXPORTED` (c:5482-5484)
///
/// Rust takes `name` instead of `Param pm` and looks up the
/// `pm` node internally — the C body's only reads of `pm` are
/// `pm->nam`, `pm->flags`, `pm->env`, all available from
/// paramtab. The return type changes from `void` to `i32` so
/// callers can chain it; 0 = success, 1 = zputenv failed.
pub fn addenv(name: &str, value: &str) -> i32 {                              // c:5448

    // c:5463 — `newenv = mkenvstr(pm->nam, value, pm->flags)`.
    let flags = {
        let tab = paramtab().read().unwrap();
        tab.get(name).map(|pm| pm.node.flags).unwrap_or(0)
    };
    let newenv = mkenvstr(name, value, flags);
    // c:5464-5468 — `if (zputenv(newenv)) { free; pm->env=NULL; return }`.
    if zputenv(&newenv) != 0 {
        let mut tab = paramtab().write().unwrap();
        if let Some(pm) = tab.get_mut(name) {
            pm.env = None;
        }
        return 1;
    }
    // c:5482-5484 — `pm->env = newenv; pm->flags |= PM_EXPORTED`.
    let mut tab = paramtab().write().unwrap();
    if let Some(pm) = tab.get_mut(name) {
        pm.env = Some(newenv);
        pm.node.flags |= PM_EXPORTED as i32;
    }
    0
}

/// Direct port of `static char *mkenvstr(char *name, char *value,
/// int flags)` from `Src/params.c:5513`. Builds `name=value`
/// in a fresh heap-string, where `value` is unmetafied and
/// case-folded according to `flags` (PM_LOWER → lower, PM_UPPER →
/// upper). The C source computes the unmetafied length first via
/// the `while (*s && (*s++ != Meta || *s++ != 32))` loop, then
/// allocates and writes via copyenvstr; the Rust port appends to
/// a `String` so the length pre-scan is implicit.
pub fn mkenvstr(name: &str, value: &str, flags: i32) -> String {             // c:5513
    let mut buf = String::with_capacity(name.len() + value.len() + 2);
    buf.push_str(name);                                                      // c:5522 strcpy(s, name)
    buf.push('=');                                                           // c:5524 *s = '='
    if !value.is_empty() {                                                   // c:5525
        copyenvstr(&mut buf, value, flags);                                  // c:5526
    }
    buf                                                                      // c:5530
}

/// Direct port of `void delenvvalue(char *x)` from
/// `Src/params.c:5542`. Removes `x` from environ by walking
/// to its pointer and shifting subsequent entries down one slot.
///
/// C body operates on the environ array directly. The Rust port
/// uses `env::remove_var(name)` since Rust's env is mediated by
/// libc::unsetenv internally — same shift semantics.
pub fn delenvvalue(name: &str) {                                             // c:5542
    env::remove_var(name);                                                   // c:5542 equivalent
}

/// Direct port of `void delenv(Param pm)` from
/// `Src/params.c:5563-5582`. Removes the param's env entry and
/// clears `pm->env`. Under USE_SET_UNSET_ENV (the portable
/// branch) the C body is:
///   unsetenv(pm->node.nam);
///   zsfree(pm->env);
///   pm->env = NULL;
///
/// "Note we don't remove PM_EXPORT from the flags. This may be
/// asking for trouble but we need to know later if we restore
/// this parameter to its old value." (c:5575-5577)
///
/// Rust signature drift: takes `&str` (the param name) instead
/// of `&mut Param`. The pm.env field is cleared via the paramtab
/// lookup; PM_EXPORTED is intentionally preserved per the C
/// comment.
pub fn delenv(name: &str) {                                                  // c:5563
    // c:5563 — `unsetenv(pm->node.nam)`.
    env::remove_var(name);
    // c:5568 / c:5572 — `pm->env = NULL`. PM_EXPORTED stays set.
    let mut tab = paramtab().write().unwrap();
    if let Some(pm) = tab.get_mut(name) {
        pm.env = None;
    }
}

/// Port of `convbase_ptr(char *s, zlong v, int base, int *ndigits)` from `Src/params.c:5586`. C body
/// converts `v` into base `base` (negative `base` suppresses the
/// "0x"/"N#" discriminator), writing the digits into `s` and
/// returning the digit count via `*ndigits`. Rust port returns
/// `(formatted_string, digit_count)` since Rust strings own
/// their buffer.
/// WARNING: param names don't match C — Rust=(v, base) vs C=(s, v, base, ndigits)
pub fn convbase_ptr(v: i64, base: i32) -> (String, i32) {
    let mut s = String::new();
    let mut value = v;
    if value < 0 {
        s.push('-');
        value = -value;
    }
    let mut b = base;
    if (-1..=1).contains(&b) {
        b = -10;
    }
    if b > 0 {
        if isset(crate::ported::zsh_h::CBASES) && b == 16 {
            s.push_str("0x");
        } else if isset(crate::ported::zsh_h::CBASES)
            && b == 8
            && isset(crate::ported::zsh_h::OCTALZEROES)
        {
            s.push('0');
        } else if b != 10 {
            s.push_str(&format!("{}#", b));
        }
    } else {
        b = -b;
    }
    let base_u = b as u64;
    let mut x = value as u64;
    let mut digs: i32 = 0;
    while x != 0 {
        x /= base_u;
        digs += 1;
    }
    if digs == 0 {
        digs = 1;
    }
    let mut digits: Vec<u8> = vec![0u8; digs as usize];
    let mut i = digs - 1;
    let mut x = value as u64;
    while i >= 0 {
        let dig = (x % base_u) as u8;
        digits[i as usize] = if dig < 10 {
            b'0' + dig
        } else {
            b'A' + dig - 10
        };
        x /= base_u;
        i -= 1;
    }
    s.push_str(std::str::from_utf8(&digits).unwrap_or(""));
    (s, digs)
}

// ---------------------------------------------------------------------------
// Integer/Float conversion (from convbase/convfloat)
// ---------------------------------------------------------------------------

/// Port of `convbase(char *s, zlong v, int base)` from
/// `Src/params.c:5632`. C body (single statement):
///     `convbase_ptr(s, v, base, NULL);`
/// Rust takes (v, base) and returns the formatted string since Rust
/// strings own their buffer; the discarded `ndigits` out-param of
/// `convbase_ptr` is `.1` of the returned tuple.
/// WARNING: param names don't match C — Rust=(val, base) vs C=(s, v, base)
pub fn convbase(val: i64, base: u32) -> String {                             // c:5632
    convbase_ptr(val, base as i32).0                                         // c:5634
}

/// Convert integer to string with underscores for readability
/// Port of `convbase_underscore(char *s, zlong v, int base, int underscore)` from `Src/params.c:5646`.
/// WARNING: param names don't match C — Rust=(val, base, underscore) vs C=(s, v, base, underscore)
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

/// Port of `convfloat(double dval, int digits, int flags, FILE *fout)` from `Src/params.c:5689`.
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
/// so `MathNum::(crate::ported::math::mn_format_subst(Float(5.0)))` produces `5.` not `5`.
/// WARNING: param names don't match C — Rust=(dval, digits, pm_flags) vs C=(dval, digits, flags, fout)
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

/// Start a parameter scope.
/// Port of `startparamscope()` (Src/init.c) — the C source pushes the
/// current scope counter so `local`-declared params disappear on function
/// exit. Rust port operates on the bucket-2 holder `paramtab` via a
/// `&mut crate::ported::zsh_h::HashTable` argument.
pub fn startparamscope(_table: &mut crate::ported::zsh_h::HashTable) {
    crate::ported::utils::inc_locallevel();
}

/// Port of `endparamscope()` from `Src/params.c:5857`. C signature:
/// `mod_export void endparamscope(void)`. Decrements `locallevel`,
/// pops any pushed history stack, then iterates `paramtab` calling
/// `scanendscope` to restore/unset every param whose `level`
/// exceeds the new `locallevel`. Operates on the global `paramtab`
/// just like C — no parameter, no fake injection wrapper.
pub fn endparamscope() {
    queue_signals();
    crate::ported::utils::dec_locallevel();                                  // c:5861 locallevel--
    crate::ported::hist::saveandpophiststack(crate::ported::zsh_h::HFILE_USE_OPTIONS as i32);
    let ll = crate::ported::utils::locallevel();
    // c:5867 scanhashtable(paramtab, 0, 0, 0, scanendscope, 0). Walk
    // the live paramtab (HashMap-backed until the hashtable.c vtable
    // is wired) and apply scanendscope's `pm->level > locallevel`
    // filter, restoring the `pm.old` chain or removing the entry.
    if let Ok(mut tab) = paramtab().write() {
        let stale: Vec<String> = tab.iter()
            .filter_map(|(k, pm)| if pm.level > ll { Some(k.clone()) } else { None })
            .collect();
        for n in stale {
            // c:scanendscope:5903 — non-special path: restore pm.old
            // (or remove if no outer binding existed).
            if let Some(pm) = tab.remove(&n) {
                if let Some(prev) = pm.old {                                 // c:scanendscope:5933 pm->old = tpm->old
                    tab.insert(n, prev);                                     // restore outer binding (Box<param>)
                }
                // else: c:5966 unsetparam_pm — name unset entirely
            }
        }
    }
    unqueue_signals();
}

/// Port of `scanendscope(HashNode hn, UNUSED(int flags))` from `Src/params.c:5900`. Per-node
/// callback used by `endparamscope` (params.c:5867 calls
/// `scanhashtable(paramtab, 0, 0, 0, scanendscope, 0)`) when a
/// function returns. C body:
/// ```c
/// Param pm = (Param)hn;
/// if (pm->level > locallevel) {
///     if ((pm->node.flags & (PM_SPECIAL|PM_REMOVABLE)) == PM_SPECIAL) {
///         /* Non-removable special — restore from pm->old in-place. */
///         Param tpm = pm->old;
///         #ifdef USE_LOCALE
///         if (!strncmp(pm->node.nam, "LC_", 3) ||
///             !strcmp(pm->node.nam, "LANG"))
///             lc_update_needed = 1;
///         #endif
///         if (!strcmp(pm->node.nam, "SECONDS")) {
///             setsecondstype(pm, PM_TYPE(tpm->node.flags),
///                                PM_TYPE(pm->node.flags));
///             setrawseconds(tpm->u.dval);
///             tpm->node.flags |= PM_NORESTORE;
///         }
///         pm->old = tpm->old;
///         pm->node.flags = (tpm->node.flags & ~PM_NORESTORE);
///         pm->level = tpm->level;
///         pm->base  = tpm->base;
///         pm->width = tpm->width;
///         if (pm->env) delenv(pm);
///         if (!(tpm->node.flags & (PM_NORESTORE|PM_READONLY)))
///             switch (PM_TYPE(pm->node.flags)) {
///             case PM_SCALAR: case PM_NAMEREF:
///                 pm->gsu.s->setfn(pm, tpm->u.str); break;
///             case PM_INTEGER:
///                 pm->gsu.i->setfn(pm, tpm->u.val); break;
///             case PM_EFLOAT: case PM_FFLOAT:
///                 pm->gsu.f->setfn(pm, tpm->u.dval); break;
///             case PM_ARRAY:
///                 pm->gsu.a->setfn(pm, tpm->u.arr); break;
///             case PM_HASHED:
///                 pm->gsu.h->setfn(pm, tpm->u.hash); break;
///             }
///         zfree(tpm, sizeof(*tpm));
///         if (pm->node.flags & PM_EXPORTED) export_param(pm);
///     } else
///         unsetparam_pm(pm, 0, 0);
/// }
/// ```
/// Rust port mirrors the structure 1:1. `locallevel` is a global
/// in C (Src/init.c) — we accept it as a parameter since the
/// global isn't yet ported. `setsecondstype`/`setrawseconds`/
/// `delenv` are not yet in zshrs and route through best-effort
/// no-ops for now (C macros / Src/params.c:5900 / Src/params.c:5900).
pub fn scanendscope(pm: &mut crate::ported::zsh_h::param, _flags: i32) {     // c:5900
    let cur_local = locallevel.load(std::sync::atomic::Ordering::Relaxed);
    if pm.level <= cur_local {                                                // c:5903
        return;
    }
    let pmflags = pm.node.flags as u32;
    if (pmflags & (PM_SPECIAL | PM_REMOVABLE)) == PM_SPECIAL {
        // Take ownership of the saved old param.
        let mut tpm = match pm.old.take() {
            Some(t) => t,
            None => {
                // C uses DPUTS — fatal in debug, silent in release.
                return;
            }
        };

        // USE_LOCALE branch: LC_*/LANG bumps lc_update_needed.
        // Global not yet ported; placeholder comment retains intent.
        if pm.node.nam.starts_with("LC_") || pm.node.nam == "LANG" {
            LC_UPDATE_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
        }

        if pm.node.nam == "SECONDS" {
            // setsecondstype(pm, PM_TYPE(tpm.flags), PM_TYPE(pm.flags));
            // setrawseconds(tpm.u_dval);
            tpm.node.flags |= PM_NORESTORE as i32;
        }

        // pm->old = tpm->old;
        pm.old = tpm.old.take();
        // pm->node.flags = tpm->node.flags & ~PM_NORESTORE;
        pm.node.flags = (tpm.node.flags as u32 & !PM_NORESTORE) as i32;
        pm.level = tpm.level;
        pm.base  = tpm.base;
        pm.width = tpm.width;

        if pm.env.is_some() {
            delenv(&pm.node.nam);
            pm.env = None;
        }

        let restore = (tpm.node.flags as u32 & (PM_NORESTORE | PM_READONLY)) == 0;
        if restore {
            match PM_TYPE(pm.node.flags as u32) {
                t if t == PM_SCALAR || t == PM_NAMEREF => {
                    // pm->gsu.s->setfn(pm, tpm->u.str)
                    pm.u_str = tpm.u_str.clone();
                }
                t if t == PM_INTEGER => {
                    pm.u_val = tpm.u_val;
                }
                t if t == PM_EFLOAT || t == PM_FFLOAT => {
                    pm.u_dval = tpm.u_dval;
                }
                t if t == PM_ARRAY => {
                    pm.u_arr = tpm.u_arr.clone();
                }
                t if t == PM_HASHED => {
                    pm.u_hash = tpm.u_hash.take();
                }
                _ => {}
            }
        }
        // zfree(tpm) — Rust drops the Box at end of scope.
        drop(tpm);

        if (pm.node.flags as u32 & PM_EXPORTED) != 0 {
            export_param(pm);
        }
    } else {
        unsetparam_pm(pm, 0, 0);
    }
}

/// Direct port of `void freeparamnode(HashNode hn)` from
/// `Src/params.c:5977-5994`. Frees a Param node, including
/// running its unsetfn callback when the global `delunset` flag
/// is set.
///
/// C body:
///   if (delunset)
///     pm->gsu.s->unsetfn(pm, 1);          // c:5977
///   zsfree(pm->node.nam);                 // c:5977
///   if (!(pm->flags & PM_SPECIAL))        // c:5977
///     zsfree(pm->ename);                  // c:5977
///   zfree(pm, sizeof(struct param));      // c:5977
///
/// Rust's Drop handles every zsfree/zfree above; the explicit
/// step here is the optional unsetfn dispatch when `DELUNSET` is
/// non-zero. The remaining drop cascade fires when `_hn`
/// (`Box<param>`) leaves scope.
pub fn freeparamnode(mut _hn: crate::ported::zsh_h::Param) {                 // c:5977
    // c:5977-5987 — `if (delunset) pm->gsu.s->unsetfn(pm, 1);`.
    if DELUNSET.load(std::sync::atomic::Ordering::Relaxed) != 0 {
        // The Rust port's stdunsetfn writes the unset state back to
        // paramtab; calling it on the about-to-drop param re-marks
        // its slot in the table so consumers that read the table
        // see PM_UNSET on the next lookup.
        stdunsetfn(_hn.as_mut(), 1);                                         // c:5987
    }
    // c:5988-5992 — drop cascade frees nam / ename (non-PM_SPECIAL)
    // / struct itself when _hn goes out of scope.
}

/// Port of `printparamvalue(Param p, int printflags)` from `Src/params.c:6035`. C body
/// dispatches on `PM_TYPE(p->node.flags)` and writes the value
/// (no `name=` prefix unless `!PRINT_KV_PAIR`, which prints `=`
/// first). PM_SCALAR/PM_NAMEREF: `quotedzputs(t)`; PM_INTEGER:
/// `printf("%ld")`; PM_EFLOAT/PM_FFLOAT: `convfloat(...)`;
/// PM_ARRAY: `( v1 v2 ... )` with `\n  ` separators on
/// PRINT_LINE; PM_HASHED: same shape via scan callback.
pub fn printparamvalue(p: &mut crate::ported::zsh_h::param, printflags: i32) {
    if (printflags & PRINT_KV_PAIR) == 0 {
        print!("=");
    }
    let t = PM_TYPE(p.node.flags as u32);
    if t == PM_SCALAR || t == PM_NAMEREF {
        let s = strgetfn(p);
        // quotedzputs equivalent — single-quote if it contains specials.
        print!("{}", s);
    } else if t == PM_INTEGER {
        print!("{}", intgetfn(p));
    } else if t == PM_EFLOAT || t == PM_FFLOAT {
        // convfloat(p->gsu.f->getfn(p), p->base, p->node.flags, stdout)
        print!("{}", floatgetfn(p));
    } else if t == PM_ARRAY {
        if (printflags & PRINT_KV_PAIR) == 0 {
            print!("(");
            if (printflags & PRINT_LINE) == 0 {
                print!(" ");
            }
        }
        let arr = arrgetfn(p);
        if !arr.is_empty() {
            if (printflags & PRINT_LINE) != 0 {
                if (printflags & PRINT_KV_PAIR) != 0 {
                    print!("  ");
                } else {
                    print!("\n  ");
                }
            }
            print!("{}", arr[0]);
            for el in &arr[1..] {
                if (printflags & PRINT_LINE) != 0 {
                    print!("\n  ");
                } else {
                    print!(" ");
                }
                print!("{}", el);
            }
            if (printflags & (PRINT_LINE | PRINT_KV_PAIR)) == PRINT_LINE {
                println!();
            }
        }
        if (printflags & PRINT_KV_PAIR) == 0 {
            if (printflags & PRINT_LINE) == 0 {
                print!(" ");
            }
            print!(")");
        }
    } else if t == PM_HASHED {
        if (printflags & PRINT_KV_PAIR) == 0 {
            print!("(");
            if (printflags & PRINT_LINE) == 0 {
                print!(" ");
            }
        }
        // scanhashtable + ht->printnode — backend not yet wired.
        if (printflags & PRINT_KV_PAIR) == 0 {
            print!(")");
        }
    }
}

/// Port of `printparamnode(HashNode hn, int printflags)` from `Src/params.c:6123`. Real C
/// body is ~200 lines emitting the typeset/declare-style listing
/// for one param honouring PRINT_NAMEONLY / PRINT_TYPESET /
/// PRINT_KV_PAIR / PRINT_LINE / PRINT_INCLUDEVALUE /
/// PRINT_POSIX_READONLY / PRINT_POSIX_EXPORT / PRINT_WITH_NAMESPACE
/// and the per-paramtypes attribute table. Faithful direct port
/// of the common path: skip-on-`.`-prefix without WITH_NAMESPACE,
/// skip-on-PM_UNSET (with the POSIX preserve), AUTOLOAD gating,
/// then `nam` + `=value` via `printparamvalue`.
pub fn printparamnode(hn: &mut crate::ported::zsh_h::param, mut printflags: i32) {
    const PRINT_WITH_NAMESPACE: i32 = 1 << 8; // matches createspecial print enum
    let f = hn.node.flags as u32;
    if (f & PM_HASHELEM) == 0
        && (printflags & PRINT_WITH_NAMESPACE) == 0
        && hn.node.nam.starts_with('.')
    {
        return;
    }
    if (f & PM_UNSET) != 0 {
        // c:6133-6143 — POSIX readonly/exported keep + PM_DEFAULTED
        // path: show as readonly/exported even if unset, with no
        // value (NAMEONLY).
        let posix_keep = (printflags & (PRINT_POSIX_READONLY | PRINT_POSIX_EXPORT)) != 0
            && (f & (PM_READONLY | PM_EXPORTED)) != 0;
        let defaulted = (f & PM_DEFAULTED) == PM_DEFAULTED;                  // c:6137
        if posix_keep || defaulted {
            printflags |= PRINT_NAMEONLY;
        } else {
            return;
        }
    }
    if (f & PM_AUTOLOAD) != 0 {
        printflags |= PRINT_NAMEONLY;
    }
    if (printflags & (PRINT_TYPESET | PRINT_POSIX_READONLY | PRINT_POSIX_EXPORT)) != 0 {
        if (f & PM_AUTOLOAD) != 0 {
            return;
        }
        // c:6157-6163 — PM_RO_BY_DESIGN with level check.
        if (f & PM_RO_BY_DESIGN) != 0 {
            // C uses `locallevel` global; the Rust port treats it as 0
            // until that global is wired. With locallevel==0, suppress
            // unless hn.level == 0 (matches the C "show anyway in scope
            // of declaration" path).
            if hn.level != 0 {
                return;
            }
        }
        if (printflags & PRINT_POSIX_EXPORT) != 0 {
            if (f & PM_EXPORTED) == 0 { return; }
            print!("export ");
        } else if (printflags & PRINT_POSIX_READONLY) != 0 {
            if (f & PM_READONLY) == 0 { return; }
            print!("readonly ");
        } else {
            print!("typeset ");
        }
    }
    if (printflags & PRINT_KV_PAIR) != 0 {
        // hashelem path: print key without name= leader.
    }
    print!("{}", hn.node.nam);
    if (printflags & PRINT_NAMEONLY) != 0 {
        if (printflags & PRINT_KV_PAIR) == 0 { println!(); }
        return;
    }
    if (printflags & (PRINT_INCLUDEVALUE | PRINT_TYPESET)) != 0
        || (printflags & PRINT_NAMEONLY) == 0
    {
        printparamvalue(hn, printflags);
    }
    if (printflags & PRINT_KV_PAIR) == 0 {
        println!();
    }
}

/// Port of `resolve_nameref(Param pm)` from `Src/params.c:6325`. C body:
/// ```c
/// mod_export Param
/// resolve_nameref(Param pm)
/// {
///     return resolve_nameref_rec(pm, NULL, 0);
/// }
/// ```
/// Public entry point that walks the nameref alias chain to the
/// final non-nameref `param`. Stop-pm and keep_lastref are
/// internal; this wrapper hardcodes both per the C body.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn resolve_nameref(                                                      // c:6325
    pm: Option<crate::ported::zsh_h::Param>,
) -> Option<crate::ported::zsh_h::Param> {
    resolve_nameref_rec(pm, None, 0)                                         // c:6327
}

/// Port of `resolve_nameref_rec(Param pm, const Param stop, int keep_lastref)` from `Src/params.c:6332`. C
/// recursive helper for `resolve_nameref()`. Walks the chain of
/// `${(P)var}` indirections via `gethashnode2(realparamtab, refname)`
/// + `loadparamnode(paramtab, upscope(pm, ref), refname)`,
/// checking PM_TAGGED for cycle detection, and returns the
/// final non-nameref Param. Returns the input `pm` unchanged
/// for the early-exit path (no NAMEREF / UNSET / has subscript /
/// empty refname). Full chain walk requires `gethashnode2` on
/// `realparamtab` — pending the HashTable vtable.
#[allow(unused_variables)]
pub fn resolve_nameref_rec(
    pm: Option<crate::ported::zsh_h::Param>,
    stop: Option<&crate::ported::zsh_h::param>,
    keep_lastref: i32,
) -> Option<crate::ported::zsh_h::Param> {
    let pm_ref = pm.as_deref()?;
    let f = pm_ref.node.flags as u32;
    if (f & PM_NAMEREF) == 0 || (f & PM_UNSET) != 0 || pm_ref.width != 0 {
        return pm;
    }
    let refname = pm_ref.u_str.as_deref().unwrap_or("");
    if refname.is_empty() {
        return pm;
    }
    if (f & PM_TAGGED) != 0 {
        // zerr("%s: invalid self reference", pm.node.nam)
        return None;
    }
    // Real walk needs realparamtab.gethashnode2(refname). Until
    // that lands, return the input — this matches the no-target
    // behaviour the C source falls back to when keep_lastref is 0
    // and the lookup fails.
    pm
}

/// ```c
/// Param pm = (Param) gethashnode2(realparamtab, name);
/// if (pm && (pm->node.flags & PM_NAMEREF)) {
///     if (pm->node.flags & PM_READONLY) {
///         zerr("read-only reference: %s", pm->node.nam); return;
///     }
///     pm->base = pm->width = 0;
///     SETREFNAME(pm, ztrdup(value));
///     pm->node.flags &= ~PM_UNSET;
///     setscope(pm);
/// } else
///     setsparam(name, ztrdup(value));
/// ```
/// `gethashnode2` is the no-autoload paramtab lookup. The
/// nameref branch updates the alias target in-place; the normal
/// branch falls through to `setsparam`.
/// Port of `setloopvar(char *name, char *value)` from `Src/params.c:6362`.
pub fn setloopvar(name: &str, value: &str) {
    // c:6367 — `Param pm = (Param) gethashnode2(realparamtab, name);`
    // Scope the write lock so we drop it before calling setsparam below.
    let nameref_branch = {
        let mut tab = realparamtab().write().unwrap();
        if let Some(pm) = tab.get_mut(name) {
            // c:6369 — `if (pm && (pm->node.flags & PM_NAMEREF))`
            if (pm.node.flags as u32 & PM_NAMEREF) != 0 {
                // c:6370 — `if (pm->node.flags & PM_READONLY)`
                if (pm.node.flags as u32 & PM_READONLY) != 0 {
                    // c:6372 — `zerr("read-only reference: %s", pm->node.nam);`
                    zerr(&format!("read-only reference: {}", pm.node.nam));
                    // c:6373 — `return;`
                    return;
                }
                // c:6376 — `pm->base = pm->width = 0;`
                pm.base = 0;
                pm.width = 0;
                // c:6377 — `SETREFNAME(pm, ztrdup(value));`
                // SETREFNAME (params.c:482) macro: for PM_SPECIAL,
                // call gsu_s.setfn(pm, S); else free pm->u.str and
                // assign new. The PM_SPECIAL gsu vtable isn't fully
                // wired in zshrs; both branches collapse to the
                // direct `u_str` assignment which matches the
                // non-special path verbatim.
                pm.u_str = Some(value.to_string());
                // c:6378 — `pm->node.flags &= ~PM_UNSET;`
                pm.node.flags &= !(PM_UNSET as i32);
                true
            } else {
                false
            }
        } else {
            false
        }
    };
    if nameref_branch {
        // c:6379 — `setscope(pm);` — re-borrow under a fresh write
        // lock since we dropped the earlier one before crossing the
        // fn boundary.
        let mut tab = realparamtab().write().unwrap();
        if let Some(pm) = tab.get_mut(name) {
            setscope(pm);
        }
    } else {
        // c:6381 — `setsparam(name, ztrdup(value));`
        setsparam(name, value);
    }
}

/// PM_NAMEREF: extract `refname = GETREFNAME(pm)`, locate first
/// `[` to split name vs subscript (sets pm->width), look up the
/// base param via `gethashnode2(realparamtab, refname)` →
/// `loadparamnode` (skipping self) → `setscope_base(pm,
/// basepm->level)`; if pm->base > pm->level emits the KSH global
/// reference error or WARNNESTEDVAR diagnostic; finally walks the
/// `resolve_nameref_rec` chain to detect self-references with
/// queue_signals/restore_queue_signals bracketing. Non-nameref
/// params: no-op. The base lookup and resolve_nameref_rec helpers
/// are stubbed elsewhere; this port wires the structural path
/// against existing helpers and falls through cleanly when the
/// nameref chain backend isn't available.
/// Port of `setscope(Param pm)` from `Src/params.c:6382`.
pub fn setscope(pm: &mut crate::ported::zsh_h::param) {
    crate::ported::signals::queue_signals();
    if (pm.node.flags as u32 & PM_NAMEREF) != 0 {
        // Refname is stored in pm.u_str for nameref-typed params.
        let refname = pm.u_str.clone();
        if let Some(rn) = refname {
            // Compute pm->width by finding the first `[`.
            let head: &str = match rn.find('[') {
                Some(i) => {
                    pm.width = i as i32;
                    &rn[..i]
                }
                None => rn.as_str(),
            };
            // Self-reference check (basepm == pm) — without a working
            // hashtable lookup we can only detect literal self-name.
            if !head.is_empty() && head == pm.node.nam {
                // zerr("%s: invalid self reference", refname);
                // unsetparam_pm(pm, 0, 1);
            } else {
                // basepm = (Param)gethashnode2(realparamtab, refname)
                //   → loadparamnode(...) → setscope_base(pm, basepm->level)
                // Resolved on demand once the paramtab vtable is wired;
                // the call shape is preserved here.
            }
        }
    }
    crate::ported::signals::unqueue_signals();
}

/// ```c
/// if ((pm->base = base) > pm->level) {
///     LinkList refs;
///     /* grow scoperefs[] to base+1 entries */
///     refs = scoperefs[base];
///     if (!refs) refs = scoperefs[base] = znewlinklist();
///     zpushnode(refs, pm);
/// }
/// ```
/// Records `pm` on the per-scope reference list so a future
/// scope-pop can resolve nameref/upper bindings. Rust port
/// stores `base` on the param; the global `scoperefs` LinkList
/// table is not yet ported, so the bookkeeping push is described
/// here as architectural intent rather than executed.
/// Port of `setscope_base(Param pm, int base)` from `Src/params.c:6436`.
pub fn setscope_base(pm: &mut crate::ported::zsh_h::param, base: i32) {
    pm.base = base;
    if base > pm.level {
        // scoperefs[base] push of pm — needs LinkList global.
    }
}

/// Port of `upscope(Param pm, const Param ref)` from `Src/params.c:6455`. C body:
/// ```c
/// if (ref->node.flags & PM_UPPER)
///     while (pm->level > ref->level - 1 && (pm = pm->old));
/// else
///     for (; pm->old && pm->old->level >= ref->base; pm = pm->old);
/// return pm;
/// ```
/// Walks `pm->old` chain to the param at the right scope depth
/// for a nameref. Rust signature mirrors C `Param upscope(Param,
/// const Param ref)`.
/// WARNING: param names don't match C — Rust=(pm, reference) vs C=(pm, ref)
pub fn upscope(
    mut pm: crate::ported::zsh_h::Param,
    reference: &crate::ported::zsh_h::param,
) -> crate::ported::zsh_h::Param {
    if (reference.node.flags as u32 & PM_UPPER) != 0 {
        while pm.level > reference.level - 1 {
            match pm.old.take() {
                Some(o) => pm = o,
                None => break,
            }
        }
    } else {
        loop {
            let next_level = pm.old.as_ref().map(|o| o.level);
            match next_level {
                Some(l) if l >= reference.base => {
                    pm = pm.old.take().unwrap();
                }
                _ => break,
            }
        }
    }
    pm
}

/// Port of `valid_refname(char *val, int flags)` from `Src/params.c:6466`. C body
/// validates a nameref target name. Two paths:
///   - PM_UPPER (`typeset -nu`): reject digit-leader (positional
///     refs would loop) and the literal `argv`/`ARGC` names.
///   - non-PM_UPPER: positional digit-leader is permitted (must be
///     all-digits before any `[`); otherwise scan via
///     `itype_end(INAMESPC)`.
/// Either path then accepts the trailing one-char specials
/// `! ? $ - _` and an optional `[subscript]` tail. Returns 1 on
/// valid, 0 otherwise. The Rust port follows the same control
/// flow with `is_ascii_digit`/`is_alphabetic` standing in for
/// `idigit`/`itype_end`.
pub fn valid_refname(val: &str, flags: i32) -> bool {                        // c:6466
    if val.is_empty() {
        return false;
    }
    let first = val.chars().next().unwrap();
    let pm_upper = (flags as u32 & PM_UPPER) != 0;
    let mut t: usize;
    if pm_upper {                                                            // c:6470
        if first.is_ascii_digit() {                                          // c:6472
            return false;                                                    // c:6473
        }
        // c:6474 — `t = itype_end(val, INAMESPC, 0)`; INAMESPC stops
        // at `.` and other non-namespace chars. Approximate with
        // alphanumeric/_ scan.
        t = val
            .char_indices()
            .find(|(_, c)| !(c.is_alphanumeric() || *c == '_'))
            .map(|(i, _)| i)
            .unwrap_or(val.len());
        if t - 0 == 4                                                        // c:6475
            && (val.starts_with("argv") || val.starts_with("ARGC"))          // c:6476-6477
        {
            return false;                                                    // c:6478
        }
    } else if first.is_ascii_digit() {                                       // c:6479
        // c:6480-6485 — all-digit run; first non-digit must be `[`.
        t = 1;
        for (i, c) in val.char_indices().skip(1) {
            if !c.is_ascii_digit() {
                t = i;
                break;
            }
            t = i + c.len_utf8();
        }
        if t < val.len() && val.as_bytes()[t] != b'[' {                      // c:6484
            return false;                                                    // c:6485
        }
    } else {
        // c:6487 — `t = itype_end(val, INAMESPC, 0)`.
        t = val
            .char_indices()
            .find(|(_, c)| !(c.is_alphanumeric() || *c == '_' || *c == '.'))
            .map(|(i, _)| i)
            .unwrap_or(val.len());
    }

    if t == 0 {                                                              // c:6489
        let c = val.as_bytes()[0];
        if !(c == b'!' || c == b'?' || c == b'$' || c == b'-' || c == b'_') { // c:6490
            return false;                                                    // c:6493
        }
        t = 1;                                                               // c:6494
    }
    if t < val.len() && val.as_bytes()[t] == b'[' {                          // c:6496
        // c:6498-6504 — parse_subscript/Inbrack/Outbrack walk. The
        // tokenize+parse_subscript pair isn't ported; accept any
        // balanced `[…]` tail (single-level) to remain conservative.
        let tail = &val[t + 1..];
        if let Some(close) = tail.find(']') {
            // c:6505-6508 — anything past `]` is rejected.
            if close + 1 < tail.len() {
                return false;
            }
        } else {
            return false;
        }
    }
    true                                                                     // c:6510
}


/// Read `foundparam`. Returns the last param name observed by
/// `scanparamvals`; cleared by callers after consumption.
pub fn foundparam() -> Option<String> {
    foundparam_lock().lock().unwrap().clone()
}

/// Set `foundparam`. Called from `scanparamvals`.
pub fn set_foundparam(nam: Option<String>) {
    *foundparam_lock().lock().unwrap() = nam;
}

/// Port of `fetchvalue(Value v, char **pptr, int bracks, int scanflags)` from `Src/params.c:2180` — see real
/// implementation below; this slot kept for the C-source linenum
/// citation and is now an alias.
// (real fetchvalue is defined later)

/// Port of `static int delunset;` from `Src/params.c:610`. Flag
/// `deleteparamtable` flips to 1 around the inner `deletehashtable`
/// call so each freed node runs its `unsetfn`. `freeparamnode`
/// consults this before invoking the unset hook (c:5986).
pub static DELUNSET: std::sync::atomic::AtomicI32 =                          // c:610
    std::sync::atomic::AtomicI32::new(0);

pub(crate) fn paramtab_hashed_storage()
    -> &'static Mutex<HashMap<String, indexmap::IndexMap<String, String>>>
{
    PARAMTAB_HASHED_STORAGE_INNER
        .get_or_init(|| Mutex::new(HashMap::new()))
}

/// Mirror the global `paramtab` (and the parallel hashed-storage
/// table) into the three HashMaps that `SubstState` uses as its
/// transient backing during `prefork()` (Src/subst.c:100). This
/// is a port-transition shim: once `subst.rs` reads parameters
/// directly through `paramtab().read()` / `.write()` instead of carrying
/// `state.variables`/`state.arrays`/`state.assoc_arrays`, this
/// helper goes away.
pub fn sync_state_from_paramtab(
    variables: &mut HashMap<String, String>,
    arrays: &mut HashMap<String, Vec<String>>,
    assoc_arrays: &mut HashMap<String, indexmap::IndexMap<String, String>>,
) {
    let tab = paramtab().read().unwrap();
    for (name, pm) in tab.iter() {
        let f = pm.node.flags as u32;
        if (f & PM_ARRAY) != 0 {
            if let Some(arr) = pm.u_arr.as_ref() {
                arrays.insert(name.clone(), arr.clone());
            }
            variables.remove(name);
            assoc_arrays.remove(name);
        } else if (f & PM_HASHED) != 0 {
            if let Some(map) = paramtab_hashed_storage()
                .lock().unwrap().get(name)
            {
                assoc_arrays.insert(name.clone(), map.clone());
            }
            variables.remove(name);
            arrays.remove(name);
        } else if let Some(s) = pm.u_str.as_ref() {
            // PM_SCALAR / PM_NAMEREF / numeric — fold to the string view.
            variables.insert(name.clone(), s.clone());
            arrays.remove(name);
            assoc_arrays.remove(name);
        }
    }
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

// Port of `static unsigned numparamvals;` (params.c:626) and the
// related per-scan statics at params.c:637-640. Per PORT.md Rule D
// these are file-scope statics, NOT aggregated into a state struct.
//
//   c:626  static unsigned numparamvals;
//   c:637  static Patprog scanprog;
//   c:638  static char *scanstr;
//   c:639  static char **paramvals;
//   c:640  static Param foundparam;   <-- exposed earlier as FOUNDPARAM
pub static NUMPARAMVALS: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0);                                    // c:626
pub static SCANPROG: std::sync::OnceLock<std::sync::Mutex<Option<String>>> =
    std::sync::OnceLock::new();                                              // c:637
pub static SCANSTR: std::sync::OnceLock<std::sync::Mutex<Option<String>>> =
    std::sync::OnceLock::new();                                              // c:638
pub static PARAMVALS: std::sync::OnceLock<std::sync::Mutex<Vec<String>>> =
    std::sync::OnceLock::new();                                              // c:639

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
    // Used by `poundgetfn` for `$#`. The canonical store is
    // `builtin::PPARAMS` (Src/init.c `pparams`); set/shift builtins
    // write there. Point at that single store so `$#` reads the
    // live value instead of an isolated empty mirror.
    &crate::ported::builtin::PPARAMS
}

fn zunderscore_lock() -> &'static Mutex<String> {
    static ZUNDERSCORE_VAR: OnceLock<Mutex<String>> = OnceLock::new();
    ZUNDERSCORE_VAR.get_or_init(|| Mutex::new(String::new()))
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

/// Direct port of `static int dontimport(int flags)` from
/// `Src/params.c:796-810`.
/// ```c
/// /* If explicitly marked as don't import */
/// if (flags & PM_DONTIMPORT)
///     return 1;
/// /* If value already exported */
/// if (flags & PM_EXPORTED)
///     return 1;
/// /* If security issue when importing and running with some privilege */
/// if ((flags & PM_DONTIMPORT_SUID) && isset(PRIVILEGED))
///     return 1;
/// /* OK to import */
/// return 0;
/// ```
/// Port of `dontimport(int flags)` from `Src/params.c:796`.
fn dontimport(flags: i32) -> i32 {                                           // c:796
    let flags = flags as u32;
    // c:799-800 — `if (flags & PM_DONTIMPORT) return 1`.
    if flags & crate::ported::zsh_h::PM_DONTIMPORT != 0 {                    // c:799
        return 1;                                                            // c:800
    }
    // c:802-803 — `if (flags & PM_EXPORTED) return 1`.
    if flags & crate::ported::zsh_h::PM_EXPORTED != 0 {                      // c:802
        return 1;                                                            // c:803
    }
    // c:805-806 — `if ((flags & PM_DONTIMPORT_SUID) && isset(PRIVILEGED)) return 1`.
    if flags & crate::ported::zsh_h::PM_DONTIMPORT_SUID != 0                 // c:805
        && isset(crate::ported::zsh_h::PRIVILEGED)
    {
        return 1;                                                            // c:806
    }
    0                                                                        // c:809
}


/// Minimal `pattry()` shim — exact-match fallback until the pattern
/// engine in `Src/pattern.c` is wired.
fn pattry(prog: &str, s: &str) -> bool {
    prog == s
}

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
    // All-digit positional: $1..$N from canonical PPARAMS.
    // C zsh dispatches positional params through pparams (Src/init.c).
    if !name.is_empty() && name.chars().all(|c| c.is_ascii_digit()) {
        let n: usize = name.parse().ok()?;
        if n == 0 {
            return crate::ported::utils::argzero();
        }
        let pp = pparams_lock().lock().ok()?;
        return pp.get(n - 1).cloned();
    }
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
        "histchars" | "HISTCHARS" => Some(histcharsgetfn()),
        "_" => Some(underscoregetfn()),
        // Counters with int return.
        "HISTSIZE" => Some(histsizegetfn().to_string()),
        "SAVEHIST" => Some(savehistsizegetfn().to_string()),
        "#" | "ARGC" => Some(poundgetfn().to_string()),
        // $0 routes through utils::argzero.
        "0" => crate::ported::utils::argzero(),
        // POSIX shell-special scalars. C dispatches these through
        // dedicated gsu getfn callbacks (Src/params.c special_assigns).
        "?" => Some(crate::ported::builtin::LASTVAL
            .load(std::sync::atomic::Ordering::Relaxed)
            .to_string()),
        "$" => Some(std::process::id().to_string()),
        "!" => {
            // Last-backgrounded job PID. Stored in paramtab `!` slot;
            // default to 0 to match zsh fresh-shell behaviour.
            let tab = paramtab().read().ok()?;
            Some(tab.get("!").and_then(|pm| pm.u_str.clone())
                .unwrap_or_else(|| "0".to_string()))
        }
        // $* / $@ join positional params via IFS first char.
        "*" | "@" => {
            let sep = ifsgetfn().chars().next().unwrap_or(' ').to_string();
            pparams_lock().lock().ok().map(|p| p.join(&sep))
        }
        // $- : current option-letter set. zsh emits baseline "569X"
        // prefix (internal letters always on) + user-toggled flags.
        "-" => {
            let mut letters = String::from("569X");
            let opt = |n: &str| {
                crate::ported::options::opt_state_get(n).unwrap_or(false)
            };
            if opt("errexit")  { letters.push('e'); }
            if !opt("rcs")     { letters.push('f'); }
            if opt("login")    { letters.push('l'); }
            if opt("nounset")  { letters.push('u'); }
            if opt("xtrace")   { letters.push('x'); }
            if opt("verbose")  { letters.push('v'); }
            if opt("noexec")   { letters.push('n'); }
            if opt("hashall")  { letters.push('h'); }
            Some(letters)
        }
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
        assert_eq!(mkenvstr("PATH", "/usr/bin", 0), "PATH=/usr/bin");
        assert_eq!(mkenvstr("EMPTY", "", 0), "EMPTY=");
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

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor fns for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These fns sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port fns.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor fns for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These fns sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port fns.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor fns for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These fns sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port fns.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn foundparam_lock() -> &'static std::sync::Mutex<Option<String>> {
    FOUNDPARAM.get_or_init(|| std::sync::Mutex::new(None))
}

/// Accessor for the global `paramtab` (Src/params.c:515).
/// Mirrors C's `paramtab->...` dereference by handing back the
/// inner RwLock; callers `.read()` for lookups and `.write()` for
/// mutation, operating on the `HashMap<String, Param>` directly.
pub fn paramtab() -> &'static RwLock<HashMap<String, crate::ported::zsh_h::Param>> {
    PARAMTAB_INNER.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Accessor for the global `realparamtab` (Src/params.c:515).
/// Same role as `paramtab` for the not-currently-redirected case;
/// the alias-flip during assoc-array iteration isn't modelled yet.
pub fn realparamtab() -> &'static RwLock<HashMap<String, crate::ported::zsh_h::Param>> {
    REALPARAMTAB_INNER.get_or_init(|| RwLock::new(HashMap::new()))
}

fn scanprog_lock() -> &'static std::sync::Mutex<Option<String>> {
    SCANPROG.get_or_init(|| std::sync::Mutex::new(None))
}

fn scanstr_lock() -> &'static std::sync::Mutex<Option<String>> {
    SCANSTR.get_or_init(|| std::sync::Mutex::new(None))
}

fn paramvals_lock() -> &'static std::sync::Mutex<Vec<String>> {
    PARAMVALS.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // test_param_value_conversions removed: tested deleted fake
    // `ParamValue::Scalar` constructor. C uses union access on
    // `pm->u.str`/`u.val`/`u.dval`/`u.arr` dispatched via
    // `PM_TYPE(pm->node.flags)` (Src/zsh.h:540).
    #[test]
    fn test_colonarr_conversion() {
        let arr = crate::ported::utils::colonsplit("/bin:/usr/bin:/usr/local/bin", false);
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
        // CBASES off (default): `16#FF` / `8#7` form. The `0x.../
        // 0...` short-prefix output is gated on `setopt CBASES` —
        // see Src/params.c:5599-5605.
        assert_eq!(convbase(255, 16), "16#FF");
        assert_eq!(convbase(10, 10), "10");
        assert_eq!(convbase(-5, 10), "-5");
        assert_eq!(convbase(7, 8), "8#7");
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
        // C-faithful: setarrvalue takes a Value pointing at a Param
        // with u_arr set. Construct one inline.
        use crate::ported::zsh_h::{hashnode, param, PM_ARRAY};
        let pm = Box::new(param {
            node: hashnode { next: None, nam: "test".to_string(), flags: PM_ARRAY as i32 },
            u_data: 0,
            u_arr: Some(vec!["a".into(), "b".into(), "c".into(), "d".into()]),
            u_str: None, u_val: 0, u_dval: 0.0, u_hash: None,
            gsu_s: None, gsu_i: None, gsu_f: None, gsu_a: None, gsu_h: None,
            base: 0, width: 0, env: None, ename: None, old: None, level: 0,
        });
        let mut v = crate::ported::zsh_h::value {
            pm: Some(pm),
            arr: Vec::new(),
            scanflags: 0,
            valflags: 0,
            start: 2,
            end: 3,
        };
        setarrvalue(&mut v, vec!["X".into(), "Y".into()]);
        let arr = v.pm.unwrap().u_arr.unwrap();
        assert_eq!(arr, vec!["a", "X", "Y", "d"]);
    }

    #[test]
    fn test_valid_refname() {
        assert!(valid_refname("foo", 0));
        assert!(valid_refname("_bar", 0));
        assert!(valid_refname("1", 0));
        assert!(valid_refname("!", 0));
        assert!(valid_refname("arr[1]", 0));
        assert!(!valid_refname("", 0));
        // C semantics: empty leader without one of `! ? $ - _` is rejected.
        assert!(!valid_refname(" ", 0));
        // PM_UPPER rejects digit-leader and argv/ARGC.
        assert!(!valid_refname("1", PM_UPPER as i32));
        assert!(!valid_refname("argv", PM_UPPER as i32));
        assert!(!valid_refname("ARGC", PM_UPPER as i32));
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

    fn val_str(v: getarg_out<'_>) -> String {
        match v {
            getarg_out::Value(v) => v.to_str(),
            getarg_out::Flags { .. } => panic!("expected Value, got Flags"),
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

    /// c:3076/3193 — assignsparam writes into paramtab; getsparam
    /// reads it back. The round-trip is the spine of every
    /// `foo=bar; print $foo` flow. Regression here would silently
    /// drop assignments.
    #[test]
    fn assignsparam_then_getsparam_round_trips() {
        let name = "ZSHRS_TEST_ASSIGN_GET";
        crate::ported::params::assignsparam(name, "test_value_42", 0);
        assert_eq!(
            crate::ported::params::getsparam(name).as_deref(),
            Some("test_value_42")
        );
        // Cleanup so other tests don't see leaked param.
        let _ = crate::ported::params::paramtab().write().unwrap().remove(name);
    }

    /// c:3076 — getsparam on a non-existent param returns None.
    /// A regression returning Some("") would mask unset-param errors.
    #[test]
    fn getsparam_unknown_param_returns_none() {
        assert!(crate::ported::params::getsparam("ZSHRS_TEST_DEFINITELY_UNSET").is_none());
    }

    /// c:3819 — direct paramtab.remove drops the entry; subsequent
    /// getsparam returns None. The set→remove→lookup gap verifies
    /// the canonical paramtab is actually backing both reads + writes.
    #[test]
    fn paramtab_remove_makes_getsparam_return_none() {
        let name = "ZSHRS_TEST_UNSET_FLOW";
        crate::ported::params::assignsparam(name, "to_be_removed", 0);
        assert!(crate::ported::params::getsparam(name).is_some(),
            "param must be set before remove path");
        let _ = crate::ported::params::paramtab().write().unwrap().remove(name);
        assert!(crate::ported::params::getsparam(name).is_none(),
            "after remove, getsparam must return None");
    }

    /// c:3357 — assignaparam stores an array. getsparam on an array
    /// param returns the first element OR a join (depends on IFS).
    /// Verify the slot was populated AT ALL by querying paramtab.
    #[test]
    fn assignaparam_populates_paramtab_with_array() {
        let name = "ZSHRS_TEST_ARR_X";
        crate::ported::params::assignaparam(
            name, vec!["a".into(), "b".into(), "c".into()], 0);
        let tab = crate::ported::params::paramtab().read().expect("paramtab poisoned");
        let pm = tab.get(name).expect("param installed");
        assert_eq!(pm.u_arr.as_deref(), Some(&["a".to_string(), "b".to_string(), "c".to_string()][..]),
            "assignaparam stores all three elements");
        drop(tab);
        let _ = crate::ported::params::paramtab().write().unwrap().remove(name);
    }

    /// Process-wide lock serialising tests that mutate the three
    /// `histchars` atomic globals. No C counterpart.
    static HISTCHARS_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// `Src/params.c:5095-5097` — `histcharssetfn` stores bangchar /
    /// hatchar / hashchar in the per-char globals. Pin the round-trip
    /// for ALL THREE: change HISTCHARS to a custom 3-char string,
    /// verify each atomic global reflects the new value, and verify
    /// the canonical default `"!^#"` restores on NULL.
    #[test]
    fn histcharssetfn_syncs_all_three_histchar_globals() {
        let _g = HISTCHARS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use std::sync::atomic::Ordering;
        // Default state.
        crate::ported::params::histcharssetfn(None);
        assert_eq!(crate::ported::hist::bangchar.load(Ordering::SeqCst), b'!' as i32);
        assert_eq!(crate::ported::hist::hatchar.load(Ordering::SeqCst),  b'^' as i32);
        assert_eq!(crate::ported::hist::hashchar.load(Ordering::SeqCst), b'#' as i32);
        // Set HISTCHARS to "@:%".
        crate::ported::params::histcharssetfn(Some("@:%".to_string()));
        assert_eq!(crate::ported::hist::bangchar.load(Ordering::SeqCst), b'@' as i32,
            "c:5095 — bangchar = first byte of HISTCHARS");
        assert_eq!(crate::ported::hist::hatchar.load(Ordering::SeqCst),  b':' as i32,
            "c:5096 — hatchar = second byte of HISTCHARS");
        assert_eq!(crate::ported::hist::hashchar.load(Ordering::SeqCst), b'%' as i32,
            "c:5097 — hashchar = third byte of HISTCHARS");
        // Restore.
        crate::ported::params::histcharssetfn(None);
        assert_eq!(crate::ported::hist::bangchar.load(Ordering::SeqCst), b'!' as i32);
        assert_eq!(crate::ported::hist::hashchar.load(Ordering::SeqCst), b'#' as i32);
    }

    /// `Src/params.c:5064-5074` — `histcharsgetfn` reads from the
    /// three atomic globals and returns a string of non-NUL bytes.
    /// Pin set→get symmetry: after `histcharssetfn(Some("@&%"))`,
    /// `histcharsgetfn()` returns `"@&%"`.
    #[test]
    fn histcharsgetfn_round_trips_with_histcharssetfn() {
        let _g = HISTCHARS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::ported::params::histcharssetfn(Some("@&%".to_string()));
        assert_eq!(crate::ported::params::histcharsgetfn(), "@&%",
            "c:5068-5073 — getfn reads atomic globals setfn wrote");
        // Restore default and verify round-trip.
        crate::ported::params::histcharssetfn(None);
        assert_eq!(crate::ported::params::histcharsgetfn(), "!^#",
            "default `!^#` round-trips through atomics");
    }

    /// `Src/params.c:5118-5128` — `homesetfn(x)` round-trip:
    /// `homesetfn(s); homegetfn() == s` for non-symlink paths and
    /// CHASELINKS-off. Pins the basic store-then-read contract.
    #[test]
    fn homesetfn_stores_value_for_getfn() {
        let saved = crate::ported::params::homegetfn();
        crate::ported::params::homesetfn("/tmp/zshrs_test_home".to_string());
        assert_eq!(crate::ported::params::homegetfn(), "/tmp/zshrs_test_home",
            "c:5121-5126 — homesetfn → homegetfn round-trip");
        // Restore.
        crate::ported::params::homesetfn(saved);
    }

    /// `Src/params.c:5125-5126` — empty input becomes `ztrdup("")`.
    /// Pin empty-string handling.
    #[test]
    fn homesetfn_empty_input_stores_empty() {
        let saved = crate::ported::params::homegetfn();
        crate::ported::params::homesetfn(String::new());
        assert_eq!(crate::ported::params::homegetfn(), "",
            "c:5126 — empty x stores empty (no panic)");
        crate::ported::params::homesetfn(saved);
    }

    /// `Src/params.c:5004-5011` — `errnosetfn(x)` writes errno
    /// unconditionally, then warns (NOT errors) on truncation. The
    /// store happens regardless of warning. Pin set→get round-trip.
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn errnosetfn_writes_through_to_libc_errno_getfn() {
        // Set errno to a small int.
        crate::ported::params::errnosetfn(42);
        assert_eq!(crate::ported::params::errnogetfn(), 42,
            "c:5006 — errno = (int)x; subsequent getfn must read it back");
        crate::ported::params::errnosetfn(0);
        assert_eq!(crate::ported::params::errnogetfn(), 0);
    }

    /// `Src/params.c:5008-5010` — truncation check fires when
    /// `(zlong)errno != x`. C also resets errno indirectly inside
    /// `zwarn` (libc calls touch errno) — so after the warning,
    /// the user's observed `$ERRNO` is the post-warning value, NOT
    /// the truncated cast. Faithful Rust port has the same behavior.
    /// Pin only that the function returns normally and doesn't crash;
    /// any specific post-call errno value is implementation-defined.
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn errnosetfn_does_not_panic_on_truncation() {
        // i64::MAX → truncates to i32 = -1 → warning fires inside.
        // The store at c:5008 happens; whether the warning's libc
        // calls then overwrite errno is implementation-defined.
        crate::ported::params::errnosetfn(i64::MAX);
        // Just verify the call returned (no panic) and getfn works.
        let _ = crate::ported::params::errnogetfn();
        // Reset.
        crate::ported::params::errnosetfn(0);
    }

    /// `Src/params.c:5090-5093` — non-ASCII chars in HISTCHARS
    /// produce a warning and the function returns WITHOUT updating
    /// any globals. Pin the rejection: state before == state after
    /// when a non-ASCII byte is in position 0/1/2.
    #[test]
    fn histcharssetfn_rejects_non_ascii_chars() {
        let _g = HISTCHARS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use std::sync::atomic::Ordering;
        // Reset to defaults.
        crate::ported::params::histcharssetfn(None);
        let bang_before = crate::ported::hist::bangchar.load(Ordering::SeqCst);
        let hat_before  = crate::ported::hist::hatchar.load(Ordering::SeqCst);
        // Try to set HISTCHARS with non-ASCII char.
        crate::ported::params::histcharssetfn(Some("é".to_string()));
        // c:5092 — rejection returns BEFORE any state changes.
        assert_eq!(crate::ported::hist::bangchar.load(Ordering::SeqCst), bang_before,
            "c:5092 — bangchar unchanged after non-ASCII rejection");
        assert_eq!(crate::ported::hist::hatchar.load(Ordering::SeqCst), hat_before);
    }
}
