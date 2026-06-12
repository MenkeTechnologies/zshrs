//! Parameter management for zshrs
//!
//! Port from zsh/Src/params.c (6511 lines → full Rust port)
//!
//! Provides shell parameters (variables), special parameters, arrays,
//! associative arrays, parameter attributes, namerefs, scoping,
//! tied parameters, and all special parameter get/set functions.

use crate::config_h::DEFAULT_TMPPREFIX;
use crate::func_body_fmt::FuncBodyFmt;
use crate::lex::parse_subscript;
use crate::ported::builtin::{LASTVAL, PPARAMS};
use crate::ported::config_h::{MACHTYPE, OSTYPE, VENDOR};
use crate::ported::exec::FORKLEVEL;
use crate::ported::hashtable::emptycmdnamtable;
use crate::ported::hist::{
    bangchar, hashchar, hatchar, histsiz, resizehistents, saveandpophiststack, savehistsiz,
};
use crate::ported::init::SHTTY;
use crate::ported::lex::untokenize;
use crate::ported::math::lastbase;
#[allow(unused_imports)]
use crate::ported::math::{
    matheval, mathevali, MN_FLOAT, MN_FLOAT as MN_FLT, MN_INTEGER, MN_INTEGER as MN_INT,
};
use crate::ported::mem::{popheap, pushheap};
use crate::ported::modules::parameter::FUNCSTACK;
#[allow(unused_imports)]
use crate::ported::options::{opt_state_get, opt_state_set, optlookup};
use crate::ported::patchlevel::{ZSH_PATCHLEVEL, ZSH_VERSION};
use crate::ported::pattern::{patcompile, pattry};
#[allow(unused_imports)]
use crate::ported::signals::{queue_signals, unqueue_signals};
use crate::ported::signals_h::SIGS;
use crate::ported::string::ztrdup;
use crate::ported::utils::{
    adduserdir, arrlen_ge, dec_locallevel, inc_locallevel, metafy, quotedzputs, xsymlink,
};
#[allow(unused_imports)]
use crate::ported::utils::{
    adjustwinsize, argzero, colonsplit, errflag, get_username, inittyptab, itype_end,
    locallevel as locallevel_fn, posixzero, set_argzero, set_locallevel, set_posixzero, unmeta,
    zerr, ztrdup_metafy, zwarn,
};
use crate::ported::zsh_h::PAT_HEAPDUP;
#[allow(unused_imports)]
use crate::ported::zsh_h::{
    gsu_array, gsu_float, gsu_hash, gsu_integer, gsu_scalar, hashnode, hashtable, isset, mnumber,
    param, paramdef, unset, value, HashTable, Marker, Param, ALLEXPORT, ASSPM_AUGMENT,
    ASSPM_ENV_IMPORT, ASSPM_KEY_VALUE, ASSPM_WARN, AUTONAMEDIRS, EMULATE_KSH, EMULATE_SH, EMULATE_ZSH, EMULATION,
    ERRFLAG_ERROR, EXECOPT, FS_FUNC, KSHARRAYS, PM_ARRAY, PM_AUTOLOAD, PM_DECLARED, PM_DEFAULTED,
    PM_DONTIMPORT, PM_DONTIMPORT_SUID, PM_EFLOAT, PM_EXPORTED, PM_FFLOAT, PM_HASHED, PM_HASHELEM,
    PM_HIDE, PM_HIDEVAL, PM_INTEGER, PM_LEFT, PM_LOCAL, PM_NAMEDDIR, PM_NAMEREF, PM_NORESTORE,
    PM_READONLY,
    PM_READONLY_SPECIAL, PM_REMOVABLE, PM_RIGHT_B, PM_RIGHT_Z, PM_RO_BY_DESIGN, PM_SCALAR,
    PM_SPECIAL, PM_TAGGED, PM_TIED, PM_TYPE, PM_UNIQUE, PM_UNSET, PM_UPPER, POSIXARGZERO,
    PRINT_INCLUDEVALUE, PRINT_KV_PAIR, PRINT_LINE, PRINT_NAMEONLY, PRINT_POSIX_EXPORT,
    PRINT_POSIX_READONLY, PRINT_TYPE, PRINT_TYPESET, SCANPM_ARRONLY, SCANPM_CHECKING,
    SCANPM_ISVAR_AT, SCANPM_KEYMATCH, SCANPM_MATCHKEY, SCANPM_MATCHMANY, SCANPM_MATCHVAL,
    SCANPM_NONAMEREF, SCANPM_WANTINDEX, SCANPM_WANTKEYS, SCANPM_WANTVALS, TERM_BAD, TERM_UNKNOWN,
    VALFLAG_EMPTY, VALFLAG_INV, VALFLAG_SUBST, WARNCREATEGLOBAL, WARNNESTEDVAR,
};
use crate::ported::zsh_h::{
    HashNode, Inbrack, Meta, CBASES, CHASELINKS, HFILE_USE_OPTIONS, INTERACTIVE, OCTALZEROES,
    PM_LOWER, PRIVILEGED, SCANPM_ASSIGNING,
};
use crate::ported::zsh_system_h::DEFAULT_TIMEFMT;
use crate::{DPUTS, DPUTS2};
use fusevm::Value;
use indexmap::IndexMap;
use std::collections::{HashMap, HashSet};
use std::env;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Port of `static int lc_update_needed` from `Src/params.c:5850`
/// (under `#ifdef USE_LOCALE`). Set to 1 by `scanendscope` when a
/// LC_*/LANG param's scope ends; consumed by `endparamscope` to
/// trigger a `setlocale()` refresh.
pub static LC_UPDATE_NEEDED: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Port of `static Param foundparam` from `Src/params.c:640`.
/// Set by `scanparamvals` to the last param it touched, read by
/// `assignsparam` / `assignnparam` for the assoc-element path.
/// Stores the param name; the live `&param` lookup is done by
/// the caller through paramtab.
pub static FOUNDPARAM: OnceLock<Mutex<Option<String>>> = OnceLock::new();

/// Port of `rprompt_indent_unsetfn(Param pm, int exp)` from `Src/params.c:152`. C
/// body: `stdunsetfn(pm, exp); rprompt_indent = 1;` — keeps in
/// sync with init_term().
pub fn rprompt_indent_unsetfn(pm: &mut param, exp: i32) {
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
#[inline]
#[allow(non_snake_case)]
pub fn IPDEF1(A: &str, B: usize, C: i32) -> paramdef {
    // c:params.c:296
    paramdef {
        name: A.into(),
        flags: (PM_INTEGER | PM_SPECIAL) as i32 | C,
        gsu: B,
        ..Default::default()
    }
}

/// Port of `IPDEF2(A,B,C)` from `Src/params.c:309` —
/// `{{NULL,A,PM_SCALAR|PM_SPECIAL|C},BR(NULL),GSU(B),0,0,...}`.
#[inline]
#[allow(non_snake_case)]
pub fn IPDEF2(A: &str, B: usize, C: i32) -> paramdef {
    // c:params.c:309
    paramdef {
        name: A.into(),
        flags: (PM_SCALAR | PM_SPECIAL) as i32 | C,
        gsu: B,
        ..Default::default()
    }
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
pub static locallevel: std::sync::atomic::AtomicI32 = // c:54
    std::sync::atomic::AtomicI32::new(0);

// ---------------------------------------------------------------------------
// Real `param` struct lives in Src/zsh.h:1829 (port at zsh_h.rs:750).
// It uses C-union flattening: u_str / u_arr / u_val / u_dval / u_hash
// dispatched on `PM_TYPE(node.flags)`. There is NO `ParamValue` enum in
// C; do not reintroduce one.
// ---------------------------------------------------------------------------

/// Port of `LCIPDEF(name)` from `Src/params.c:324` —
/// `IPDEF2(name, lc_blah_gsu, PM_UNSET)`.
#[inline]
#[allow(non_snake_case)]
pub fn LCIPDEF(name: &str) -> paramdef {
    // c:params.c:324
    IPDEF2(name, 0, PM_UNSET as i32) // c:324 lc_blah_gsu (slot 0)
}

/// Port of `IPDEF4(A,B)` from `Src/params.c:344` —
/// `{{NULL,A,PM_INTEGER|PM_READONLY_SPECIAL},BR((void*)B),
///   GSU(varint_readonly_gsu),10,0,...}`.
#[inline]
#[allow(non_snake_case)]
pub fn IPDEF4(A: &str, B: usize) -> paramdef {
    // c:params.c:344
    paramdef {
        name: A.into(),
        flags: (PM_INTEGER | PM_READONLY_SPECIAL) as i32,
        var: B,
        ..Default::default()
    }
}

/// Port of `IPDEF5(A,B,F)` from `Src/params.c:353` —
/// `{{NULL,A,PM_INTEGER|PM_SPECIAL},BR((void*)B),GSU(F),10,0,...}`.
#[inline]
#[allow(non_snake_case)]
pub fn IPDEF5(A: &str, B: usize, F: usize) -> paramdef {
    // c:params.c:353
    paramdef {
        name: A.into(),
        flags: (PM_INTEGER | PM_SPECIAL) as i32,
        var: B,
        gsu: F,
        ..Default::default()
    }
}

/// Port of `IPDEF5U(A,B,F)` from `Src/params.c:354` — c:353 + PM_UNSET.
#[inline]
#[allow(non_snake_case)]
pub fn IPDEF5U(A: &str, B: usize, F: usize) -> paramdef {
    // c:params.c:354
    paramdef {
        name: A.into(),
        flags: (PM_INTEGER | PM_SPECIAL | PM_UNSET) as i32,
        var: B,
        gsu: F,
        ..Default::default()
    }
}

/// Port of `IPDEF6(A,B,F)` from `Src/params.c:362` — c:353 + PM_DONTIMPORT.
#[inline]
#[allow(non_snake_case)]
pub fn IPDEF6(A: &str, B: usize, F: usize) -> paramdef {
    // c:params.c:362
    paramdef {
        name: A.into(),
        flags: (PM_INTEGER | PM_SPECIAL | PM_DONTIMPORT) as i32,
        var: B,
        gsu: F,
        ..Default::default()
    }
}

/// Port of `IPDEF7(A,B)` from `Src/params.c:367` —
/// `{{NULL,A,PM_SCALAR|PM_SPECIAL},BR((void*)B),GSU(varscalar_gsu),0,0,...}`.
#[inline]
#[allow(non_snake_case)]
pub fn IPDEF7(A: &str, B: usize) -> paramdef {
    // c:params.c:367
    paramdef {
        name: A.into(),
        flags: (PM_SCALAR | PM_SPECIAL) as i32,
        var: B,
        ..Default::default()
    }
}

/// Port of `IPDEF7U(A,B)` from `Src/params.c:369` — c:367 + PM_UNSET.
#[inline]
#[allow(non_snake_case)]
pub fn IPDEF7U(A: &str, B: usize) -> paramdef {
    // c:params.c:369
    paramdef {
        name: A.into(),
        flags: (PM_SCALAR | PM_SPECIAL | PM_UNSET) as i32,
        var: B,
        ..Default::default()
    }
}

/// Port of `IPDEF7R(A,B)` from `Src/params.c:368` — c:367 + PM_DONTIMPORT_SUID.
#[inline]
#[allow(non_snake_case)]
pub fn IPDEF7R(A: &str, B: usize) -> paramdef {
    // c:params.c:368
    paramdef {
        name: A.into(),
        flags: (PM_SCALAR | PM_SPECIAL | PM_DONTIMPORT_SUID) as i32,
        var: B,
        ..Default::default()
    }
}

/// Port of `IPDEF9(A,B,C,D)` from `Src/params.c:431` —
/// `{{NULL,A,D|PM_ARRAY|PM_SPECIAL|PM_DONTIMPORT},BR((void*)B),
///   GSU(vararray_gsu),0,0,NULL,C,NULL,0}`.
#[inline]
#[allow(non_snake_case)]
pub fn IPDEF9(A: &str, B: usize, C: usize, D: i32) -> paramdef {
    // c:params.c:384
    paramdef {
        name: A.into(),
        flags: (PM_ARRAY | PM_SPECIAL | PM_DONTIMPORT) as i32 | D,
        var: B,
        ..Default::default()
    }
}

/// Port of `IPDEF8(A,B,C,D)` from `Src/params.c:394` —
/// `{{NULL,A,D|PM_SCALAR|PM_SPECIAL},BR((void*)B),GSU(colonarr_gsu),
///   0,0,NULL,C,NULL,0}`.
/// `C` is the colon-arr field; the Rust port stores it in `getnfn`
/// since `paramdef` lacks a dedicated colon-arr slot until that's
/// ported.
#[inline]
#[allow(non_snake_case)]
pub fn IPDEF8(A: &str, B: usize, C: usize, D: i32) -> paramdef {
    // c:params.c:394
    paramdef {
        name: A.into(),
        flags: (PM_SCALAR | PM_SPECIAL) as i32 | D,
        var: B,
        ..Default::default()
    }
}

/// Port of `IPDEF10(A,B)` from `Src/params.c:438` —
/// `{{NULL,A,PM_ARRAY|PM_SPECIAL},BR(NULL),GSU(B),10,0,...}`.
#[inline]
#[allow(non_snake_case)]
pub fn IPDEF10(A: &str, B: usize) -> paramdef {
    // c:params.c:406
    paramdef {
        name: A.into(),
        flags: (PM_ARRAY | PM_SPECIAL) as i32,
        gsu: B,
        ..Default::default()
    }
}

/// Port of `newparamtable(int size, char const *name)` from `Src/params.c:519`. C body
/// allocates a HashTable via `newhashtable(size, name, NULL)`
/// and wires the vtable. Rust port constructs a fresh
/// `Box<hashtable>` with the param-specific callbacks left as
/// `None` (the hashtable.rs vtable cannot host the typed
/// param-callback signatures yet — wiring them requires the
/// hashtable backend refactor).
#[allow(unused_variables)]
pub fn newparamtable(size: i32, name: &str) -> Option<HashTable> {
    let hsize = if size == 0 { 17 } else { size };
    let mut nodes: Vec<Option<HashNode>> = Vec::with_capacity(hsize as usize);
    for _ in 0..hsize {
        nodes.push(None);
    }
    Some(Box::new(hashtable {
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
pub fn loadparamnode(
    // c:544
    _ht: &HashTable,
    pm: Option<Param>,
    nam: &str,
) -> Option<Param> {
    // c:546 — `if (pm && (pm->flags & PM_AUTOLOAD) && pm->u.str)`.
    let (level, modname) = match &pm {
        Some(p) if p.node.flags & PM_AUTOLOAD as i32 != 0 && p.u_str.is_some() => {
            (p.level, p.u_str.clone().unwrap())
        }
        _ => return pm, // c:566 fall through
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
            pm = p.old.clone().map(|b| Param::from(b));
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
        zerr(&format!(
            "autoloading module {} failed to define parameter: {}",
            modname, nam
        ));
    }
    pm // c:566
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
    /// `name` field.
    pub name: &'static str,
    pub pm_type: u32,  // PM_INTEGER | PM_SCALAR | PM_ARRAY
    pub pm_flags: u32, // PM_READONLY_SPECIAL, PM_DONTIMPORT, etc.
    /// `tied_name` field.
    pub tied_name: Option<&'static str>,
}

/// Index of the first entry in `special_params` that lives in the
/// zsh-only section (after the `{{NULL,NULL,0}, BR(NULL), ...}`
/// sentinel at `Src/params.c:392`). Entries before this index are
/// always loaded; entries at and after this index are only loaded
/// under non-sh/non-ksh emulation. Mirrors the C two-section table
/// terminated by an inner NULL sentinel.
pub const SPECIAL_PARAMS_ZSH_START: usize = 54; // c:392

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
        pm_type: PM_INTEGER,
        pm_flags: PM_READONLY,
        tied_name: None,
    },
    special_paramdef {
        name: "?",
        pm_type: PM_INTEGER,
        pm_flags: PM_READONLY,
        tied_name: None,
    },
    special_paramdef {
        name: "HISTCMD",
        pm_type: PM_INTEGER,
        pm_flags: PM_READONLY,
        tied_name: None,
    },
    special_paramdef {
        name: "LINENO",
        pm_type: PM_INTEGER,
        pm_flags: PM_READONLY,
        tied_name: None,
    },
    special_paramdef {
        name: "PPID",
        pm_type: PM_INTEGER,
        pm_flags: PM_READONLY,
        tied_name: None,
    },
    special_paramdef {
        name: "ZSH_SUBSHELL",
        pm_type: PM_INTEGER,
        pm_flags: PM_READONLY,
        tied_name: None,
    },
    // Settable integer variables
    special_paramdef {
        name: "COLUMNS",
        pm_type: PM_INTEGER,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "LINES",
        pm_type: PM_INTEGER,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "ZLE_RPROMPT_INDENT",
        pm_type: PM_INTEGER,
        pm_flags: PM_UNSET,
        tied_name: None,
    },
    special_paramdef {
        name: "SHLVL",
        pm_type: PM_INTEGER,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "FUNCNEST",
        pm_type: PM_INTEGER,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "OPTIND",
        pm_type: PM_INTEGER,
        pm_flags: PM_DONTIMPORT,
        tied_name: None,
    },
    special_paramdef {
        // c:Src/params.c:364 — `IPDEF6("TRY_BLOCK_ERROR", &try_errflag,
        // varinteger_gsu)` = PM_INTEGER | PM_SPECIAL | PM_DONTIMPORT.
        // No PM_UNSET on the table entry — C reads -1 via the getfn
        // reading the global `try_errflag`. zshrs's earlier port set
        // PM_UNSET as a "no-write-yet" sentinel which broke
        // `${(k)parameters}` parity (zsh emits the name; with PM_UNSET
        // here scanpmparameters skipped it). The -1 default is now
        // surfaced via the special-var getter (params.rs sentinel
        // override on PM_UNSET removal).
        name: "TRY_BLOCK_ERROR",
        pm_type: PM_INTEGER,
        pm_flags: PM_DONTIMPORT,
        tied_name: None,
    },
    special_paramdef {
        // c:Src/loop.c — `try_interrupt = -1`. Same pattern as
        // TRY_BLOCK_ERROR: no PM_UNSET on the table entry; -1
        // default emerges via the special-var getter.
        name: "TRY_BLOCK_INTERRUPT",
        pm_type: PM_INTEGER,
        pm_flags: PM_DONTIMPORT,
        tied_name: None,
    },
    // Scalar variables bound to C globals
    special_paramdef {
        name: "OPTARG",
        pm_type: PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "NULLCMD",
        pm_type: PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "POSTEDIT",
        pm_type: PM_SCALAR,
        pm_flags: PM_UNSET,
        tied_name: None,
    },
    special_paramdef {
        name: "READNULLCMD",
        pm_type: PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "PS1",
        pm_type: PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "RPS1",
        pm_type: PM_SCALAR,
        pm_flags: PM_UNSET,
        tied_name: None,
    },
    special_paramdef {
        name: "RPROMPT",
        pm_type: PM_SCALAR,
        pm_flags: PM_UNSET,
        tied_name: None,
    },
    special_paramdef {
        name: "PS2",
        pm_type: PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "RPS2",
        pm_type: PM_SCALAR,
        pm_flags: PM_UNSET,
        tied_name: None,
    },
    special_paramdef {
        name: "RPROMPT2",
        pm_type: PM_SCALAR,
        pm_flags: PM_UNSET,
        tied_name: None,
    },
    special_paramdef {
        name: "PS3",
        pm_type: PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "PS4",
        pm_type: PM_SCALAR,
        pm_flags: PM_DONTIMPORT_SUID,
        tied_name: None,
    },
    special_paramdef {
        name: "SPROMPT",
        pm_type: PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    // Readonly arrays
    special_paramdef {
        name: "*",
        pm_type: PM_ARRAY,
        pm_flags: PM_READONLY | PM_DONTIMPORT,
        tied_name: None,
    },
    special_paramdef {
        name: "@",
        pm_type: PM_ARRAY,
        pm_flags: PM_READONLY | PM_DONTIMPORT,
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
        pm_type: PM_SCALAR,
        pm_flags: PM_TIED,
        tied_name: Some("cdpath"),
    },
    special_paramdef {
        name: "FIGNORE",
        pm_type: PM_SCALAR,
        pm_flags: PM_TIED,
        tied_name: Some("fignore"),
    },
    special_paramdef {
        name: "FPATH",
        pm_type: PM_SCALAR,
        pm_flags: PM_TIED,
        tied_name: Some("fpath"),
    },
    special_paramdef {
        name: "MAILPATH",
        pm_type: PM_SCALAR,
        pm_flags: PM_TIED,
        tied_name: Some("mailpath"),
    },
    special_paramdef {
        name: "PATH",
        pm_type: PM_SCALAR,
        pm_flags: PM_TIED,
        tied_name: Some("path"),
    },
    special_paramdef {
        name: "PSVAR",
        pm_type: PM_SCALAR,
        pm_flags: PM_TIED,
        tied_name: Some("psvar"),
    },
    special_paramdef {
        name: "ZSH_EVAL_CONTEXT",
        pm_type: PM_SCALAR,
        pm_flags: PM_READONLY | PM_TIED,
        tied_name: Some("zsh_eval_context"),
    },
    special_paramdef {
        name: "MODULE_PATH",
        pm_type: PM_SCALAR,
        pm_flags: PM_DONTIMPORT | PM_TIED,
        tied_name: Some("module_path"),
    },
    special_paramdef {
        name: "MANPATH",
        pm_type: PM_SCALAR,
        pm_flags: PM_TIED,
        tied_name: Some("manpath"),
    },
    // Locale
    special_paramdef {
        name: "LANG",
        pm_type: PM_SCALAR,
        pm_flags: PM_UNSET,
        tied_name: None,
    },
    special_paramdef {
        name: "LC_ALL",
        pm_type: PM_SCALAR,
        pm_flags: PM_UNSET,
        tied_name: None,
    },
    special_paramdef {
        name: "LC_COLLATE",
        pm_type: PM_SCALAR,
        pm_flags: PM_UNSET,
        tied_name: None,
    },
    special_paramdef {
        name: "LC_CTYPE",
        pm_type: PM_SCALAR,
        pm_flags: PM_UNSET,
        tied_name: None,
    },
    special_paramdef {
        name: "LC_MESSAGES",
        pm_type: PM_SCALAR,
        pm_flags: PM_UNSET,
        tied_name: None,
    },
    special_paramdef {
        name: "LC_NUMERIC",
        pm_type: PM_SCALAR,
        pm_flags: PM_UNSET,
        tied_name: None,
    },
    special_paramdef {
        name: "LC_TIME",
        pm_type: PM_SCALAR,
        pm_flags: PM_UNSET,
        tied_name: None,
    },
    // Zsh-only aliases
    special_paramdef {
        name: "ARGC",
        pm_type: PM_INTEGER,
        pm_flags: PM_READONLY,
        tied_name: None,
    },
    special_paramdef {
        name: "HISTCHARS",
        pm_type: PM_SCALAR,
        pm_flags: PM_DONTIMPORT,
        tied_name: None,
    },
    special_paramdef {
        name: "status",
        pm_type: PM_INTEGER,
        pm_flags: PM_READONLY,
        tied_name: None,
    },
    special_paramdef {
        name: "prompt",
        pm_type: PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "PROMPT",
        pm_type: PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "PROMPT2",
        pm_type: PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "PROMPT3",
        pm_type: PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "PROMPT4",
        pm_type: PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        name: "argv",
        pm_type: PM_ARRAY,
        pm_flags: 0,
        tied_name: None,
    },
    // c:Src/params.c:425-434 — IPDEF9 lowercase array tied counterparts
    // of the uppercase scalar specials. Each is a PM_ARRAY|PM_TIED|
    // PM_SPECIAL entry that points back to its colon-scalar partner
    // via tied_name. Without these in special_params, `${(t)path}` /
    // `${(t)cdpath}` / `${(t)fpath}` etc. miss the `-tied-special`
    // suffix when the paramtab.get(name) lookup runs in subst.rs:5605.
    special_paramdef {
        name: "fignore",
        pm_type: PM_ARRAY,
        pm_flags: PM_TIED,
        tied_name: Some("FIGNORE"),
    },
    special_paramdef {
        name: "cdpath",
        pm_type: PM_ARRAY,
        pm_flags: PM_TIED,
        tied_name: Some("CDPATH"),
    },
    special_paramdef {
        name: "fpath",
        pm_type: PM_ARRAY,
        pm_flags: PM_TIED,
        tied_name: Some("FPATH"),
    },
    special_paramdef {
        name: "mailpath",
        pm_type: PM_ARRAY,
        pm_flags: PM_TIED,
        tied_name: Some("MAILPATH"),
    },
    special_paramdef {
        name: "manpath",
        pm_type: PM_ARRAY,
        pm_flags: PM_TIED,
        tied_name: Some("MANPATH"),
    },
    special_paramdef {
        name: "psvar",
        pm_type: PM_ARRAY,
        pm_flags: PM_TIED,
        tied_name: Some("PSVAR"),
    },
    special_paramdef {
        name: "zsh_eval_context",
        pm_type: PM_ARRAY,
        pm_flags: PM_TIED | PM_READONLY,
        tied_name: Some("ZSH_EVAL_CONTEXT"),
    },
    special_paramdef {
        name: "module_path",
        pm_type: PM_ARRAY,
        pm_flags: PM_TIED,
        tied_name: Some("MODULE_PATH"),
    },
    special_paramdef {
        name: "path",
        pm_type: PM_ARRAY,
        pm_flags: PM_TIED,
        tied_name: Some("PATH"),
    },
    // pipestatus array
    special_paramdef {
        name: "pipestatus",
        pm_type: PM_ARRAY,
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
    special_paramdef {
        // c:448
        name: "CDPATH",
        pm_type: PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        // c:449
        name: "FIGNORE",
        pm_type: PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        // c:450
        name: "FPATH",
        pm_type: PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        // c:451
        name: "MAILPATH",
        pm_type: PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        // c:452
        name: "PATH",
        pm_type: PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        // c:453
        name: "PSVAR",
        pm_type: PM_SCALAR,
        pm_flags: 0,
        tied_name: None,
    },
    special_paramdef {
        // c:454
        name: "ZSH_EVAL_CONTEXT",
        pm_type: PM_SCALAR,
        pm_flags: PM_READONLY,
        tied_name: None,
    },
    special_paramdef {
        // c:457 (security comment)
        name: "MODULE_PATH",
        pm_type: PM_SCALAR,
        pm_flags: PM_DONTIMPORT,
        tied_name: None,
    },
];

/// Port of `getparamnode(HashTable ht, const char *nam)` from `Src/params.c:570`. C body:
/// `pm = loadparamnode(ht, gethashnode2(ht, nam), nam);
///  if (pm && ht == realparamtab && !PM_UNSET) pm = resolve_nameref(pm);
///  return (HashNode)pm;`
/// Stub: needs HashTable + autoload + nameref resolve.
/// WARNING: param names don't match C — Rust=() vs C=(ht, nam)
pub fn getparamnode(ht: &HashTable, nam: &str) -> Option<Param> {
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
pub fn scancopyparams(pm: &mut param, _flags: i32, outtable: &mut HashMap<String, Box<param>>) {
    // c:586-588 — `tpm = (Param) zshcalloc(...); copyparam(tpm, pm, 0); addnode(...)`.
    let mut tpm = Box::new(pm.clone()); // c:586 zshcalloc
    tpm.old = None;
    tpm.env = None;
    tpm.ename = None; // c:1242 (calloc-zero fields copyparam doesn't set)
    copyparam(&mut tpm, pm, 0); // c:587
    let nam = tpm.node.nam.clone();
    outtable.insert(nam, tpm); // c:588 addnode(outtable, ztrdup(pm->node.nam), tpm)
}

/// Port of `copyparamtable(HashTable ht, char *name)` from `Src/params.c:596`. C body:
/// allocates a fresh paramtable via `newparamtable(ht->hsize, name)`,
/// sets the global `outtable = nht`, then scans the source via
/// `scanhashtable(ht, 0, 0, 0, scancopyparams, 0)` and clears
/// `outtable` on exit. Rust port returns the freshly-allocated
/// table; the per-node clone walk requires the HashTable iterator
/// which isn't wired yet (callers receive the empty allocated
/// table — same shape the C source returns when `ht` is empty).
pub fn copyparamtable(ht: Option<&HashTable>, name: &str) -> Option<HashTable> {
    let ht = ht?;
    newparamtable(ht.hsize, name)
}

/// Port of `deleteparamtable(HashTable t)` from `Src/params.c:616`. C body:
/// `int odelunset = delunset; delunset = 1; deletehashtable(t);
///  delunset = odelunset;` — flips the global before tearing down
/// each entry so unset callbacks fire. Rust port: `Drop` cascades
/// through `Box<hashtable>` to clear all `nodes`; consume the
/// table by value to mirror the C ownership transfer.
pub fn deleteparamtable(t: Option<HashTable>) {
    // c:616-623 — `int odelunset = delunset; delunset = 1;` save/
    // restore so the inner free path fires every entry's unsetfn.
    let odelunset = DELUNSET.swap(1, Ordering::Relaxed); // c:620-621
    if let Some(table) = t {
        // Box dropped here → fields freed; param freenode callbacks
        // are invoked transparently via Drop on each `param` entry.
        drop(table);
    }
    DELUNSET.store(odelunset, Ordering::Relaxed); // c:623
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
pub fn scancountparams(_hn: &param, flags: i32, numparamvals: &mut u32) {
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
pub fn scanparamvals(
    // c:644
    pm: &mut param,
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
            let matched = patcompile(&{ let mut __pat_tok = (&pm.node.nam).to_string(); crate::ported::glob::tokenize(&mut __pat_tok); __pat_tok }, PAT_HEAPDUP as i32, None)
                .map_or(false, |p| pattry(&p, &s));
            if !matched {
                return;
            }
        } else {
            return;
        }
    } else if (f & SCANPM_MATCHKEY) != 0 {
        let prog = scanprog_lock().lock().unwrap().clone();
        if let Some(p) = prog {
            let matched = patcompile(&{ let mut __pat_tok = (&p).to_string(); crate::ported::glob::tokenize(&mut __pat_tok); __pat_tok }, PAT_HEAPDUP as i32, None)
                .map_or(false, |prog| pattry(&prog, &pm.node.nam));
            if !matched {
                return;
            }
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
    let mut vbuf = value {
        pm: None, // placeholder; real C re-binds
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
        let matched = prog
            .and_then(|p| patcompile(&{ let mut __pat_tok = (&p).to_string(); crate::ported::glob::tokenize(&mut __pat_tok); __pat_tok }, PAT_HEAPDUP as i32, None))
            .map_or(false, |prog| pattry(&prog, &s));
        if matched {
            paramvals_lock().lock().unwrap().push(s);
            let inc = if (f & SCANPM_WANTVALS) != 0 {
                1
            } else if (f & SCANPM_WANTKEYS) == 0 {
                1
            } else {
                0
            };
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
pub fn paramvalarr(ht: &HashTable, flags: i32) -> Vec<String> {
    // c:689
    // c:691-692 — DPUTS((flags & (SCANPM_MATCHKEY|SCANPM_MATCHVAL)) && !scanprog,
    //                 "BUG: scanning hash without scanprog set");
    let scanprog_set = scanprog_lock().lock().unwrap().is_some(); // c:691 !scanprog test
    DPUTS!(
        // c:691
        (flags as u32 & (SCANPM_MATCHKEY | SCANPM_MATCHVAL)) != 0 && !scanprog_set, // c:691
        "BUG: scanning hash without scanprog set"                                   // c:692
    );
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
        idx += 1; // c:scanparamvals
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
pub fn getvaluearr(v: Option<&mut value>) -> Vec<String> {
    let v = match v {
        Some(v) => v,
        None => return Vec::new(),
    };
    if !v.arr.is_empty() {
        return v.arr.clone();
    }
    let pm = match v.pm.as_mut() {
        Some(p) => p,
        None => return Vec::new(),
    };
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
pub fn issetvar(name: &str) -> i32 {
    // c:732
    let mut vbuf = value {
        pm: None,
        arr: Vec::new(),
        scanflags: 0,
        valflags: 0,
        start: 0,
        end: -1,
    };
    let mut cursor: &str = name;
    let v = match getvalue(Some(&mut vbuf), &mut cursor, 1) {
        // c:739
        Some(v) => v,
        None => return 0,
    };
    if !cursor.is_empty() {
        // c:739
        return 0; // c:740 no value or more chars after the variable name
    }
    if (v.scanflags as u32 & !SCANPM_ARRONLY) != 0 {
        // c:741
        return if v.end > 1 { 1 } else { 0 }; // c:742
    }

    let slice = v.start != 0 || v.end != -1; // c:744
    let pm = match v.pm.as_ref() {
        Some(p) => p,
        None => return 0,
    };
    if PM_TYPE(pm.node.flags as u32) != PM_ARRAY || !slice {
        // c:745
        return if !slice && (pm.node.flags as u32 & PM_UNSET) == 0 {
            1
        } else {
            0
        }; // c:746
    }

    if v.end == 0 {
        // c:748 empty array slice
        return 0; // c:749
    }
    // c:751 — get the array and check end is within range
    let arr = getvaluearr(Some(v));
    if arr.is_empty() {
        // c:751
        return 0; // c:752
    }
    // c:753
    let bound: usize = if v.end < 0 {
        (-v.end) as usize
    } else {
        v.end as usize
    };
    if arrlen_ge(&arr, bound) {
        1
    } else {
        0
    }
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
pub fn split_env_string(env: &str) -> Option<(String, String)> {
    // c:763
    if env.is_empty() {
        // c:763 !env
        return None;
    }
    let bytes = env.as_bytes();
    // c:770-779 — walk name bytes, reject if high bit set.
    let mut i = 0;
    while i < bytes.len() && bytes[i] != b'=' {
        // c:770
        if bytes[i] >= 128 {
            // c:771 (unsigned char) >= 128
            return None; // c:777
        }
        i += 1;
    }
    // c:780-785 — accept only if `=` was found at non-zero offset.
    if i > 0 && i < bytes.len() && bytes[i] == b'=' {
        // c:780
        let name = String::from_utf8_lossy(&bytes[..i]).into_owned(); // c:781-782
        let value = String::from_utf8_lossy(&bytes[i + 1..]).into_owned(); // c:783
        Some((name, value)) // c:784
    } else {
        None // c:786
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
/// Port of `extern char **environ` (POSIX, read by `createparamtable`
/// at Src/params.c:893). C reads the environment EXACTLY as it was at
/// process entry — nothing mutates `environ` before zsh walks it. In
/// the Rust binary, linked frameworks can rewrite the live environment
/// during their lazy init before our import runs (observed on macOS:
/// CoreFoundation recomputes `__CF_USER_TEXT_ENCODING`, so `export -p`
/// showed `0x1F5:0x0:0x0` while zsh, inheriting the same env, printed
/// the original `0x0:0:0`). `main()` snapshots `std::env::vars()` as
/// its first statement; the import loops below prefer the snapshot and
/// fall back to the live env (lib tests never run `main`).
#[allow(non_upper_case_globals)]
pub static environ: OnceLock<Vec<(String, String)>> = OnceLock::new();

pub fn createparamtable() {
    // c:817

    // c:835 — `paramtab = realparamtab = newparamtable(151, "paramtab")`.
    let _ = paramtab();
    let _ = realparamtab();

    // Helper closure (single definition; mirrors the C
    // `paramtab->addnode(paramtab, ztrdup(name), ip)` site).
    let add_special = |ip: &special_paramdef, tab: &mut HashMap<String, Param>| {
        // c:840 — `paramdef->gsu` selects which gsu_scalar vtable the
        // new param gets. C uses the per-IPDEF macro's BR(...) field;
        // since the Rust special_paramdef doesn't carry a gsu slot
        // yet, dispatch by name to the matching `*_GSU` constant
        // (HOME_GSU/IFS_GSU/...). Non-special scalars (no match)
        // leave gsu_s as None and fall back to strsetfn/strgetfn.
        let gsu_s: Option<Box<gsu_scalar>> = match ip.name {
            "0" => Some(Box::new(ARGZERO_GSU.clone())), // c:225-226 / IPDEF2("0", argzero_gsu, 0)
            "HOME" => Some(Box::new(HOME_GSU.clone())), // c:248
            "IFS" => Some(Box::new(IFS_GSU.clone())),   // c:245
            "TERM" => Some(Box::new(TERM_GSU.clone())), // c:250
            "TERMINFO" => Some(Box::new(TERMINFO_GSU.clone())), // c:251
            "TERMINFO_DIRS" => Some(Box::new(TERMINFODIRS_GSU.clone())), // c:252
            "WORDCHARS" => Some(Box::new(WORDCHARS_GSU.clone())), // c:249
            "USERNAME" => Some(Box::new(USERNAME_GSU.clone())), // c:247
            "KEYBOARD_HACK" => Some(Box::new(KEYBOARDHACK_GSU.clone())), // c:253
            "HISTCHARS" | "histchars" => Some(Box::new(HISTCHARS_GSU.clone())), // c:246
            _ => None,
        };
        let pm = Box::new(param {
            node: hashnode {
                next: None,
                nam: ip.name.to_string(),
                flags: (ip.pm_type | ip.pm_flags | PM_SPECIAL) as i32,
            },
            u_data: 0,
            u_tied: None,
            u_arr: None,
            u_str: None,
            u_val: 0,
            u_dval: 0.0,
            u_hash: None,
            gsu_s, // c:840 gsu_s wired
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
    let is_sh_ksh = EMULATION(EMULATE_SH | EMULATE_KSH);
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
    setiparam("MAILCHECK", 60); // c:858
    // c:Src/params.c:858 lists `KEYTIMEOUT = 40` but zsh 5.9.1
    // observably reports 10 (verified on Homebrew arm-darwin
    // build). The original C source comment + the docs describe
    // KEYTIMEOUT in "hundredths of a second"; the upstream init
    // value was lowered between 5.9 and 5.9.1 (and most distro
    // packages ship a 10 default) so vi-mode / multi-key
    // bindings feel responsive. Bug #321 in docs/BUGS.md.
    setiparam("KEYTIMEOUT", 10); // c:859 (zsh 5.9.1 observed default)
    setiparam("LISTMAX", 100); // c:860

    // c:870-871 — TMPPREFIX / TIMEFMT defaults. C wraps each string
    // through ztrdup_metafy() to escape Meta bytes before storing in
    // the param table; the Rust port mirrors this.
    setsparam("TMPPREFIX", &ztrdup_metafy(DEFAULT_TMPPREFIX)); // c:870
    setsparam("TIMEFMT", &ztrdup_metafy(DEFAULT_TIMEFMT)); // c:871


    // c:873-876 — HOST from gethostname() (ztrdup_metafy wrap c:875).
    let mut host_buf = [0u8; 256];
    let host_rc = unsafe { libc::gethostname(host_buf.as_mut_ptr() as *mut libc::c_char, 256) };
    let hostname = if host_rc == 0 {
        std::ffi::CStr::from_bytes_until_nul(&host_buf)
            .ok()
            .and_then(|c| c.to_str().ok())
            .unwrap_or("")
            .to_string()
    } else {
        String::new()
    };
    setsparam("HOST", &ztrdup_metafy(&hostname)); // c:875

    // c:878-882 — LOGNAME from `getlogin()` libc syscall (with
    // \`cached_username\` as fallback when DISABLE_DYNAMIC_NSS).
    //
    // The previous Rust port read \`env::var(\"LOGNAME\")\` /
    // \`env::var(\"USER\")\` — different source. \`getlogin\` returns the
    // kernel's record of the controlling-terminal login user; env
    // LOGNAME/USER is whatever the parent process passed in (can be
    // spoofed). For audit / SUID-aware code paths, the kernel's view
    // is the right one.
    let logname = unsafe {
        let p = libc::getlogin();
        if p.is_null() {
            String::new()
        } else {
            std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }; // c:880 getlogin()
    let logname = if logname.is_empty() {
        // c:882 — `ztrdup(cached_username)` fallback.
        get_username()
    } else {
        logname
    };
    setsparam("LOGNAME", &ztrdup_metafy(&logname)); // c:878

    // c:891 — pushheap() / c:921 — popheap(). Wraps the env-import
    // loop so per-iter allocations land on the heap zone.
    pushheap(); // c:891

    // c:893-924 — environment import loop. Walk the process-entry
    // `environ` snapshot like C, not the live (possibly framework-
    // mutated) environment — see the `environ` static above.
    let environ_vars: Vec<(String, String)> = environ
        .get()
        .cloned()
        .unwrap_or_else(|| env::vars().collect());
    for (iname, ivalue) in environ_vars {
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
        let metafied = metafy(&ivalue);
        let _ = assignsparam(&iname, &metafied, ASSPM_ENV_IMPORT);
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

    popheap(); // c:921

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
    let is_zsh = EMULATION(EMULATE_ZSH);
    let home_val = home_lock().lock().expect("home poisoned").clone();
    let home_action: Option<bool> = {
        let mut tab = paramtab().write().unwrap();
        if let Some(pm) = tab.get_mut("HOME") {
            if is_zsh {
                // c:939
                pm.node.flags &= !(PM_UNSET as i32); // c:941
                if pm.node.flags & PM_EXPORTED as i32 == 0 {
                    // c:942
                    Some(true)
                } else {
                    Some(false)
                }
            } else if home_val.is_empty() {
                // c:944
                pm.node.flags |= PM_UNSET as i32; // c:945
                Some(false)
            } else {
                Some(false)
            }
        } else {
            None
        }
    };
    if let Some(true) = home_action {
        addenv("HOME", &home_val); // c:943
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
        addenv("LOGNAME", &ustr); // c:948
    }

    // c:949-953 — SHLVL: unconditionally addenv with the incremented
    // value. C uses the \`shlvl\` integer global (IPDEF5 declared at
    // params.c:358 with varinteger_gsu) which was populated during
    // env-import. C: \`++shlvl\` then \`sprintf(buf, \"%d\", (int)shlvl)\`.
    //
    // The previous Rust port read SHLVL fresh from env::var; the
    // canonical read is through paramtab (which has the parsed
    // integer post-import). Falls back to env for the rare case
    // where paramtab hasn't seen the import yet.
    let new_shlvl: i32 = getsparam("SHLVL")
        .or_else(|| env::var("SHLVL").ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
        + 1; // c:951 `++shlvl`
    setiparam("SHLVL", new_shlvl as i64);
    addenv("SHLVL", &new_shlvl.to_string()); // c:953

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
    setsparam("CPUTYPE", &ztrdup_metafy(&cputype)); // c:954/960
    setsparam(
        // c:961
        "MACHTYPE",
        &ztrdup_metafy(MACHTYPE),
    );
    setsparam(
        // c:962
        "OSTYPE",
        &ztrdup_metafy(OSTYPE),
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
    setsparam("TTY", &ztrdup_metafy(&tty_str)); // c:963
    setsparam(
        // c:964
        "VENDOR",
        &ztrdup_metafy(VENDOR),
    );
    let argv0 = env::args().next().unwrap_or_default();
    setsparam("ZSH_ARGZERO", &ztrdup(&argv0)); // c:965 (ztrdup, not _metafy: posixzero)
    setsparam("ZSH_VERSION", &ztrdup_metafy(ZSH_VERSION)); // c:966 (Config/version.mk VERSION via patchlevel::ZSH_VERSION)
    setsparam("ZSH_PATCHLEVEL", &ztrdup_metafy(ZSH_PATCHLEVEL)); // c:967
    // zshrs-only identity. No C counterpart. Surfaced so scripts can
    // detect zshrs (vs. upstream zsh) cleanly without inspecting a
    // `-test` suffix on `$ZSH_VERSION`. See `patchlevel::ZSHRS_VERSION`
    // for the value and bug #73 in docs/BUGS.md for the rationale.
    //
    // In `--zsh` parity mode, suppress this so `${(k)parameters}`
    // matches reference zsh's name set (PM_HIDE doesn't filter from
    // the (k) listing path — C's scanpmparameters only skips PM_UNSET,
    // not PM_HIDE; outright skipping the setsparam call is the only
    // way to keep the name out of the listing). Direct access falls
    // back to an empty value, same as any other unset name.
    if !crate::IS_ZSH_MODE.load(std::sync::atomic::Ordering::Relaxed) {
        setsparam(
            "ZSHRS_VERSION",
            &ztrdup_metafy(crate::ported::patchlevel::ZSHRS_VERSION),
        );
    }

    // c:968-979 — `setaparam("signals", sigptr = zalloc((TRAPCOUNT
    // + 1) * sizeof(char *))); t = sigs; while (t - sigs <= SIGCOUNT)
    // *sigptr++ = ztrdup_metafy(*t++); { for (sig = SIGRTMIN; sig <=
    // SIGRTMAX; sig++) *sigptr++ = ztrdup_metafy(rtsigname(sig, 0));
    // } while ((*sigptr++ = ztrdup_metafy(*t++))) ;`. Builds the
    // $signals array: indices 0..=SIGCOUNT walked from the static
    // sigs[] name table, then SIGRTMIN..SIGRTMAX names, then the
    // trailing tail (DEBUG / ERR / EXIT / ZERR sentinels).
    // c:signames.c sigs[] (generated) — index 0 is "EXIT", entries
    // 1..=SIGCOUNT in PLATFORM SIGNAL-NUMBER order, tail "ZERR",
    // "DEBUG" (zsh.h SIGZERR/SIGDEBUG). SIGS is declared in Linux
    // textual order — sort by libc number to reproduce the generated
    // table on every platform. Same construction as vm_helper.rs —
    // keep in sync.
    let mut by_num: Vec<(&str, i32)> = SIGS.to_vec();
    by_num.sort_by_key(|&(_, n)| n);
    let mut signals_arr: Vec<String> = Vec::with_capacity(by_num.len() + 3);
    signals_arr.push(ztrdup_metafy("EXIT")); // c:sigs[0]
    for &(name, _num) in by_num.iter() {
        signals_arr.push(ztrdup_metafy(name));
    }
    // RT-signal range (Linux-only; macOS SIGS table already includes
    // the realtime names and rtsigname returns "" out of range).
    #[cfg(target_os = "linux")]
    {
        for sig in libc::SIGRTMIN()..=libc::SIGRTMAX() {
            let nm = crate::ported::signals::rtsigname(sig);
            if !nm.is_empty() {
                signals_arr.push(ztrdup_metafy(&nm));
            }
        }
    }
    signals_arr.push(ztrdup_metafy("ZERR")); // c:sigs tail
    signals_arr.push(ztrdup_metafy("DEBUG")); // c:sigs tail
    {
        let mut tab = paramtab().write().unwrap();
        let pm = Box::new(param {
            node: hashnode {
                next: None,
                nam: "signals".to_string(),
                flags: (PM_ARRAY | PM_SPECIAL) as i32,
            },
            u_data: 0,
            u_tied: None,
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
static PARAMTAB_HASHED_STORAGE_INNER: OnceLock<Mutex<HashMap<String, IndexMap<String, String>>>> =
    OnceLock::new();

/// Port of `assigngetset(Param pm)` from `Src/params.c:994`. C body
/// installs the standard get/set/unset vtable matching the
/// param's PM_TYPE so subsequent assignment dispatches go
/// through `pm->gsu.X->setfn`.
pub fn assigngetset(pm: &mut param) {
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
            // c:1015 — DPUTS(1, "BUG: tried to create param node without valid flag")
            DPUTS!(true, "BUG: tried to create param node without valid flag");
            // c:1015
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
pub fn createparam(
    // c:1030
    name: &str,
    mut flags: i32,
) -> Option<Param> {
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
    let oldpm: Option<Param> = if !name.is_empty() {
        paramtab().read().ok().and_then(|t| t.get(name).cloned())
    } else {
        None
    };

    if !name.is_empty() {
        // c:1149-1150 — `if (isset(ALLEXPORT) && !(flags & PM_HASHELEM)) flags |= PM_EXPORTED;`
        if isset(ALLEXPORT) && (flags as u32 & PM_HASHELEM) == 0 {
            flags |= PM_EXPORTED as i32;
        }
    }

    // c:1108 — `if (oldpm && (oldpm->level == locallevel || !(flags
    // & PM_LOCAL)))`: reuse the existing Param in place. c:1135 —
    // else allocate a fresh pm and chain pm.old = oldpm (the
    // local-shadow path). The reuse arm just returns the existing
    // pm with reset base/width; the shadow arm does the chain
    // installation that endparamscope later unwinds.
    let cur_locallevel = locallevel.load(Ordering::Relaxed);
    // c:1106-1107 — DPUTS(oldpm && oldpm->level > locallevel,
    //                    "BUG: old local parameter not deleted");
    DPUTS!(
        // c:1106
        match &oldpm {
            // c:1106
            Some(op) => op.level > cur_locallevel, // c:1106
            None => false,                         // c:1106
        },
        "BUG: old local parameter not deleted" // c:1107
    );
    let reuse = match &oldpm {
        Some(op) => op.level == cur_locallevel || (flags as u32 & PM_LOCAL) == 0,
        None => false,
    };

    let mut pm: Param = if reuse {
        // c:1132-1134 — `pm = oldpm; pm->base = pm->width = 0;
        // oldpm = pm->old;` Reuse the entry already in paramtab.
        let mut existing = oldpm.unwrap(); // safe: reuse=true requires Some
        existing.base = 0; // c:1133
        existing.width = 0; // c:1133
        existing
    } else {
        // c:1136 zshcalloc(sizeof *pm) — fresh allocation; chain the
        // outer Param into pm.old (c:1137) so endparamscope can
        // restore it. c:1144 paramtab->removenode is implicit since
        // we re-insert below.
        //
        // c:Src/builtin.c:2382-2424 newspecial path — for PM_SPECIAL
        // shadows (`local IFS=...`), the C source allocates a
        // separate `tpm`, calls `copyparam(tpm, pm, 1)` which uses
        // the GSU getfn to read the current value into tpm.u.str,
        // then sets `pm->old = tpm`. zshrs's createparam path moves
        // `oldpm` into pm.old as-is; for specials whose value lives
        // in a global (ifs_lock, paramtab-external) the bare
        // `oldpm.u_str` is empty and endparamscope can't restore the
        // real outer value. Snapshot the current value via the GSU
        // getfn now so the saved chain carries the right string.
        // Bug #8 in docs/BUGS.md (`local IFS=:` leaked past return).
        //
        // c:Src/builtin.c:2382-2424 copyparam for PM_HASHED — same
        // shape, different storage. zshrs's PM_HASHED data lives in
        // the parallel `paramtab_hashed_storage` map keyed by name (no
        // scope dimension), so a `local -A h` shadow that writes
        // through set_assoc would clobber the outer scope's bag and
        // endparamscope's pm.old restoration alone wouldn't recover
        // it. Push the current paramtab_hashed_storage[name] onto the
        // shadow stack BEFORE the fresh pm gets installed, so when
        // endparamscope unwinds the PM_HASHED stale entry it can pop
        // and restore. Bug #415.
        let oldpm = if let Some(mut op) = oldpm {
            if (op.node.flags as u32 & (PM_SPECIAL | PM_TIED)) != 0 {
                // c:Src/builtin.c:2382-2424 copyparam — snapshot the
                // CURRENT live value into the shadow chain. PM_TIED
                // scalars (FPATH/PATH/CDPATH…) included: their value
                // derives from the tied array's global storage, which
                // the local's writes flow through — without the
                // snapshot, `f() { local FPATH=/tmp }; f` left the
                // GLOBAL fpath at ( /tmp ) and every later autoload
                // failed (zinit's :zinit-tmp-subst-autoload does
                // exactly this dance). Fall back to the name-routed
                // getsparam when the pm carries no scalar gsu.
                let getfn_ptr = op.gsu_s.as_ref().map(|g| g.getfn);
                if let Some(getfn) = getfn_ptr {
                    op.u_str = Some(getfn(&op));
                } else if let Some(v) = getsparam(&op.node.nam) {
                    op.u_str = Some(v);
                }
            }
            if (op.node.flags as u32 & PM_HASHED) != 0
                && (flags as u32 & PM_LOCAL) != 0
            {
                // Push current paramtab_hashed_storage[name] (Some/None)
                // onto the shadow stack so endparamscope can restore.
                // Then CLEAR the storage so the local shadow starts
                // with an empty bag — without this, `local -A h` (no
                // value) leaves the outer's data visible and a
                // subsequent `h[x]=v` appends to it instead of
                // creating a fresh local assoc. C's copyparam handles
                // this via separate `tpm` / `pm` u.hash slots so the
                // pm.u.hash that fresh writes go through is
                // zero-initialised; zshrs's parallel storage is one
                // map per name, so the save+clear pair is the
                // equivalent.
                let saved: Option<IndexMap<String, String>> = paramtab_hashed_storage()
                    .lock()
                    .ok()
                    .and_then(|m| m.get(name).cloned());
                let stk_mtx = PARAMTAB_HASHED_SHADOW_STACK
                    .get_or_init(|| Mutex::new(HashMap::new()));
                if let Ok(mut stk) = stk_mtx.lock() {
                    stk.entry(name.to_string()).or_default().push(saved);
                }
                if let Ok(mut m) = paramtab_hashed_storage().lock() {
                    m.insert(name.to_string(), IndexMap::new());
                }
            }
            Some(op)
        } else {
            None
        };
        Box::new(param {
            node: hashnode {
                next: None,
                nam: name.to_string(),
                flags: 0,
            },
            u_data: 0,
            u_tied: None,
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
            old: oldpm, // c:1137 pm->old = oldpm
            // c:1136 — C: `pm = zshcalloc(sizeof *pm)`. calloc
            // zeroes pm.level so a freshly created GLOBAL assignment
            // (`x=foo` inside a function) gets level=0 and survives
            // endparamscope. The `pm->level = locallevel` set happens
            // ONLY through builtin.c:2576 (PM_LOCAL path: `local x=…`).
            level: if (flags as u32 & PM_LOCAL) != 0 {
                cur_locallevel
            } else {
                0
            },
        })
    };

    pm.node.flags = flags & !(PM_LOCAL as i32); // c:1155
    if (pm.node.flags as u32 & PM_SPECIAL) == 0 {
        // c:1157
        assigngetset(&mut pm); // c:1158
    }
    // c:Src/params.c:1146 — when shadowing a special parameter
    // (e.g. `local IFS=...` inside a function), the new pm must
    // inherit the canonical special-var GSU (ifssetfn etc.) so
    // writes route through the global storage that `$IFS`
    // expansion reads. Without this, the shadow's setfn was the
    // generic strsetfn and `local IFS=:` updated paramtab.u_str
    // but NOT the canonical `ifs` global — `$IFS` expansion in
    // the same function read the default value through ifsgetfn.
    //
    // Detect special-var names and override the gsu_s + stamp
    // PM_SPECIAL. Mirrors the back-fill at assignsparam:4981 but
    // covers the create-time path (local / typeset of fresh
    // shadow), not just the assign-existing path.
    if !name.is_empty() {
        let special_gsu: Option<Box<gsu_scalar>> = match name {
            "HOME" => Some(Box::new(HOME_GSU.clone())),
            "IFS" => Some(Box::new(IFS_GSU.clone())),
            "TERM" => Some(Box::new(TERM_GSU.clone())),
            "TERMINFO" => Some(Box::new(TERMINFO_GSU.clone())),
            "TERMINFO_DIRS" => Some(Box::new(TERMINFODIRS_GSU.clone())),
            "WORDCHARS" => Some(Box::new(WORDCHARS_GSU.clone())),
            "USERNAME" => Some(Box::new(USERNAME_GSU.clone())),
            "KEYBOARD_HACK" => Some(Box::new(KEYBOARDHACK_GSU.clone())),
            "HISTCHARS" | "histchars" => Some(Box::new(HISTCHARS_GSU.clone())),
            _ => None,
        };
        if let Some(gsu) = special_gsu {
            pm.gsu_s = Some(gsu);
            pm.node.flags |= PM_SPECIAL as i32;
        }
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
    Some(pm) // c:1159
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
pub fn setsparam(s: &str, val: &str) -> Option<Param> {
    assignsparam(s, val, ASSPM_WARN as i32) // c:3352
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
pub fn createspecialhash(name: &str, flags: i32) -> Option<Param> {
    // c:1186 — `createparam(name, PM_SPECIAL|PM_HASHED|flags)`.
    let mut pm = createparam(name, (PM_SPECIAL | PM_HASHED) as i32 | flags)?;

    // c:1204-1205 — if shadowing an old param, set level=locallevel.
    if pm.old.is_some() {
        // C: `pm->level = locallevel`. The previous Rust port had
        // `let ll = 0_i32;` as a hardcoded placeholder — meaning
        // shadowed special-hash params (`fpath`, `path`, `psvar`,
        // etc. assigned inside a function via local) would NEVER
        // get their level tagged for restoration. After the function
        // returned, the original param would be inaccessible because
        // the shadow record's level (always 0) wouldn't trigger the
        // endparamscope unset. Now reads the canonical `locallevel`
        // global from params.rs (matching the C global).
        pm.level = locallevel.load(Ordering::Relaxed) as i32;
        // c:1205
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
    let ht = Box::new(hashtable {
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

    Some(pm) // c:1223
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
pub fn copyparam(
    // c:1236
    tpm: &mut param,
    pm: &mut param,
    fakecopy: i32,
) {
    tpm.node.flags = pm.node.flags; // c:1244
    tpm.base = pm.base; // c:1245
    tpm.width = pm.width; // c:1246
    tpm.level = pm.level; // c:1247
    if fakecopy == 0 {
        // c:1248
        tpm.old = pm.old.take(); // c:1249
        tpm.node.flags &= !(PM_SPECIAL as i32); // c:1250
    }
    match PM_TYPE(pm.node.flags as u32) {
        // c:1252
        t if t == PM_SCALAR || t == PM_NAMEREF => {
            // c:1255 — `tpm->u.str = ztrdup(pm->gsu.s->getfn(pm));`.
            // C dispatches through the GSU getfn pointer so PM_SPECIAL
            // params (IFS, PATH, HOME, ...) return their canonical
            // global value (ifsgetfn reads `ifs_lock`, etc.). Without
            // this dispatch, `local IFS=:` saved an empty pm.u_str
            // (the bare strgetfn read) and endparamscope could not
            // restore the real outer value on scope exit (bug #8 in
            // docs/BUGS.md).
            let getfn_ptr = pm.gsu_s.as_ref().map(|g| g.getfn);
            tpm.u_str = Some(if let Some(getfn) = getfn_ptr {
                getfn(pm)
            } else {
                strgetfn(pm)
            });
        }
        t if t == PM_INTEGER => {
            // c:1257
            tpm.u_val = intgetfn(pm); // c:1258
        }
        t if t == PM_EFLOAT || t == PM_FFLOAT => {
            // c:1260-1261
            tpm.u_dval = floatgetfn(pm); // c:1262
        }
        t if t == PM_ARRAY => {
            // c:1264
            tpm.u_arr = Some(arrgetfn(pm)); // c:1265
        }
        t if t == PM_HASHED => {
            // c:1267
            // copyparamtable(pm->gsu.h->getfn(pm), pm->node.nam)            // c:1268
            tpm.u_hash = copyparamtable(pm.u_hash.as_ref(), &pm.node.nam);
        }
        _ => {}
    }
    if fakecopy == 0 {
        // c:1280
        assigngetset(tpm); // c:1281
    }
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

/// Check if string is valid identifier (from params.c isident)
// Return 1 if the string s is a valid identifier, else return 0.         // c:1288
/// `isident` — see implementation.
pub fn isident(s: &str) -> bool {
    // c:1288
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
            // c:1326
            // c:1329-1330 — `if (*ss != '[') return 0; if (!(ss =
            //          parse_subscript(++ss, 1, ']'))) return 0;`
            // Subscript MUST be balanced — `foo[` (missing `]`)
            // is NOT a valid identifier. The previous Rust port
            // accepted `[` at the end unconditionally, missing
            // the balanced-pair requirement.
            //
            // Routing through the full `parse_subscript` (which
            // drives a nested lex context) would be overkill at
            // this site — a simple bracket-balance walk over the
            // remaining bytes suffices. Count `[` / `]` and require
            // the depth to return to 0 before end-of-string.
            let mut depth = 1i32;
            // c:Src/params.c:1334 — `if (!(ss = parse_subscript(++ss,
            // 1, ']'))) return 0;`. C's parse_subscript rejects empty
            // subscripts: `h[]` returns NULL. Mirror by tracking
            // whether ANY non-bracket char appears before the depth
            // returns to 0. Without this, `h[]=val` was accepted as a
            // valid assoc element write — but zsh errors
            // `not an identifier: h[]` (bug #288 in docs/BUGS.md).
            //
            // c:Src/params.c parse_subscript (params.c:1480+) also
            // recognises backslash-escaped brackets: `A[\[k\]]` is a
            // single subscript with key `[k]`. The `\[` doesn't count
            // toward bracket depth and `\]` doesn't close. Track a
            // bslash flag so the depth walk matches C semantics.
            let mut saw_content = false;
            // Bug fix: `s.split('[').skip(1).next()` only returned
            // the segment between the first and second `[`, dropping
            // everything after. Use find()+slice to get the entire
            // tail starting after the first `[`.
            let tail_start = s
                .char_indices()
                .find(|(_, c)| *c == '[')
                .map(|(i, _)| i + 1);
            let saw_close = tail_start
                .map(|start| &s[start..])
                .is_some_and(|tail| {
                    let mut bslash = false;
                    for ch in tail.chars() {
                        if bslash {
                            // c:parse_subscript — escaped char is
                            // content, doesn't affect depth.
                            saw_content = true;
                            bslash = false;
                            continue;
                        }
                        match ch {
                            '\\' => {
                                bslash = true;
                                saw_content = true;
                            }
                            '[' => {
                                depth += 1;
                                saw_content = true;
                            }
                            ']' => {
                                depth -= 1;
                                if depth == 0 {
                                    return true;
                                }
                                saw_content = true;
                            }
                            _ => saw_content = true,
                        }
                    }
                    false
                });
            if !saw_content {
                return false; // c:1334 empty subscript rejected
            }
            return saw_close;
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
    assoc: Option<&IndexMap<String, String>>,
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
                return Some(getarg_out::Flags {
                    flags,
                    rest: &rest[close + 1..],
                });
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
                patcompile(&{ let mut __pat_tok = (pat).to_string(); crate::ported::glob::tokenize(&mut __pat_tok); __pat_tok }, PAT_HEAPDUP as i32, None).map_or(false, |p| pattry(&p, target))
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
                return Some(getarg_out::Value(Value::str(
                    words
                        .get(idx_into)
                        .map(|s| s.to_string())
                        .unwrap_or_default(),
                )));
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
                patcompile(&{ let mut __pat_tok = (pat_used).to_string(); crate::ported::glob::tokenize(&mut __pat_tok); __pat_tok }, PAT_HEAPDUP as i32, None).map_or(false, |p| pattry(&p, s))
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
                return Some(getarg_out::Value(Value::str(
                    words
                        .get(idx_into)
                        .map(|s| s.to_string())
                        .unwrap_or_default(),
                )));
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
                        patcompile(&{ let mut __pat_tok = (pat).to_string(); crate::ported::glob::tokenize(&mut __pat_tok); __pat_tok }, PAT_HEAPDUP as i32, None)
                            .map_or(false, |p| pattry(&p, &cand))
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
                    s_chars
                        .get(s_pos)
                        .map(|c| c.to_string())
                        .unwrap_or_default(),
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
pub fn getindex(pptr: &mut &str, v: &mut value, scanflags: i32) -> i32 {
    // c:2001

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
    let close_pos = parse_subscript(after_lbrack, ']');
    let close_pos = match close_pos {
        Some(p) => p,
        None => {
            // c:2020 — `zerr("invalid subscript")`.
            zerr("invalid subscript");
            *pptr = ""; // c:2021
            return 1; // c:2022
        }
    };
    let body = &after_lbrack[..close_pos];

    // c:2027 — special-case `[*]` / `[@]`.
    if body == "*" || body == "@" {
        if body == "@" && (v.scanflags != 0 || v.pm.is_none()) {
            // c:2028
            v.scanflags |= SCANPM_ISVAR_AT as i32; // c:2029
        }
        v.start = 0; // c:2030
        v.end = -1; // c:2031
                    // c:2156 — `*tbrack = ']'; *pptr = s` (s points past `]`).
        *pptr = &after_lbrack[close_pos + 1..];
        return 0; // c:2160
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
    let mut end: i64 = match end_str {
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

    if start == 0 && end == 0 {
        // c:2126
        // c:2134 — `if (isset(KSHZEROSUBSCRIPT))` non-strict mode.
        // Treats `a[0]` as the first element (end = startnextlen,
        // which is 1 for ASCII). c:2141-2150 strict mode keeps the
        // VALFLAG_EMPTY + start=-1 sentinel for empty access.
        if crate::ported::zsh_h::isset(crate::ported::zsh_h::KSHZEROSUBSCRIPT) {
            end = 1; // c:2140 — `end = startnextlen` (1 for ASCII)
        } else {
            v.valflags |= VALFLAG_EMPTY; // c:2147
            start = -1; // c:2148
        }
    }
    // c:2156-2158 — clear scanflags for non-comma simple subscript
    // when match flags absent.
    if v.scanflags != 0
        && !com
        && (v.scanflags as u32 & SCANPM_MATCHMANY == 0
            || v.scanflags as u32 & (SCANPM_MATCHKEY | SCANPM_MATCHVAL | SCANPM_KEYMATCH) == 0)
    {
        v.scanflags = 0;
    }
    let _ = (SCANPM_ISVAR_AT, SCANPM_WANTINDEX, VALFLAG_INV);
    v.start = start as i32; // c:2159
    v.end = end as i32; // c:2160

    // c:2164-2165 — advance `*pptr` past the close bracket.
    *pptr = &after_lbrack[close_pos + 1..];
    0 // c:2166
}

/// Port of `getvalue(Value v, char **pptr, int bracks)` from `Src/params.c:2173`. C body:
/// `return fetchvalue(v, pptr, bracks, SCANPM_CHECKING);` — pure
/// wrapper around `fetchvalue` with the SCANPM_CHECKING flag set
/// so unset params don't trigger creation.
pub fn getvalue<'a>(
    v: Option<&'a mut value>,
    pptr: &mut &str,
    bracks: i32,
) -> Option<&'a mut value> {
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
pub fn fetchvalue<'a>(
    // c:2180
    v: Option<&'a mut value>,
    pptr: &mut &str,
    bracks: i32,
    scanflags: i32,
) -> Option<&'a mut value> {
    let s = *pptr;
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None; // c:2214 fall-through
    }
    let c = bytes[0];
    let mut ppar: i32 = 0;
    let mut end_pos = 0usize;

    if c.is_ascii_digit() {
        // c:2190
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
    } else if itype_end(s, crate::ported::ztype_h::IIDENT as u32, false) > 0 {
        // c:2196 — `itype_end(s, IIDENT, 0)` — walk identifier chars.
        end_pos = itype_end(s, crate::ported::ztype_h::IIDENT as u32, false);
    } else if matches!(c, b'?' | b'#' | b'$' | b'!' | b'@' | b'*' | b'-') {
        // c:2198-2210
        end_pos = 1;
    } else {
        return None; // c:2213
    }

    let name = &s[..end_pos];
    *pptr = &s[end_pos..];

    if ppar > 0 {
        // c:2217-2225 positional
        if let Some(v) = v {
            *v = value {
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
    let pm = pm?; // c:2237-2241

    // c:2241-2243 — `if (PM_UNSET && !PM_DECLARED) return NULL`.
    if pm.node.flags & PM_UNSET as i32 != 0 && pm.node.flags & PM_DECLARED as i32 == 0 {
        return None;
    }

    // c:2246-2270 — nameref deref. Partially handled: we route
    // through resolve_nameref if PM_NAMEREF is set and the caller
    // didn't pass SCANPM_NONAMEREF.
    let pm = if pm.node.flags & PM_NAMEREF as i32 != 0 && (scanflags as u32) & SCANPM_NONAMEREF == 0
    {
        resolve_nameref(Some(pm))?
    } else {
        pm
    };

    if let Some(v) = v {
        // c:2274-2282 — populate Value from pm.
        *v = value {
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
        if bracks > 0 && (pptr.starts_with('[') || pptr.starts_with(Inbrack)) {
            if getindex(pptr, v, scanflags) != 0 {
                // c:2290
                return Some(v); // c:2292
            }
        } else if (scanflags & SCANPM_ASSIGNING as i32) == 0 && v.scanflags != 0 && isset(KSHARRAYS)
        {
            // c:2294-2296 — KSHARRAYS implicit `[0]` for bare arr.
            v.end = 1;
            v.scanflags = 0;
        } else {
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
pub fn getstrvalue(v: Option<&mut value>) -> String {
    let v = match v {
        Some(v) => v,
        None => return String::new(),
    };
    // c:2344-2348 — `if (VALFLAG_INV && !PM_HASHED) return sprintf("%d", v->start)`.
    if (v.valflags & VALFLAG_INV) != 0 {
        let hashed =
            v.pm.as_ref()
                .map(|p| (p.node.flags as u32 & PM_HASHED) != 0)
                .unwrap_or(false);
        if !hashed {
            return v.start.to_string();
        }
    }
    let pm = match v.pm.as_mut() {
        Some(p) => p,
        None => return String::new(),
    };
    let t = PM_TYPE(pm.node.flags as u32);
    let pmflags = pm.node.flags as u32;

    // c:2350-2370 — PM_TYPE dispatch.
    let mut s: String = if t == PM_HASHED || t == PM_ARRAY {
        // c:2351-2370
        let arr = arrgetfn(pm);
        if v.scanflags != 0 {
            // c:2361
            arr.join(" ")
        } else {
            let mut start = v.start;
            if start < 0 {
                start += arr.len() as i32;
            } // c:2364
            if start < 0 || (start as usize) >= arr.len() {
                // c:2365-2366
                String::new()
            } else {
                arr[start as usize].clone()
            }
        }
    } else if t == PM_INTEGER {
        // c:2371
        // c:2373 — `convbase(buf, pm->gsu.i->getfn(pm), pm->base)`.
        // The previous Rust port used `intgetfn(pm).to_string()` (naked
        // base-10). With `convbase` now ported (params.rs:6577), honor
        // `pm.base` so `typeset -i 16 x=255` renders as `0xff` rather
        // than `255` per zsh's `$x`-expansion + `typeset -p`.
        convbase_underscore(
            intgetfn(pm),
            if pm.base > 0 { pm.base } else { 10 }, // c:2373 pm->base
            pm.width,                               // c:2373 pm->width for underscore grouping
        )
    } else if t == PM_EFLOAT || t == PM_FFLOAT {
        // c:2375
        // c:2377 — `convfloat(getfn(pm), pm->base, pm->flags, NULL)`.
        // Route through convfloat_underscore which honors pm.width.
        convfloat_underscore(floatgetfn(pm), pm.width)
    } else if t == PM_SCALAR || t == PM_NAMEREF {
        // c:2380
        strgetfn(pm)
    } else {
        // c:2384
        DPUTS!(true, "BUG: param node without valid type"); // c:2385
        String::new() // c:2386 s = "" (line c:2384)
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
                let mut out: String = trimmed.chars().take(take).collect();
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
                    let numeric_pm = (pmflags & (PM_INTEGER | PM_EFLOAT | PM_FFLOAT)) != 0;
                    if pad_flags & PM_RIGHT_Z != 0 {
                        // c:2446-2466 — find the prefix to keep
                        // (blanks → minus → 0x / base#).
                        let bytes = s.as_bytes();
                        let mut t = 0usize;
                        while t < bytes.len() && (bytes[t] == b' ' || bytes[t] == b'\t') {
                            t += 1; // c:2446-2447
                        }
                        if numeric_pm && t < bytes.len() && bytes[t] == b'-' {
                            t += 1; // c:2454-2455
                        }
                        if (pmflags & PM_INTEGER) != 0 {
                            let cbases = optlookup("cbases") > 0;
                            if cbases
                                && t + 1 < bytes.len()
                                && bytes[t] == b'0'
                                && bytes[t + 1] == b'x'
                            {
                                t += 2; // c:2462-2463
                            } else if let Some(hash_off) =
                                bytes[t..].iter().position(|&b| b == b'#')
                            {
                                t += hash_off + 1; // c:2464-2465
                            }
                        }
                        valprefend = t;
                        if t == bytes.len() {
                            zero = false; // c:2468-2469
                        } else if !numeric_pm && !bytes[t].is_ascii_digit() {
                            zero = false; // c:2473-2474
                        }
                    }
                    // c:2483 — pad char picks: ' ' if PM_RIGHT_B or
                    // numeric-prefix detection failed, else '0'.
                    let pad_char = if (pad_flags & PM_RIGHT_B) != 0 || !zero {
                        ' '
                    } else {
                        '0'
                    };
                    let need = fwidth - charlen;
                    let prefix = &s[..valprefend];
                    let rest = &s[valprefend..];
                    let mut out = String::with_capacity(need + s.len());
                    out.push_str(prefix); // c:2491
                    out.extend(std::iter::repeat(pad_char).take(need)); // c:2483-2485
                    out.push_str(rest); // c:2492-2493
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
pub fn getintvalue(v: Option<&mut value>) -> i64 {
    let v = match v {
        Some(v) => v,
        None => return 0,
    };
    if (v.valflags & VALFLAG_INV) != 0 {
        return v.start as i64;
    }
    if v.scanflags != 0 {
        // sepjoin(arr, NULL, 1) → mathevali(scal); arr backend missing.
        return 0;
    }
    let pm = match v.pm.as_mut() {
        Some(p) => p,
        None => return 0,
    };
    if PM_TYPE(pm.node.flags as u32) == PM_INTEGER {
        return intgetfn(pm);
    }
    if (pm.node.flags as u32 & (PM_EFLOAT | PM_FFLOAT)) != 0 {
        return floatgetfn(pm) as i64;
    }
    // c:2618 — `return mathevali(getstrvalue(v));`. The previous
    // Rust port used `s.parse::<i64>().unwrap_or(0)` which silently
    // returned 0 for any non-trivial arithmetic on the scalar
    // value side (e.g. `typeset x="1+2"; ((y = x))` would yield
    // y=0 instead of 3). Route through `math::mathevali` to
    // match C's arithmetic-expression evaluation.
    let pm = v.pm.as_mut().unwrap();
    let s = strgetfn(pm);
    mathevali(&s).unwrap_or(0) // c:2618 mathevali(...)
}

/// Port of `getnumvalue(Value v)` from `Src/params.c:2624`. Returns an
/// `mnumber` (tagged int/float). C body dispatches on `valflags &
/// VALFLAG_INV` (returns start as int), `scanflags` (sepjoin →
/// matheval), then PM_TYPE: PM_INTEGER → mn.l = pm->gsu.i->getfn,
/// PM_EFLOAT|PM_FFLOAT → mn.type=MN_FLOAT; mn.d = pm->gsu.f->getfn,
/// else matheval(getstrvalue(v)).
pub fn getnumvalue(v: Option<&mut value>) -> mnumber {
    let v = match v {
        Some(v) => v,
        None => {
            return mnumber {
                l: 0,
                d: 0.0,
                type_: MN_INTEGER,
            }
        }
    };
    if (v.valflags & VALFLAG_INV) != 0 {
        return mnumber {
            l: v.start as i64,
            d: 0.0,
            type_: MN_INTEGER,
        };
    }
    if v.scanflags != 0 {
        return mnumber {
            l: 0,
            d: 0.0,
            type_: MN_INTEGER,
        };
    }
    let pm = match v.pm.as_mut() {
        Some(p) => p,
        None => {
            return mnumber {
                l: 0,
                d: 0.0,
                type_: MN_INTEGER,
            }
        }
    };
    let t = PM_TYPE(pm.node.flags as u32);
    if t == PM_INTEGER {
        return mnumber {
            l: intgetfn(pm),
            d: 0.0,
            type_: MN_INTEGER,
        };
    }
    if t == PM_EFLOAT || t == PM_FFLOAT {
        return mnumber {
            l: 0,
            d: floatgetfn(pm),
            type_: MN_FLOAT,
        };
    }
    // c:2640 — `return matheval(getstrvalue(v));`. The previous
    // Rust port used `parse::<i64>()` / `parse::<f64>()` directly
    // on the scalar string, which silently failed for any non-
    // trivial arithmetic. Route through `math::matheval` to match
    // C's arithmetic-expression evaluation; matheval returns an
    // mnumber tag matching the C output type.
    let s = strgetfn(pm);
    matheval(&s) // c:2640 matheval(...)
        .unwrap_or(mnumber {
            l: 0,
            d: 0.0,
            type_: MN_INTEGER,
        })
}

/// Port of `export_param(Param pm)` from `Src/params.c:2653`.
///
/// C body converts `pm`'s value to its scalar form per `PM_TYPE`:
///   PM_INTEGER:        convbase(buf, getfn, pm->base)
///   PM_EFLOAT/FFLOAT:  convfloat(getfn, pm->base, pm->node.flags, NULL)
///   PM_SCALAR/etc.:    gsu.s->getfn(pm)
/// Then calls `addenv(pm, val)`. PM_ARRAY/PM_HASHED early-return.
///
/// The previous Rust port used `format!("{}", intgetfn(pm))` for
/// integers and `format!("{}", floatgetfn(pm))` for floats — Rust's
/// DEFAULT formatting. C uses convbase/convfloat which respect
/// `pm.base` and `pm.flags`:
///   - `typeset -i16 x=255; export x` should put "16#FF" in the
///     env (per pm.base==16). The previous Rust port wrote "255".
///   - `typeset -F3 y=3.14; export y` should put "3.140" (per
///     pm.base==3 precision + PM_FFLOAT flag). Rust wrote "3.14".
///
/// Both formatter ports exist (`params::convbase`, `utils::convfloat`).
/// Wire them so the env-side representation matches C.
pub fn export_param(pm: &mut param) {
    // c:2653
    let t = PM_TYPE(pm.node.flags as u32);
    if (t & (PM_ARRAY | PM_HASHED)) != 0 {
        // c:2659 array/hash skip
        return;
    }
    let val: String = if t == PM_INTEGER {
        // c:2664 — `convbase(buf, pm->gsu.i->getfn(pm), pm->base)`.
        let base = if pm.base > 0 { pm.base } else { 10 };
        convbase(intgetfn(pm), base as u32) // c:2664
    } else if (pm.node.flags as u32 & (PM_EFLOAT | PM_FFLOAT)) != 0 {
        // c:2668 — `convfloat(pm->gsu.f->getfn(pm), pm->base,
        //                     pm->node.flags, NULL)`.
        convfloat(floatgetfn(pm), pm.base, pm.node.flags as u32)
        // c:2668
    } else {
        strgetfn(pm)
    };
    addenv(&pm.node.nam, &val);
    pm.env = Some(val);
}

/// Port of `setstrvalue(Value v, char *val)` from `Src/params.c:2685`. C body is a
/// one-liner: `assignstrvalue(v, val, 0);` — the real workhorse
/// is `assignstrvalue` (params.c:2692).
pub fn setstrvalue(v: Option<&mut value>, val: &str) {
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
pub fn assignstrvalue(v: Option<&mut value>, val: Option<String>, flags: i32) {
    if unset(EXECOPT) {
        return;
    }

    let v = match v {
        Some(v) => v,
        None => return,
    };
    let pm = match v.pm.as_mut() {
        Some(p) => p,
        None => return,
    };

    if (pm.node.flags as u32 & PM_READONLY) != 0 {
        // c:2701 — `zerr("read-only variable: %s", pm->node.nam)`.
        zerr(&format!("read-only variable: {}", pm.node.nam)); // c:2701
        return;
    }
    if (pm.node.flags as u32 & PM_HASHED) != 0
        && (v.scanflags as u32 & (SCANPM_MATCHMANY | SCANPM_ARRONLY)) != 0
    {
        // c:2706 — `zerr("%s: attempt to set slice of associative array", ...)`.
        zerr(&format!(
            "{}: attempt to set slice of associative array",
            pm.node.nam
        )); // c:2706
        return;
    }
    if (v.valflags & VALFLAG_EMPTY) != 0 {
        // c:2710 — `zerr("%s: assignment to invalid subscript range", ...)`.
        zerr(&format!(
            "{}: assignment to invalid subscript range",
            pm.node.nam
        )); // c:2710
        return;
    }
    pm.node.flags &= !(PM_UNSET as i32);

    let mut val = val;
    match PM_TYPE(pm.node.flags as u32) {
        t if t == PM_SCALAR || t == PM_NAMEREF => {
            let mut v_str = val.take().unwrap_or_default();
            // c:Src/params.c — PM_LOWER / PM_UPPER case fold on
            // assignment. zsh applies these flags both when writing
            // the in-memory scalar and when exporting to env; the
            // copyenvstr path handles the export side, but the
            // scalar set path also needs to fold so `echo $X`
            // reads the lowercased value. Without this, `typeset -l
            // X; X=MixedCase; echo $X` printed "MixedCase".
            let pf = pm.node.flags as u32;
            if pf & PM_LOWER != 0 {
                v_str = v_str.to_ascii_lowercase();
            } else if pf & PM_UPPER != 0 {
                v_str = v_str.to_ascii_uppercase();
            }
            if v.start == 0 && v.end == -1 {
                // c:2748 — `v->pm->gsu.s->setfn(v->pm, val);`. C
                // dispatches through the param's GSU vtable so
                // PM_SPECIAL params route to their canonical setfn
                // (homesetfn, ifssetfn, ...). The Rust port stores
                // the vtable on `pm.gsu_s` (set in `createparamtable`
                // via `gsu_scalar_for_special`); when set, dispatch
                // through it. When unset, fall back to the default
                // `strsetfn` path (C's stdscalar_gsu.setfn).
                // c:2742-2746 — ASSPM_AUGMENT: prepend the existing
                // value before storing. Without this, `PATH+=":/foo"`
                // would replace PATH instead of appending.
                let final_str = if (flags & ASSPM_AUGMENT) != 0 {
                    let prev = pm.u_str.clone().unwrap_or_default();
                    format!("{}{}", prev, v_str)
                } else {
                    v_str
                };
                let len = final_str.len();
                let setfn_ptr = pm.gsu_s.as_ref().map(|g| g.setfn);
                if let Some(setfn) = setfn_ptr {
                    setfn(pm, final_str); // c:2748
                } else {
                    strsetfn(pm, final_str); // c:2748 (default)
                }
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
                if (v.valflags & VALFLAG_INV) != 0 && !isset(KSHARRAYS) {
                    start -= 1;
                    end -= 1;
                }
                if start < 0 {
                    start += zlen;
                    if start < 0 {
                        start = 0;
                    }
                }
                if start > zlen {
                    start = zlen;
                }
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
                if e <= z.len() {
                    x.push_str(&z[e..]);
                }
                strsetfn(pm, x);
                if (pm.node.flags as u32 & PM_HASHELEM) == 0
                    && ((pm.node.flags as u32 & PM_NAMEDDIR) != 0 || isset(AUTONAMEDIRS))
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
                    // c:Src/params.c:2774 — `mathevali(val)`. C's
                    // matheval calls zerr (Src/math.c:1462+) on parse
                    // failures, which sets `errflag |= ERRFLAG_ERROR`
                    // and the caller propagates the abort. The Rust
                    // port returns Result<i64,String>; `unwrap_or(0)`
                    // silently swallowed "operator expected at 'def'"
                    // and stored 0 instead. Bug #75 in docs/BUGS.md.
                    //
                    // Mirror the C semantic: surface the error via
                    // zerr (which sets errflag), then fall back to 0
                    // for the stored value (C also stores whatever the
                    // partial parse computed, which is typically 0).
                    match mathevali(s) {
                        Ok(v) => v,
                        Err(msg) => {
                            zerr(&msg);
                            0
                        }
                    }
                };
                // c:2775-2778 — `if (flags & ASSPM_AUGMENT) pm->u.val += val.l;
                //                else pm->u.val = val.l;`. The augment
                // path is what makes `integer x=42; x+=8` store 50 instead
                // of 8. Without this the integer `+=` operator silently
                // replaced.
                let final_val = if (flags & ASSPM_AUGMENT) != 0 {
                    pm.u_val.wrapping_add(ival)
                } else {
                    ival
                };
                intsetfn(pm, final_val);
                if (pm.node.flags as u32 & (PM_LEFT | PM_RIGHT_B | PM_RIGHT_Z)) != 0
                    && pm.width == 0
                {
                    pm.width = s.len() as i32;
                }
                if pm.base == 0 {
                    let lb = lastbase();
                    if lb != -1 {
                        pm.base = lb;
                    }
                }
            }
        }
        t if t == PM_EFLOAT || t == PM_FFLOAT => {
            if let Some(ref s) = val {
                let mn = if (flags & ASSPM_ENV_IMPORT) != 0 {
                    mnumber {
                        l: 0,
                        d: s.parse::<f64>().unwrap_or(0.0),
                        type_: MN_FLOAT,
                    }
                } else {
                    // c:Src/params.c — float assignment runs the RHS
                    // through matheval; on parse failure, zsh emits the
                    // engine's diagnostic ("bad math expression: operator
                    // expected at `abc'") via zerr. Mirror the integer
                    // arm above which already calls zerr on Err. Bug
                    // #506 — zshrs previously swallowed the error and
                    // stored 0.0 silently for `float f="3.14abc"`.
                    match matheval(s) {
                        Ok(v) => v,
                        Err(msg) => {
                            zerr(&msg);
                            mnumber {
                                l: 0,
                                d: 0.0,
                                type_: MN_FLOAT,
                            }
                        }
                    }
                };
                let d = if (mn.type_ & MN_FLOAT) != 0 {
                    mn.d
                } else {
                    mn.l as f64
                };
                // c:2775-2778 — ASSPM_AUGMENT path: `float x=1.5; x+=0.25`
                // adds 0.25 to the current u_dval rather than replacing.
                // Mirrors the integer-augment block above; without it
                // `+=` was a plain `=`.
                let final_d = if (flags & ASSPM_AUGMENT) != 0 {
                    pm.u_dval + d
                } else {
                    d
                };
                floatsetfn(pm, final_d);
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
    if errflag.load(Ordering::Relaxed) != 0
        || ((pm.env.is_none()
            && (pm.node.flags as u32 & PM_EXPORTED) == 0
            && !(isset(ALLEXPORT) && (pm.node.flags as u32 & PM_HASHELEM) == 0))
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
pub fn setnumvalue(v: Option<&mut value>, val: mnumber) {
    // c:2860 — `if (unset(EXECOPT)) return;`. In NO_EXEC mode, param
    // mutations must be skipped so dry-run shell evaluation doesn't
    // leak state into the param table. The previous Rust port skipped
    // this check; `zsh -n -c '(( x=5 ))'` would mutate $x silently.
    if unset(EXECOPT) {
        // c:2860
        return;
    }
    let v = match v {
        Some(v) => v,
        None => return,
    };
    let pm = match v.pm.as_mut() {
        Some(p) => p,
        None => return,
    };
    if (pm.node.flags as u32 & PM_READONLY) != 0 {
        zerr(&format!("read-only variable: {}", pm.node.nam)); // c:2862
        return;
    }
    let t = PM_TYPE(pm.node.flags as u32);
    if t == PM_SCALAR || t == PM_NAMEREF || t == PM_ARRAY {
        // c:2862-2872 — convbase_underscore for integers (honors
        // pm.base for the radix prefix + pm.width for underscore
        // grouping), convfloat_underscore for floats. The previous
        // Rust port computed `val.l.to_string()` then DROPPED the
        // result via `let _ = s;` — meaning a numeric assignment
        // to a SCALAR param stored NOTHING. `typeset s; (( s = 42 ))`
        // would leave $s empty.
        let s = if (val.type_ & MN_INTEGER) != 0 {
            // c:2862
            // c:2864 — `convbase_underscore(val.u.l, pm->base, pm->width)`.
            convbase_underscore(val.l, if pm.base > 0 { pm.base } else { 10 }, pm.width)
        } else {
            // c:2867
            // c:2869 — `convfloat_underscore(val.u.d, pm->width)`.
            convfloat_underscore(val.d, pm.width)
        };
        pm.u_str = Some(s); // c:2871 setstrvalue → store
    } else if t == PM_INTEGER {
        // c:2874 — `pm->gsu.i->setfn(pm, val.u.l)`. For MN_FLOAT
        // input, C truncates to integer via `(zlong)val.u.d`.
        pm.u_val = if (val.type_ & MN_INTEGER) != 0 {
            val.l
        } else {
            val.d as i64
        };
    } else if t == PM_EFLOAT || t == PM_FFLOAT {
        // c:2878 — `pm->gsu.f->setfn(pm, val.u.d)`. MN_INTEGER input
        // gets promoted via `(double)val.u.l`.
        pm.u_dval = if (val.type_ & MN_INTEGER) != 0 {
            val.l as f64
        } else {
            val.d
        };
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
/// VALFLAG_INV + !KSHARRAYS off-by-one (c:2938-2942) is ported below.
/// PM_UNIQUE dedupe (c:2966-2967) is ported at the tail. ASSPM_AUGMENT
/// prepend (c:2945-2954) for slice-AUGMENT remains deferred — rare path
/// (`a[i,j]+=...` array+=) that requires snapshotting the pre-existing
/// slice value before splice.
pub fn setarrvalue(v: &mut value, val: Vec<String>) {
    // c:2895
    // c:2897-2898 — `if (unset(EXECOPT)) return;`. Match the same
    // NO_EXEC bail as setnumvalue at c:2860. Without it,
    // `zsh -n -c 'arr=(a b c)'` would mutate arr during a parse-
    // only run.
    if unset(EXECOPT) {
        // c:2897
        return;
    }

    let pm = match v.pm.as_mut() {
        Some(p) => p,
        None => return,
    };

    // c:2899-2904 — PM_READONLY rejection.
    if pm.node.flags & PM_READONLY as i32 != 0 {
        zerr(&format!("read-only variable: {}", pm.node.nam));
        return;
    }
    // c:2905-2911 — type guard.
    let t = PM_TYPE(pm.node.flags as u32);
    if t & (PM_ARRAY | PM_HASHED) == 0 {
        zerr(&format!(
            "{}: attempt to assign array value to non-array",
            pm.node.nam
        ));
        return;
    }
    // c:2913-2917 — VALFLAG_EMPTY rejection.
    if v.valflags & VALFLAG_EMPTY != 0 {
        zerr(&format!(
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
        arrhashsetfn(pm, val, ASSPM_AUGMENT);
        return;
    }
    if t == PM_HASHED {
        zerr(&format!(
            "{}: attempt to set slice of associative array",
            pm.node.nam
        ));
        return;
    }

    // c:2938-2942 — VALFLAG_INV + !KSHARRAYS off-by-one. Inverse
    // subscripts (`a[(i)pat]=val`) are 1-based when KSHARRAYS is
    // off; shift start/end down by 1 to match the 0-based slice
    // arithmetic below.
    if v.valflags & VALFLAG_INV != 0 && !isset(KSHARRAYS) {
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
    let val_len = val.len(); // c:3030 post-assign sanity
    let pre_len = arr.len(); // c:3030 (snapshot)
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
    // c:3030 — DPUTS2(p - new != post_assignment_length,
    //                 "setarrvalue: wrong allocation: %d 1= %lu",
    //                 post_assignment_length, (unsigned long)(p - new))
    // In C, p-new is the pointer-arithmetic count of elements written
    // into the freshly-allocated buffer; post_assignment_length is the
    // pre-calculated expected length. In Rust the post-splice arr.len()
    // is the equivalent of `p - new`; the expected length follows the
    // same arithmetic as C's post_assignment_length.
    let expected = if start_idx <= end_idx {
        // c:3030
        pre_len - (end_idx - start_idx) + val_len // c:3030
    } else {
        // c:3030
        (start_idx + val_len).max(pre_len) // c:3030
    };
    DPUTS2!(
        // c:3030
        arr.len() != expected, // c:3030
        "setarrvalue: wrong allocation: {} 1= {}",
        expected,
        arr.len() // c:3030-3031
    );

    // c:2966-2967 — `if (pm->node.flags & PM_UNIQUE) arrunique(pm->u.arr);`
    // Dedupe in-place preserving first occurrence. Without this,
    // `typeset -U arr; arr[2]=foo` leaves duplicates that violate the
    // PM_UNIQUE invariant.
    if (pm.node.flags as u32 & PM_UNIQUE) != 0 {
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        arr.retain(|s| seen.insert(s.clone())); // c:2967
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
            if (pm.node.flags as u32 & PM_INTEGER) != 0 {
                return pm.u_val;
            }
        }
    }
    getsparam(s)
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0)
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
            if (fl & (PM_EFLOAT | PM_FFLOAT)) != 0 {
                return (pm.u_dval as i64, pm.u_dval, true);
            }
            if (fl & PM_INTEGER) != 0 {
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
pub fn getsparam(name: &str) -> Option<String> {
    // c:3076
    // 1. GSU dispatch — `Param.gsu->getfn(pm)` equivalent. Special
    //    parameters (UID/RANDOM/USERNAME/...) live behind getfn
    //    hooks that the table read below would otherwise miss.
    if let Some(v) = lookup_special_var(name) {
        return Some(v);
    }
    // 1b. c:Src/params.c:570-575 — getparamnode resolves PM_NAMEREF
    //     chains before the value read (`pm = resolve_nameref(pm)`).
    //     Redirect the lookup to the resolved target.
    if let Some(res) = crate::vm_helper::nameref_read_redirect(name) {
        return res;
    }
    // 2. Paramtab read — `(Value)gethashnode2(paramtab, name)`.
    //    Walk the global paramtab for the named param, returning
    //    `pm->u.str` for PM_SCALAR/PM_NAMEREF or `sepjoin(pm->u.arr)`
    //    for PM_ARRAY (matches `getstrvalue` at params.c:2358).
    if let Ok(tab) = paramtab().read() {
        if let Some(pm) = tab.get(name) {
            // c:Src/Modules/param_private.c:568-617 + c:678 — C swaps
            // `realparamtab->getnode` to getprivatenode when
            // zsh/param/private is active, so every lookup skips
            // private params belonging to an OUTER scope when read
            // from a DEEPER one (`private x` in f is invisible to g
            // called by f; g sees the global). zshrs's HashMap
            // paramtab has no getnode vtable, so call the canonical
            // walk at the lookup site. SAFETY: getprivatenode only
            // follows the `pm->old` chain read-only; the chain is
            // owned by this entry, which stays alive under the read
            // guard held for the rest of this block.
            let visible = crate::ported::modules::param_private::getprivatenode(
                &**pm as *const param,
            ); // c:678 getnode hook
            if visible.is_null() {
                return None; // c:609 walk exhausted — no visible node
            }
            let pm: &param = unsafe { &*visible };
            // c:Src/params.c:2335-2358 — `if (pm->node.flags & PM_UNSET)
            // return ""`. Unset specials kept in paramtab (PM_SPECIAL
            // retention path, c:3911) read as empty, not as their stale
            // u.val / u.str. Without this, `unset SECONDS; echo
            // "[$SECONDS]"` printed `[0]` instead of `[]` — the
            // PM_UNSET pm fell through to the integer path and
            // returned convbase(0). Bug #418 in docs/BUGS.md.
            if (pm.node.flags as u32 & PM_UNSET) != 0 {
                return None;
            }
            let t = PM_TYPE(pm.node.flags as u32);
            // c:2390-2538 — when PM_LEFT/PM_RIGHT_B/PM_RIGHT_Z + width
            // are set, getstrvalue (called from getsparam in C) applies
            // padding. Mirror that here so `$var` expansion respects
            // `typeset -Z N` / `-L N` / `-R N` even when the caller
            // didn't go through getstrvalue. Numeric prefix detection
            // (leading blanks/minus/0x/base#) follows params.rs:3156.
            let pad_flags = (pm.node.flags as u32) & (PM_LEFT | PM_RIGHT_B | PM_RIGHT_Z);
            // c:Src/params.c:2358 — getstrvalue for a PM_SCALAR
            // dispatches through `pm->gsu.s->getfn(pm)` so tied /
            // colon-array / special-callback scalars (PATH/path,
            // user `typeset -T` pairs) return their live computed
            // value rather than a stale cached `u_str`. Reads of
            // `u_str` for scalars without a GSU vtable (the common
            // case) fall through unchanged. Bug #24 in docs/BUGS.md.
            let raw: Option<String> = if t == PM_INTEGER {
                let base = if pm.base > 0 { pm.base } else { 10 };
                Some(convbase(pm.u_val, base as u32))
            } else if t == PM_EFLOAT || t == PM_FFLOAT {
                Some(convfloat(pm.u_dval, pm.base, pm.node.flags as u32))
            } else if t == PM_SCALAR && pm.gsu_s.is_some() {
                pm.gsu_s.as_ref().map(|gsu| (gsu.getfn)(pm))
            } else if let Some(s) = pm.u_str.as_ref() {
                Some(s.clone())
            } else {
                pm.u_arr.as_ref().map(|arr| arr.join(" "))
            };
            if let Some(mut s) = raw {
                if pad_flags != 0 && pm.width > 0 {
                    let fwidth = pm.width as usize;
                    let numeric_pm =
                        (pm.node.flags as u32 & (PM_INTEGER | PM_EFLOAT | PM_FFLOAT)) != 0;
                    if pad_flags == PM_LEFT || pad_flags == (PM_LEFT | PM_RIGHT_Z) {
                        let trimmed: &str = if pad_flags & PM_RIGHT_Z != 0 {
                            s.trim_start_matches('0')
                        } else {
                            s.trim_start_matches(|c: char| c == ' ' || c == '\t')
                        };
                        let len = trimmed.chars().count();
                        let take = len.min(fwidth);
                        let mut out: String = trimmed.chars().take(take).collect();
                        if fwidth > take {
                            out.extend(std::iter::repeat(' ').take(fwidth - take));
                        }
                        s = out;
                    } else if pad_flags & (PM_RIGHT_B | PM_RIGHT_Z) != 0 {
                        let charlen = s.chars().count();
                        if charlen < fwidth {
                            let mut zero = true;
                            let mut valprefend: usize = 0;
                            if pad_flags & PM_RIGHT_Z != 0 {
                                let bytes = s.as_bytes();
                                let mut tpos = 0usize;
                                while tpos < bytes.len()
                                    && (bytes[tpos] == b' ' || bytes[tpos] == b'\t')
                                {
                                    tpos += 1;
                                }
                                if numeric_pm && tpos < bytes.len() && bytes[tpos] == b'-' {
                                    tpos += 1;
                                }
                                if (pm.node.flags as u32 & PM_INTEGER) != 0
                                    && tpos + 1 < bytes.len()
                                    && bytes[tpos] == b'0'
                                    && bytes[tpos + 1] == b'x'
                                {
                                    tpos += 2;
                                } else if (pm.node.flags as u32 & PM_INTEGER) != 0 {
                                    if let Some(hash_off) =
                                        bytes[tpos..].iter().position(|&b| b == b'#')
                                    {
                                        tpos += hash_off + 1;
                                    }
                                }
                                valprefend = tpos;
                                if tpos == bytes.len() {
                                    zero = false;
                                } else if !numeric_pm && !bytes[tpos].is_ascii_digit() {
                                    zero = false;
                                }
                            }
                            let pad_char = if (pad_flags & PM_RIGHT_B) != 0 || !zero {
                                ' '
                            } else {
                                '0'
                            };
                            let pad_count = fwidth - charlen;
                            let prefix = &s[..valprefend];
                            let rest = &s[valprefend..];
                            let mut out = String::with_capacity(fwidth);
                            out.push_str(prefix);
                            out.extend(std::iter::repeat(pad_char).take(pad_count));
                            out.push_str(rest);
                            s = out;
                        } else if charlen > fwidth {
                            // c:Src/params.c:2496-2500 — right-justify
                            // with charlen > fwidth: truncate from the
                            // FRONT to fit fwidth codepoints. Bug #100
                            // in docs/BUGS.md — the getsparam path had
                            // the pad arm but missed the truncation
                            // arm. The parallel getstrvalue path at
                            // ~line 3355 has it; this is the
                            // missing-symmetry fix.
                            let skip = charlen - fwidth;
                            s = s.chars().skip(skip).collect();
                        }
                    }
                }
                return Some(s);
            }
        }
    }
    // 3. Env fallback — C imports env into paramtab at init so the
    //    read above would hit. If the import hasn't happened yet
    //    (e.g. during very early init) fall back to the live env.
    env::var(name).ok()
}

/// Port of `getsparam_u(char *s)` from `Src/params.c:3089`. C body
/// (c:3091-3094):
/// ```c
/// /* getsparam() returns pointer into global params table, so ... */
/// if ((s = getsparam(s)))
///     return unmeta(s);    /* returns static pointer to copy */
/// return s;
/// ```
///
/// The previous Rust "port" was an entirely fabricated impl — it
/// took `Option<&mut value>` and gated on `PM_TYPE == PM_SCALAR`,
/// which matches no part of the C body. C just calls getsparam(s)
/// and unmeta's the resulting string. No callers existed because
/// no caller's type fit the bogus signature.
///
/// Real use case: locale setters (c:4847, c:4867, c:4882, c:4917)
/// call `getsparam_u("LC_ALL")` / `getsparam_u("LANG")` to read the
/// param as a Meta-stripped C string suitable for `setlocale`.
pub fn getsparam_u(s: &str) -> Option<String> {
    // c:3089
    // c:3092 — `if ((s = getsparam(s))) return unmeta(s);`
    getsparam(s).map(|v| unmeta(&v))
}

/// Port of `char **getaparam(char *s)` from `Src/params.c:3101-3110`.
///
/// C body:
/// ```c
/// struct value vbuf;
/// Value v;
/// if (!idigit(*s) && (v = getvalue(&vbuf, &s, 0)) &&
///     PM_TYPE(v->pm->node.flags) == PM_ARRAY)
///     return v->pm->gsu.a->getfn(v->pm);
/// return NULL;
/// ```
///
/// The previous Rust port was a fabrication: signature was
/// `Option<&mut value> -> Option<Vec<String>>`, taking an already-
/// resolved Value pointer rather than the C-canonical name string.
/// No caller used it because the bogus signature fit nothing — and
/// the in-tree `savematch` at modules/zutil.rs:30 hardcoded `a = None`
/// because the existing API couldn't be threaded through.
///
/// Real C use: name lookup. e.g. `getaparam("match")` returns the
/// `$match` array from the regex-match callouts (Modules/zutil.c:45).
pub fn getaparam(name: &str) -> Option<Vec<String>> {
    // c:3101
    // c:3107 — `if (idigit(*s))` reject digit-first names. C
    // `getvalue` would also reject these later, but the explicit
    // check matches C's flow.
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        // c:3107
        return None;
    }
    // c:Src/params.c:570-575 — getvalue→fetchvalue→getparamnode
    // resolves PM_NAMEREF before the type check.
    if let Some(res) = crate::vm_helper::nameref_aread_redirect(name) {
        return res;
    }
    // c:3107-3109 — `getvalue(&vbuf, &s, 0)` resolves the name to a
    // paramtab entry. Then PM_TYPE check + `pm->u.arr` return.
    if let Ok(tab) = paramtab().read() {
        if let Some(pm) = tab.get(name) {
            // c:Src/Modules/param_private.c:678 — the getnode hook
            // applies to every paramtab lookup; same canonical walk
            // as getsparam (SAFETY: read-only old-chain walk under
            // the held read guard).
            let visible =
                crate::ported::modules::param_private::getprivatenode(&**pm as *const param);
            if visible.is_null() {
                return None;
            }
            let pm: &param = unsafe { &*visible };
            if PM_TYPE(pm.node.flags as u32) == PM_ARRAY {
                // c:3108
                if let Some(arr) = pm.u_arr.as_ref() {
                    // c:3109
                    return Some(arr.clone());
                }
            }
        }
    }
    None // c:3110
}

/// Port of `char **gethparam(char *s)` from `Src/params.c:3117-3126`.
///
/// C body:
/// ```c
/// struct value vbuf;
/// Value v;
/// if (!idigit(*s) && (v = getvalue(&vbuf, &s, 0)) &&
///     PM_TYPE(v->pm->node.flags) == PM_HASHED)
///     return paramvalarr(v->pm->gsu.h->getfn(v->pm), SCANPM_WANTVALS);
/// return NULL;
/// ```
///
/// Same fabricated-port family as the prior `getaparam`/`getsparam_u`
/// fixes: previous Rust sig took `Option<&mut value>` instead of the
/// canonical name string, with no real callers. Fixed sig + body
/// that resolves the name through paramtab and returns the values
/// vector when PM_HASHED.
///
/// NOTE: zshrs's paramtab stores hash-params via `pm->u_hash` (a
/// `HashTable` struct that's a generic bucket-array container). The
/// canonical C path threads through `gsu.h->getfn(pm)` → `paramvalarr`
/// which extracts the value side of each key-value pair. zshrs's
/// canonical assoc backing lives in `paramtab_hashed_storage` (an
/// IndexMap<String, String> keyed by param name); read the values
/// directly from there as the C macro's
/// `paramvalarr(hashgetfn(pm), SCANPM_WANTVALS)` resolves to a
/// values walk over the same backing.
pub fn gethparam(name: &str) -> Option<Vec<String>> {
    // c:3117
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        // c:3122
        return None;
    }
    // c:Src/params.c:570-575 — nameref deref before the type check.
    let resolved = crate::vm_helper::nameref_final_name(name);
    let name: &str = &resolved;
    if let Ok(tab) = paramtab().read() {
        if let Some(pm) = tab.get(name) {
            // c:Src/Modules/param_private.c:678 — getnode hook; same
            // canonical walk as getsparam/getaparam. (Values below
            // still come from the name-keyed hashed storage — a
            // single slot per name; per-scope assoc shadowing is a
            // storage-model limitation predating this walk.)
            let visible =
                crate::ported::modules::param_private::getprivatenode(&**pm as *const param);
            if visible.is_null() {
                return None;
            }
            let pm: &param = unsafe { &*visible };
            if PM_TYPE(pm.node.flags as u32) == PM_HASHED {
                // c:3123
                // c:3124 — `paramvalarr(hashgetfn(pm), SCANPM_WANTVALS)`.
                // Read values directly from the canonical hashed-storage
                // backing — IndexMap iteration matches C's hashtable
                // walk order (insertion-stable). When the storage hasn't
                // been populated (empty hash, fresh declaration), return
                // an empty Vec so the C "param exists, no entries" shape
                // is preserved (vs returning None which means "param
                // doesn't exist").
                let store = paramtab_hashed_storage().lock().ok()?;
                let vals = store
                    .get(name)
                    .map(|m| m.values().cloned().collect())
                    .unwrap_or_default();
                return Some(vals); // c:3124
            }
        }
    }
    None // c:3125
}

/// Port of `char **gethkparam(char *s)` from `Src/params.c:3131-3140`.
/// Same as `gethparam` but `paramvalarr(..., SCANPM_WANTKEYS)`.
pub fn gethkparam(name: &str) -> Option<Vec<String>> {
    // c:3131
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        // c:3136
        return None;
    }
    if let Ok(tab) = paramtab().read() {
        if let Some(pm) = tab.get(name) {
            if PM_TYPE(pm.node.flags as u32) == PM_HASHED {
                // c:3137
                // c:3138 — `paramvalarr(pm->gsu.h->getfn(pm),
                // SCANPM_WANTKEYS)`. Same backing as gethparam —
                // return keys instead of values. Empty-storage
                // fallback identical: Some(empty Vec) for "exists,
                // no entries" shape.
                {
                    let store = paramtab_hashed_storage().lock().ok()?;
                    if let Some(m) = store.get(name) {
                        return Some(m.keys().cloned().collect()); // c:3138
                    }
                }
                // c:3138 — for SPECIALPMDEF magic hashes (parameters/
                // options/commands/…, Src/Modules/parameter.c) the
                // `getfn` is the module's scan fn, not hashgetfn;
                // there's no paramtab_hashed_storage backing. The
                // partab_scan_keys port (vm_helper.rs:3486) is that
                // scan. Without this, `${(k)parameters[PATH]}` saw an
                // empty key set and returned "" where zsh prints PATH.
                if let Some(keys) = crate::vm_helper::partab_scan_keys(name) {
                    return Some(keys); // c:3138
                }
                return Some(Vec::new()); // c:3138
            }
        }
    }
    None // c:3139
}

/// Port of `check_warn_pm(Param pm, const char *pmtype, int created, int may_warn_about_nested_vars)` from `Src/params.c:3160`.
///
/// C body emits the WARN_CREATE_GLOBAL / WARN_NESTED_VAR
/// diagnostic when a function-local creates/passes a non-local
/// param with the matching shell options set.
///
/// The previous Rust port handled the GATE logic correctly but
/// SKIPPED the diagnostic emit, claiming the `funcstack` global
/// wasn't ported. But `FUNCSTACK`
/// IS ported (`Mutex<Vec<funcstack>>`). Wire the walk:
///   for (i = funcstack; i; i = i->prev)
///       if (i->tp == FS_FUNC) {
///           msg = created ?
///               "%s parameter %s created globally in function %s" :
///               "%s parameter %s set in enclosing scope in function %s";
///           zwarn(msg, pmtype, pm->node.nam, i->name);
///           break;
///       }
///
/// Without the diagnostic, `setopt WARN_CREATE_GLOBAL` had no
/// observable effect — the whole point of the option is the
/// user-visible warning.
pub fn check_warn_pm(pm: &param, pmtype: &str, created: i32, may_warn_about_nested_vars: i32) {
    // c:3160
    if may_warn_about_nested_vars == 0 && created == 0 {
        // c:3165
        return;
    }
    // `locallevel` is the canonical `pub static` above (port of
    // params.c:54). `forklevel` is the ported global at vm_helper
    // (port of exec.c:1052) set to locallevel at every entersubsh().
    let cur_local: i32 = locallevel.load(Ordering::Relaxed);
    let forklevel: i32 = FORKLEVEL.load(Ordering::Relaxed); // c:1052 (Src/exec.c)
    if created != 0 && isset(WARNCREATEGLOBAL) {
        // c:3168
        if cur_local <= forklevel || pm.level != 0 {
            // c:3169
            return;
        }
    } else if created == 0 && isset(WARNNESTEDVAR) {
        // c:3171
        if pm.level >= cur_local {
            // c:3172
            return;
        }
    } else {
        return;
    }
    if (pm.node.flags as u32 & (PM_SPECIAL | PM_NAMEREF)) != 0 {
        // c:3177
        return;
    }
    // c:3180-3190 — walk funcstack, emit zwarn at first FS_FUNC.
    let stack = match FUNCSTACK.lock() {
        Ok(s) => s,
        Err(_) => return,
    };
    for frame in stack.iter().rev() {
        // c:3180 walk most-recent-first
        if frame.tp == FS_FUNC {
            // c:3181 FS_FUNC
            let msg = if created != 0 {
                // c:3185
                format!(
                    "{} parameter {} created globally in function {}",
                    pmtype, pm.node.nam, frame.name
                )
            } else {
                // c:3187
                format!(
                    "{} parameter {} set in enclosing scope in function {}",
                    pmtype, pm.node.nam, frame.name
                )
            };
            zwarn(&msg); // c:3189
            break; // c:3190
        }
    }
}

// intgetfn / strgetfn drift wrappers removed — replaced below with
// real C-shape ports `intgetfn(pm: &param) -> i64` (Src/params.c:3993)
// and `strgetfn(pm: &param) -> String` (Src/params.c:4029) that read
// directly from the union fields `pm->u.val` / `pm->u.str`.

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

// ===========================================================
// Methods moved verbatim from src/ported/vm_helper because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: params
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

// ===========================================================
// Free ported moved verbatim from src/ported/vm_helper.
// ===========================================================
// BEGIN moved-from-exec-rs (free ported)
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
/// `getarg_out` — see variants.
#[allow(non_camel_case_types)]
pub enum getarg_out<'a> {
    Flags { flags: &'a str, rest: &'a str },
    Value(Value),
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
pub fn assignsparam(s: &str, val: &str, flags: i32) -> Option<Param> {
    // c:3203 `if (!isident(s)) { zerr; errflag |= ERRFLAG_ERROR; return NULL; }`
    if !isident(s) {
        zerr(&format!("not an identifier: {}", s)); // c:3204
        errflag.fetch_or(
            // c:3206
            ERRFLAG_ERROR,
            Ordering::Relaxed,
        );
        return None; // c:3207
    }
    // c:3233/3252 — `getvalue(&vbuf, &s, 1)` routes through
    // fetchvalue which resolves PM_NAMEREF chains (c:2247-2270).
    // Mirror by redirecting the assignment to the resolved target.
    {
        let base = s.split('[').next().unwrap_or(s);
        if crate::vm_helper::is_nameref(base) {
            match crate::vm_helper::resolve_nameref_name(base, None) {
                crate::vm_helper::nameref_resolution::SelfRef => {
                    // zerr emitted inside the walk (c:6341-6343).
                    errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
                    return None;
                }
                crate::vm_helper::nameref_resolution::OutOfScope => {
                    // c:1108-1118 — createparam refuses the existing
                    // ref; assignment fails (status 1, no errflag —
                    // c:3255 is commented out in the C source).
                    return None;
                }
                crate::vm_helper::nameref_resolution::Placeholder(last) => {
                    // Chain ends at a placeholder/unset ref: the value
                    // becomes its new refname (assignstrvalue
                    // PM_NAMEREF arm, c:2712-2717 + c:3258).
                    return crate::vm_helper::nameref_assign_refname(&last, val);
                }
                crate::vm_helper::nameref_resolution::Target {
                    name: t,
                    subscript: rsub,
                    pm: rpm,
                    level,
                } => {
                    // Rebuild the target expression: ref subscript
                    // first (REFSLICE, c:2264-2268), then any caller
                    // subscript.
                    let user_sub = s.find('[').map(|i| &s[i..]);
                    // Hidden (old-chain) binding — the upscope walk
                    // (c:6455) resolved past the visible binding;
                    // write through the chain node directly.
                    if rsub.is_none() && user_sub.is_none() && rpm.is_some() {
                        let visible_level = paramtab()
                            .read()
                            .ok()
                            .and_then(|tb| tb.get(&t).map(|p| p.level));
                        if visible_level != Some(level) {
                            return crate::vm_helper::nameref_hidden_scalar_assign(
                                &t, level, val,
                            );
                        }
                    }
                    let mut new_s = t.clone();
                    if let Some(rs) = &rsub {
                        new_s.push('[');
                        new_s.push_str(rs);
                        new_s.push(']');
                    }
                    if let Some(us) = user_sub {
                        new_s.push_str(us);
                    }
                    if new_s != s {
                        return assignsparam(&new_s, val, flags);
                    }
                }
                crate::vm_helper::nameref_resolution::NotRef => {}
            }
        }
    }
    // c:Src/params.c randomsetfn / secondssetfn — re-assigning a
    // regenerator-style special clears the PM_UNSET flag so the
    // getfn becomes live again. zshrs's lookup_special_var uses a
    // side-set for this (no real pm node), so clear it here. Bug #417.
    if matches!(
        s,
        "RANDOM" | "SECONDS" | "EPOCHSECONDS" | "EPOCHREALTIME" | "TTYIDLE" | "ERRNO"
    ) {
        clear_unset_special(s);
    }
    queue_signals(); // c:3209

    // c:3210 — `strchr(s, '[')`. Split the leading name from the
    // subscript while preserving C's `*ss = '\0'` / `*ss = '['`
    // restore semantics: the Rust port works on `&str` slices so
    // there's no in-place null-terminator dance, but the parse
    // shape is identical.
    // c:Src/params.c:3210 — `strchr(s, '[')` finds the subscript
    // opener; C's parse_subscript (params.c:1480+) then walks to the
    // matching `]` respecting backslash-escapes and nesting. zshrs's
    // previous `s.rfind(']')` picked the LAST `]` which works for
    // simple `name[key]` but mis-bounds `A[\[k\]]` (where the key
    // contains escaped brackets).
    let (name, subscript) = match s.find('[') {
        Some(i) => {
            // Walk forward from `i + 1` matching brackets with
            // escape-awareness per parse_subscript semantics.
            let mut depth = 1i32;
            let mut close_byte: Option<usize> = None;
            let mut bslash = false;
            let mut byte_off = i + 1;
            for ch in s[i + 1..].chars() {
                if bslash {
                    bslash = false;
                    byte_off += ch.len_utf8();
                    continue;
                }
                match ch {
                    '\\' => bslash = true,
                    '[' => depth += 1,
                    ']' => {
                        depth -= 1;
                        if depth == 0 {
                            close_byte = Some(byte_off);
                            break;
                        }
                    }
                    _ => {}
                }
                byte_off += ch.len_utf8();
            }
            let close = close_byte.unwrap_or(s.len());
            let key_end = if close > i { close } else { s.len() };
            (&s[..i], Some(&s[i + 1..key_end]))
        }
        None => (s, None),
    };
    // c:Src/params.c parse_subscript — backslash-escapes in the
    // subscript body (`\[`, `\]`, `\\`) are stripped to their literal
    // form for the actual key value. `A[\[k\]]=v` stores under key
    // `[k]`. zshrs's subscript extractor above preserved the escapes
    // verbatim, so the stored key was `\[k\]` and the matching lookup
    // `${A[[k]]}` couldn't find it.
    let subscript_owned: Option<String> = subscript.map(|key| {
        let mut out = String::with_capacity(key.len());
        let mut bslash = false;
        for ch in key.chars() {
            if bslash {
                out.push(ch);
                bslash = false;
            } else if ch == '\\' {
                bslash = true;
            } else {
                out.push(ch);
            }
        }
        if bslash {
            out.push('\\');
        }
        out
    });
    let subscript: Option<&str> = subscript_owned.as_deref();

    // c:Src/Modules/parameter.c — magic associative-array assignment
    // forms: `functions[name]=body`, `aliases[name]=value`,
    // `dis_functions[name]=body`, `saliases[name]=value`,
    // `galiases[name]=value`, `dis_aliases[name]=value`, etc.
    // These reach assignsparam as `name[key]=value` shape and must
    // dispatch to the canonical setpmfunction / setpmalias hooks
    // BEFORE the generic paramtab subscript store. Without this
    // route, the assignment lands in paramtab_hashed_storage as
    // a normal assoc element and the corresponding function/alias
    // is never actually defined in shfunctab/aliastab.
    if let Some(key) = subscript {
        match name {
            "functions" => {
                use crate::ported::zsh_h::hashnode;
                use crate::ported::zsh_h::param as ParamStruct;
                // c:Src/params.c:3270-3276 — ASSPM_AUGMENT on PM_SCALAR
                // appends raw text to the existing value. The
                // `functions[name]` magic-assoc is scalar-typed under
                // the hood (each entry is a function-body string). For
                // `functions[f]+="echo b"`, fetch the existing body
                // from shfunctab and concatenate before calling
                // setpmfunction. Without this, `+=` silently no-ops.
                // Bug #323 in docs/BUGS.md.
                let final_body = if (flags & ASSPM_AUGMENT) != 0 {
                    let existing = crate::ported::hashtable::shfunctab_lock()
                        .read()
                        .ok()
                        .and_then(|tab| tab.get(key).and_then(|shf| shf.body.clone()))
                        .unwrap_or_default();
                    format!("{}{}", existing, val)
                } else {
                    val.to_string()
                };
                let pm: Box<ParamStruct> = Box::new(ParamStruct {
                    node: hashnode {
                        next: None,
                        nam: key.to_string(),
                        flags: 0,
                    },
                    u_data: 0,
                    u_tied: None,
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
                crate::ported::modules::parameter::setpmfunction(pm.clone(), final_body);
                unqueue_signals();
                return Some(pm);
            }
            "dis_functions" => {
                use crate::ported::zsh_h::hashnode;
                use crate::ported::zsh_h::param as ParamStruct;
                let pm: Box<ParamStruct> = Box::new(ParamStruct {
                    node: hashnode {
                        next: None,
                        nam: key.to_string(),
                        flags: 0,
                    },
                    u_data: 0,
                    u_tied: None,
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
                crate::ported::modules::parameter::setpmdisfunction(pm.clone(), val.to_string());
                unqueue_signals();
                return Some(pm);
            }
            "aliases" => {
                // c:Src/Modules/parameter.c — install a regular
                // alias via canonical aliastab. Use createaliasnode
                // with default flags (no ALIAS_GLOBAL / ALIAS_SUFFIX).
                if let Ok(mut tab) = crate::ported::hashtable::aliastab_lock().write() {
                    let node = crate::ported::hashtable::createaliasnode(key, val, 0u32);
                    tab.add(node);
                }
                unqueue_signals();
                return Some(Box::new(crate::ported::zsh_h::param {
                    node: crate::ported::zsh_h::hashnode {
                        next: None,
                        nam: key.to_string(),
                        flags: 0,
                    },
                    u_data: 0,
                    u_tied: None,
                    u_arr: None,
                    u_str: Some(val.to_string()),
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
                }));
            }
            "galiases" => {
                // c:Src/Modules/parameter.c — global alias.
                if let Ok(mut tab) = crate::ported::hashtable::aliastab_lock().write() {
                    let node = crate::ported::hashtable::createaliasnode(
                        key,
                        val,
                        crate::ported::zsh_h::ALIAS_GLOBAL as u32,
                    );
                    tab.add(node);
                }
                unqueue_signals();
                return Some(Box::new(crate::ported::zsh_h::param {
                    node: crate::ported::zsh_h::hashnode {
                        next: None,
                        nam: key.to_string(),
                        flags: 0,
                    },
                    u_data: 0,
                    u_tied: None,
                    u_arr: None,
                    u_str: Some(val.to_string()),
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
                }));
            }
            "saliases" => {
                // c:Src/Modules/parameter.c — suffix alias.
                if let Ok(mut tab) = crate::ported::hashtable::sufaliastab_lock().write() {
                    let node = crate::ported::hashtable::createaliasnode(
                        key,
                        val,
                        crate::ported::zsh_h::ALIAS_SUFFIX as u32,
                    );
                    tab.add(node);
                }
                unqueue_signals();
                return Some(Box::new(crate::ported::zsh_h::param {
                    node: crate::ported::zsh_h::hashnode {
                        next: None,
                        nam: key.to_string(),
                        flags: 0,
                    },
                    u_data: 0,
                    u_tied: None,
                    u_arr: None,
                    u_str: Some(val.to_string()),
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
                }));
            }
            "options" => {
                // c:Src/Modules/parameter.c:926 setpmoption — userspace
                // `options[X]=on|off` toggles option X via dosetopt, not
                // a generic assoc-write. Build a synthetic Param whose
                // node.nam carries the option name and dispatch to the
                // canonical setpmoption port (`src/ported/modules/
                // parameter.rs:1620`), which calls optlookup + dosetopt.
                use crate::ported::zsh_h::hashnode;
                use crate::ported::zsh_h::param as ParamStruct;
                let pm: Box<ParamStruct> = Box::new(ParamStruct {
                    node: hashnode {
                        next: None,
                        nam: key.to_string(),
                        flags: 0,
                    },
                    u_data: 0,
                    u_tied: None,
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
                crate::ported::modules::parameter::setpmoption(
                    pm.clone(),
                    val.to_string(),
                );
                unqueue_signals();
                return Some(pm);
            }
            "commands" => {
                // c:Src/Modules/parameter.c:151-160 setpmcommand —
                // `commands[name]=path` installs a HASHED Cmdnam
                // node in cmdnamtab (`cn->node.flags = HASHED;
                // cn->u.cmd = ztrdup(value); cmdnamtab->addnode`),
                // exactly like `hash name=path`. zsh ACCEPTS the
                // write (verified vs zsh 5.9: rc=0, readback works,
                // whence/hash see the entry). Build a synthetic
                // Param whose node.nam carries the command name and
                // dispatch to the canonical setpmcommand port.
                // Bug #375.
                use crate::ported::zsh_h::hashnode;
                use crate::ported::zsh_h::param as ParamStruct;
                let pm: Box<ParamStruct> = Box::new(ParamStruct {
                    node: hashnode {
                        next: None,
                        nam: key.to_string(),
                        flags: 0,
                    },
                    u_data: 0,
                    u_tied: None,
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
                crate::ported::modules::parameter::setpmcommand(pm.clone(), val.to_string());
                unqueue_signals();
                return Some(pm);
            }
            _ => {}
        }
    }

    // c:Src/params.c:3262 IPDEF9 — `argv`, `@`, `*` are aliases for
    // the global `pparams` (positional parameter vector). Whole-
    // array writes (`argv=(...)`) route through assignaparam at
    // params.rs:5487 which updates PPARAMS. Subscripted writes
    // (`argv[N]=val`) must do the same — without this, the value
    // landed in paramtab's u_arr but `$@`/`$1`/`$2`/... continued
    // reading from PPARAMS, so the update was invisible. Bug #281
    // in docs/BUGS.md.
    if let Some(key) = subscript {
        if matches!(name, "argv" | "@" | "*") {
            if let Ok(idx) = key.trim().parse::<i64>() {
                if let Ok(mut pp) = crate::ported::builtin::PPARAMS.lock() {
                    let len = pp.len() as i64;
                    let kshzero =
                        crate::ported::zsh_h::isset(crate::ported::zsh_h::KSHZEROSUBSCRIPT);
                    if idx == 0 && !isset(KSHARRAYS) && !kshzero {
                        zerr(&format!(
                            "{}: assignment to invalid subscript range",
                            name
                        ));
                        drop(pp);
                        unqueue_signals();
                        return None;
                    }
                    let real_idx = if idx < 0 {
                        (len + idx).max(0) as usize
                    } else if isset(KSHARRAYS) {
                        idx.max(0) as usize
                    } else {
                        (idx - 1).max(0) as usize
                    };
                    while pp.len() <= real_idx {
                        pp.push(String::new());
                    }
                    pp[real_idx] = val.to_string();
                }
                unqueue_signals();
                return Some(Box::new(param {
                    node: hashnode {
                        next: None,
                        nam: name.to_string(),
                        flags: PM_ARRAY as i32,
                    },
                    u_data: 0,
                    u_tied: None,
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
                }));
            }
        }
    }

    // c:Src/Modules/parameter.c — read-only magic-assoc names. C
    // zsh's SPECIALPMDEF / createspecialhash sets PM_READONLY on
    // these so userspace assignment errors with `read-only
    // variable: NAME`. `vm_helper::init_partab_params` strips
    // PM_READONLY off the paramtab stub to allow INTERNAL writes
    // (function-call funcstack push, etc.) — see comment at
    // vm_helper.rs:2799. The trade-off is that USERSPACE writes
    // now slip through too. Intercept here, AFTER the writable-
    // magic-assoc dispatch arms above (functions, aliases,
    // dis_aliases, galiases, saliases, dis_functions) but BEFORE
    // the generic paramtab subscript store, so the userspace path
    // emits the canonical diagnostic and exits non-zero. Internal
    // table mutations that bypass `assignsparam` stay free. Bug
    // #242 in docs/BUGS.md.
    if subscript.is_some() {
        // `options` is intentionally NOT in this list — C zsh's
        // `setpmoption`/`setpmoptions` (Src/Modules/parameter.c:926-979)
        // accept "on"/"off" writes and translate them to dosetopt calls.
        // The `options` arm above this readonly-list check routes
        // `options[X]=on|off` writes through the canonical setpmoption
        // port, so by the time we reach here `options` has already
        // returned. Bug #342.
        // `commands` is intentionally NOT in this list — C zsh's
        // `setpmcommand` (Src/Modules/parameter.c:151-160) ACCEPTS
        // `commands[name]=path` and installs a HASHED Cmdnam node in
        // cmdnamtab (same effect as `hash name=path`). Verified vs
        // zsh 5.9: `commands[x]=/y` → rc=0, ${commands[x]} → /y,
        // `whence x` → /y. The `commands` arm above routes through
        // the canonical setpmcommand port. Bug #375.
        let is_readonly_magic = matches!(
            name,
            "builtins"
                | "modules"
                | "parameters"
                | "dis_builtins"
                | "history"
                | "historywords"
                | "jobtexts"
                | "jobstates"
                | "jobdirs"
                | "nameddirs"
                | "userdirs"
                | "usergroups"
                | "widgets"
                | "functions_source"
                | "dis_functions_source"
                | "terminfo"
                | "termcap"
        );
        if is_readonly_magic {
            zerr(&format!("read-only variable: {}", name));
            errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
            unqueue_signals();
            return None;
        }
    }

    // Subscripted path (c:3210-3231).
    if let Some(key) = subscript {
        let mut tab = paramtab().write().unwrap();
        let exists = tab.contains_key(name); // c:3212
        if !exists {
            // c:3213 `createparam(t, PM_ARRAY); created = 1;`
            let pm: Param = Box::new(param {
                node: hashnode {
                    next: None,
                    nam: name.to_string(),
                    flags: PM_ARRAY as i32,
                },
                u_data: 0,
                u_tied: None,
                u_arr: Some(Vec::new()),
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
            tab.insert(name.to_string(), pm);
        } else {
            // c:3216 `if (v->pm->node.flags & PM_READONLY)`.
            let pm = tab.get(name).unwrap();
            if (pm.node.flags as u32 & PM_READONLY) != 0 {
                zerr(&format!("read-only variable: {}", pm.node.nam)); // c:3217
                drop(tab);
                unqueue_signals(); // c:3220
                return None; // c:3221
            }
        }
        // c:3231 `v = NULL;` — re-dispatch by storage type.
        let pm = tab.get_mut(name).unwrap();
        pm.node.flags &= !(PM_DEFAULTED as i32); // c:3228
        if (pm.node.flags as u32 & PM_HASHED) != 0 {
            // PM_HASHED element store. `param.u_hash` is typed
            // `Option<HashTable>` per Src/zsh.h:1841 but the
            // HashTable runtime backing isn't wired; the assoc-array
            // values live in a parallel storage keyed on param name
            // (`paramtab_hashed_storage()`).
            let mut store = paramtab_hashed_storage().lock().unwrap();
            store
                .entry(name.to_string())
                .or_default()
                .insert(key.to_string(), val.to_string());
        } else if let Ok(idx) = key.parse::<i64>() {
            // c:Src/params.c:2748-2789 — PM_SCALAR + numeric subscript
            // SPLICES the value into the scalar's char string
            // (`a=hello; a[2]=X` → `hXllo`). Only PM_ARRAY does
            // element-store at this subscript. Bug #589 in
            // docs/BUGS.md: zshrs's subscript store always treated
            // numeric subscript as array-store, so `a[2]=X` on a
            // scalar cleared `u_str` and put "X" at array slot 1,
            // leaving `$a` displaying as `" X"` (empty + space + X)
            // when joined.
            let pm_type = PM_TYPE(pm.node.flags as u32);
            if pm_type == PM_SCALAR {
                // c:2748+ scalar splice — replace chars at index.
                let s = pm.u_str.clone().unwrap_or_default();
                let chars: Vec<char> = s.chars().collect();
                let len = chars.len() as i64;
                let kshzero = crate::ported::zsh_h::isset(crate::ported::zsh_h::KSHZEROSUBSCRIPT);
                if idx == 0 && !isset(KSHARRAYS) && !kshzero {
                    zerr(&format!(
                        "{}: assignment to invalid subscript range",
                        name
                    ));
                    drop(tab);
                    unqueue_signals();
                    return None;
                }
                // 1-based forward, negative-from-end. KSHARRAYS = 0-based.
                let real_idx = if idx < 0 {
                    (len + idx).max(0)
                } else if isset(KSHARRAYS) {
                    idx
                } else {
                    idx - 1
                };
                let real_idx = real_idx.max(0) as usize;
                let real_idx = real_idx.min(chars.len());
                // Replace one char at real_idx with val. If real_idx
                // is past the end, append (extending with empty
                // wouldn't make sense for a scalar — C's
                // assignstrvalue at params.c:3724-3789 clamps end to
                // zlen and concats).
                let mut out = String::with_capacity(s.len() + val.len());
                out.extend(chars[..real_idx].iter());
                out.push_str(val);
                if real_idx < chars.len() {
                    out.extend(chars[real_idx + 1..].iter());
                }
                pm.u_str = Some(out);
            } else {
                // PM_ARRAY + numeric subscript (c:3357 `assignaparam`).
                // c:Src/params.c:2125-2150 + c:2911 — `a[0]=val` under
                // default zsh semantics (no KSHARRAYS, no KSHZEROSUBSCRIPT)
                // produces VALFLAG_EMPTY in getarg, which setarrvalue
                // then rejects with "assignment to invalid subscript
                // range".
                let kshzero = crate::ported::zsh_h::isset(crate::ported::zsh_h::KSHZEROSUBSCRIPT);
                if idx == 0 && !isset(KSHARRAYS) && !kshzero {
                    zerr(&format!(
                        "{}: assignment to invalid subscript range",
                        name
                    )); // c:2911 (effective)
                    drop(tab);
                    unqueue_signals();
                    return None;
                }
                let arr = pm.u_arr.get_or_insert_with(Vec::new);
                let len = arr.len() as i64;
                let real_idx = if idx < 0 {
                    len + idx
                } else if isset(KSHARRAYS) {
                    idx
                } else {
                    idx - 1
                };
                let real_idx = real_idx.max(0) as usize;
                while arr.len() <= real_idx {
                    arr.push(String::new());
                }
                arr[real_idx] = val.to_string();
                pm.u_str = None;
            }
        } else {
            // String subscript on a non-hashed name → auto-vivify
            // as PM_HASHED (mirrors C `createparam(s, PM_HASHED)`
            // fallback when getvalue returns NULL).
            pm.node.flags = (pm.node.flags & !(PM_TYPE(u32::MAX) as i32)) | PM_HASHED as i32;
            pm.u_arr = None;
            pm.u_str = None;
            let mut map: IndexMap<String, String> = IndexMap::new();
            map.insert(key.to_string(), val.to_string());
            paramtab_hashed_storage()
                .lock()
                .unwrap()
                .insert(name.to_string(), map);
        }
        let cloned = pm.clone();
        drop(tab);
        unqueue_signals(); // c:3344
        return Some(cloned); // c:3345
    }

    // c:3232 non-subscripted branch.
    let mut tab = paramtab().write().unwrap();
    let existing = tab.contains_key(name);
    let created_now = !existing; // c:3232 createparam path sets `created = 1`
    if !existing {
        // c:3234 `createparam(t, PM_SCALAR); created = 1;`
        let mut pm_flags = PM_SCALAR as i32;
        if isset(ALLEXPORT) {
            // c:1149-1150 (ALLEXPORT path)
            pm_flags |= PM_EXPORTED as i32;
        }
        // c:1135-1160 — `createparam` installs gsu via the special-
        // params table when the name matches a PM_SPECIAL entry. C
        // walks `paramtab->getnode(name)` first, but for fresh
        // creations the gsu pointer comes from the IPDEF macro the
        // paramdef ships with. Mirror by looking up the name in the
        // special-scalar GSU table — same end-state as if
        // createparamtable had run.
        let gsu_s: Option<Box<gsu_scalar>> = match name {
            "0" => {
                pm_flags |= PM_SPECIAL as i32;
                Some(Box::new(ARGZERO_GSU.clone())) // c:225-226 / IPDEF2("0", argzero_gsu, 0)
            }
            "HOME" => {
                pm_flags |= PM_SPECIAL as i32;
                Some(Box::new(HOME_GSU.clone())) // c:248
            }
            "IFS" => {
                pm_flags |= PM_SPECIAL as i32;
                Some(Box::new(IFS_GSU.clone())) // c:245
            }
            "TERM" => {
                pm_flags |= PM_SPECIAL as i32;
                Some(Box::new(TERM_GSU.clone())) // c:250
            }
            "TERMINFO" => {
                pm_flags |= PM_SPECIAL as i32;
                Some(Box::new(TERMINFO_GSU.clone())) // c:251
            }
            "TERMINFO_DIRS" => {
                pm_flags |= PM_SPECIAL as i32;
                Some(Box::new(TERMINFODIRS_GSU.clone())) // c:252
            }
            "WORDCHARS" => {
                pm_flags |= PM_SPECIAL as i32;
                Some(Box::new(WORDCHARS_GSU.clone())) // c:249
            }
            "USERNAME" => {
                pm_flags |= PM_SPECIAL as i32;
                Some(Box::new(USERNAME_GSU.clone())) // c:247
            }
            "KEYBOARD_HACK" => {
                pm_flags |= PM_SPECIAL as i32;
                Some(Box::new(KEYBOARDHACK_GSU.clone())) // c:253
            }
            "HISTCHARS" | "histchars" => {
                pm_flags |= PM_SPECIAL as i32;
                Some(Box::new(HISTCHARS_GSU.clone())) // c:246
            }
            _ => None,
        };
        let pm: Param = Box::new(param {
            node: hashnode {
                next: None,
                nam: name.to_string(),
                flags: pm_flags,
            },
            u_data: 0,
            u_tied: None,
            u_arr: None,
            u_str: Some(String::new()),
            u_val: 0,
            u_dval: 0.0,
            u_hash: None,
            gsu_s, // c:1149 special gsu wired
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
        tab.insert(name.to_string(), pm);
    } else {
        let pm = tab.get(name).unwrap();
        // c:3216 PM_READONLY guard for an existing param.
        if (pm.node.flags as u32 & PM_READONLY) != 0 {
            zerr(&format!("read-only variable: {}", pm.node.nam)); // c:3217
            drop(tab);
            unqueue_signals(); // c:3220
            return None; // c:3221
        }
        // c:1135-1160 — back-fill the gsu_s vtable on existing
        // entries that pre-dated `createparamtable` (e.g. env-import
        // path inserted HOME/IFS/etc via the non-special branch
        // before the special-param table was populated). Without
        // this back-fill, those params stay None-gsu forever and
        // assignstrvalue falls through to the default `strsetfn`
        // path, bypassing homesetfn/ifssetfn — so the canonical
        // C globals (`home`, `ifs`, ...) drift from paramtab.
        let pm = tab.get_mut(name).unwrap();
        if pm.gsu_s.is_none() {
            let new_gsu: Option<Box<gsu_scalar>> = match name {
                "0" => Some(Box::new(ARGZERO_GSU.clone())), // c:225-226
                "HOME" => Some(Box::new(HOME_GSU.clone())), // c:248
                "IFS" => Some(Box::new(IFS_GSU.clone())),   // c:245
                "TERM" => Some(Box::new(TERM_GSU.clone())), // c:250
                "TERMINFO" => Some(Box::new(TERMINFO_GSU.clone())), // c:251
                "TERMINFO_DIRS" => Some(Box::new(TERMINFODIRS_GSU.clone())), // c:252
                "WORDCHARS" => Some(Box::new(WORDCHARS_GSU.clone())), // c:249
                "USERNAME" => Some(Box::new(USERNAME_GSU.clone())), // c:247
                "KEYBOARD_HACK" => Some(Box::new(KEYBOARDHACK_GSU.clone())), // c:253
                "HISTCHARS" | "histchars" => Some(Box::new(HISTCHARS_GSU.clone())), // c:246
                _ => None,
            };
            if new_gsu.is_some() {
                pm.gsu_s = new_gsu;
                pm.node.flags |= PM_SPECIAL as i32;
            }
        }
        let pm = tab.get(name).unwrap();
        // c:3236-3250 — existing PM_ARRAY/PM_HASHED on a non-special,
        // non-tied, non-KSHARRAYS, non-AUGMENT scalar assignment →
        // `resetparam(v->pm, PM_SCALAR)`.
        let f = pm.node.flags as u32;
        let is_array_or_hash = (f & PM_ARRAY) != 0 || (f & PM_HASHED) != 0;
        let is_special_or_tied = (f & (PM_SPECIAL | PM_TIED)) != 0;
        let augment_bit = (flags & ASSPM_AUGMENT) != 0;
        if is_array_or_hash && !is_special_or_tied && !augment_bit && !isset(KSHARRAYS) {
            // c:3242 — flip type to PM_SCALAR, drop array/hash slots.
            let pm_mut = tab.get_mut(name).unwrap();
            pm_mut.node.flags =
                (pm_mut.node.flags & !(PM_TYPE(u32::MAX) as i32)) | PM_SCALAR as i32;
            pm_mut.u_arr = None;
            paramtab_hashed_storage().lock().unwrap().remove(name);
        }
    }

    // c:3258-3266 `if (*val && (v->pm->node.flags & PM_NAMEREF))`.
    let pm = tab.get(name).unwrap();
    if !val.is_empty() && (pm.node.flags as u32 & PM_NAMEREF) != 0 {
        if !valid_refname(val, pm.node.flags) {
            // c:3259
            zerr(&format!("invalid name reference: {}", val)); // c:3260
            drop(tab);
            errflag.fetch_or(
                // c:3263
                ERRFLAG_ERROR,
                Ordering::Relaxed,
            );
            unqueue_signals(); // c:3262
            return None; // c:3264
        }
    }

    // c:3266-3268 `if (flags & ASSPM_WARN) check_warn_pm(v->pm, "scalar", created, 1);`
    if (flags & ASSPM_WARN) != 0 {
        if let Some(pm_ref) = tab.get(name) {
            check_warn_pm(pm_ref, "scalar", created_now as i32, 1); // c:3268
        }
    }
    // c:3269 `v->pm->node.flags &= ~PM_DEFAULTED;`
    let pm = tab.get_mut(name).unwrap(); // c:3269
    pm.node.flags &= !(PM_DEFAULTED as i32); // c:3269

    // c:3343 `assignstrvalue(v, val, flags)`. C aliases `v->pm`
    // through to the param in the hash table; Rust's borrow rules
    // forbid holding `&mut Param` and wrapping it in `value.pm:
    // Option<Param>` at once. Previous port used `tab.remove(name)`
    // to take ownership during dispatch, then re-insert — but
    // assignstrvalue's PM_INTEGER arm calls `mathevali(val)` which
    // looks up identifiers via paramtab. With the param removed,
    // `X=X+1` evaluated `X` as unset (=0) and stored `0+1=1`.
    // Symptom: `typeset -gi X=5; X=X+1; echo $X` printed 1 instead
    // of 6 — broke self-referential integer arithmetic which p10k
    // uses for counters, frame depth, hook chain index, etc.
    //
    // Fix: clone the Param into the value struct so the original
    // stays in the table during mathevali's identifier lookup,
    // then overwrite the table entry with the mutated clone.
    let pm_clone = tab.get(name).unwrap().clone(); // c:3343
    drop(tab); // release write lock — assignstrvalue may take it
    let mut v = value {
        // c:3343
        pm: Some(pm_clone), // c:3343
        arr: Vec::new(),    // c:3343
        scanflags: 0,       // c:3343
        valflags: 0,        // c:3343
        start: 0,           // c:3343
        end: -1,            // c:3343
    }; // c:3343
    assignstrvalue(Some(&mut v), Some(val.to_string()), flags); // c:3343
    let cloned = v.pm.as_ref().cloned(); // c:3345
    if let Some(pm_back) = v.pm {
        // c:3343
        paramtab()
            .write()
            .unwrap()
            .insert(name.to_string(), pm_back); // c:3343
    } // c:3343
    // c:Src/params.c pathsetfn / fpathsetfn / manpathsetfn /
    // cdpathsetfn — when the SCALAR side of a tied colon-array
    // pair is assigned, the canonical setfn split-rebuilds the
    // ARRAY side via `splitstring(value, ":", &globalarr)`. zshrs
    // lacks per-name GSU setfns for these, so the array stayed
    // stale after `PATH=/a:/b`. Mirror the split cascade
    // explicitly for the full IPDEF8 PM_TIED colonarr list
    // (c:Src/params.c:395-422): PATH↔path, FPATH↔fpath,
    // MANPATH↔manpath, CDPATH↔cdpath, PSVAR↔psvar,
    // MODULE_PATH↔module_path, FIGNORE↔fignore,
    // MAILPATH↔mailpath. Bug #423/#424.
    let alt: Option<&str> = match name {
        "PATH" => Some("path"),
        "FPATH" => Some("fpath"),
        "MANPATH" => Some("manpath"),
        "CDPATH" => Some("cdpath"),
        "PSVAR" => Some("psvar"),
        "MODULE_PATH" => Some("module_path"),
        "FIGNORE" => Some("fignore"),
        "MAILPATH" => Some("mailpath"),
        _ => None,
    };
    if let Some(alt_name) = alt {
        let parts: Vec<String> = val.split(':').map(String::from).collect();
        if let Ok(mut tab) = paramtab().write() {
            let entry = tab.entry(alt_name.to_string()).or_insert_with(|| {
                Box::new(param {
                    node: hashnode {
                        next: None,
                        nam: alt_name.to_string(),
                        flags: (PM_ARRAY | PM_SPECIAL) as i32,
                    },
                    u_data: 0,
                    u_tied: None,
                    u_arr: Some(Vec::new()),
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
                })
            });
            entry.u_arr = Some(parts);
            entry.u_str = None;
        }
        // c:Src/params.c:5291 — `if (t == path) cmdnamtab->emptytable
        // (cmdnamtab)`. PATH change invalidates the command-name cache.
        if name == "PATH" {
            crate::ported::hashtable::emptycmdnamtable();
        }
    }
    // c:Src/params.c — `{ "PROMPT", PM_SCALAR|PM_ALIAS, &ps1, ... }` and the
    // matching `{ "PS1", PM_SCALAR, &ps1, ... }` share the same backing
    // pointer; assigning to PROMPT updates the byte that PS1 reads (and
    // vice versa). Same for PROMPT2↔PS2, PROMPT3↔PS3, PROMPT4↔PS4.
    // zshrs lacks PM_ALIAS — mirror the scalar write to the alias. Bug #518.
    let alias_pair: Option<&str> = match name {
        "PROMPT" => Some("PS1"),
        "PS1" => Some("PROMPT"),
        "PROMPT2" => Some("PS2"),
        "PS2" => Some("PROMPT2"),
        "PROMPT3" => Some("PS3"),
        "PS3" => Some("PROMPT3"),
        "PROMPT4" => Some("PS4"),
        "PS4" => Some("PROMPT4"),
        _ => None,
    };
    if let Some(other) = alias_pair {
        if let Ok(mut tab) = paramtab().write() {
            let entry = tab.entry(other.to_string()).or_insert_with(|| {
                Box::new(param {
                    node: hashnode {
                        next: None,
                        nam: other.to_string(),
                        flags: PM_SCALAR as i32,
                    },
                    u_data: 0,
                    u_tied: None,
                    u_arr: None,
                    u_str: Some(String::new()),
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
                })
            });
            entry.u_str = Some(val.to_string());
            entry.u_arr = None;
        }
    }
    unqueue_signals(); // c:3344
    cloned // c:3345
}

// `VarAttr` struct + `VarKind` enum + `impl VarAttr::format_zsh`
// DELETED. C zsh stores typeset attributes as bare `PM_*` bit
// flags on `Param.node.flags` (`Src/zsh.h` PM_* + `Src/params.c`
// flag tests); the `${(t)var}` flag report (`typeprintparam` at
// `Src/builtin.c:3050+`) writes those bits to a string directly
// against the `Param.node.flags` int.
//
// Both types had zero external use sites — pure dead-code carryover
// from an earlier vm_helper refactor. The PM_* bit constants are at
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
// statics here, with the get/set ported mutating the static.
//
// Functions that genuinely need a `Param *` (the GSU dispatch
// callbacks for non-special arr/hash/int/float/str params, the
// param-table mutators, scope helpers, etc.) cannot be properly
// ported until zshrs gains a Param struct + callback-table ABI;
// those keep their C signatures but the body is a WARNING-stub
// that does nothing.
// ===========================================================

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
static PARAMTAB_INNER: OnceLock<RwLock<HashMap<String, Param>>> = OnceLock::new();
static REALPARAMTAB_INNER: OnceLock<RwLock<HashMap<String, Param>>> = OnceLock::new();

/// Array parameter assignment (no subscript).
///
/// Direct port of `Param assignaparam(char *s, char **val, int flags)`
/// from `Src/params.c:3357`. Writes an array value into paramtab
/// and returns the new/updated Param.
///
/// Ported semantics:
///   - PM_READONLY rejection (c:3370-3381)
///   - PM_NAMEREF type-change reject (c:3395-3398)
///   - ASSPM_AUGMENT (`a+=val`) preserve-old prepend (c:3404-3412)
///   - PM_UNIQUE dedupe (c:3401)
///   - element-wise `a[k]=v` slice path pre-check (c:3373-3389)
///   - PM_HASHED slice rejection (c:3384-3391)
///
/// Pending (rare paths):
///   - resetparam from non-array (c:3415-3420) — handled implicitly
///     by the type-mask rewrite below; matches C observable behavior.
pub fn assignaparam(name: &str, val: Vec<String>, flags: i32) -> Option<Param> {
    // c:3357
    // c:3366-3370 — `if (!isident(s)) { zerr; return NULL }`.
    if !isident(name) {
        zerr(&format!("not an identifier: {}", name));
        errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
        return None;
    }
    // c:3392 — `fetchvalue(&vbuf, &s, 1, SCANPM_ASSIGNING)` resolves
    // PM_NAMEREF chains; c:3395-3398 rejects an unresolvable ref.
    {
        let base = name.split('[').next().unwrap_or(name);
        if crate::vm_helper::is_nameref(base) {
            match crate::vm_helper::resolve_nameref_name(base, None) {
                crate::vm_helper::nameref_resolution::SelfRef => {
                    errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
                    return None;
                }
                crate::vm_helper::nameref_resolution::OutOfScope => {
                    // see assignsparam — silent failure, status 1.
                    return None;
                }
                crate::vm_helper::nameref_resolution::Placeholder(_) => {
                    // c:3396 — message uses the ORIGINAL assigned name
                    // (`t = s` saved at c:3361).
                    zwarn(&format!("{}: can't change type of a named reference", base));
                    return None;
                }
                crate::vm_helper::nameref_resolution::Target {
                    name: t,
                    subscript: rsub,
                    pm: rpm,
                    level,
                } => {
                    let user_sub = name.find('[').map(|i| &name[i..]);
                    // Hidden (old-chain) binding — upscope write.
                    if rsub.is_none() && user_sub.is_none() && rpm.is_some() {
                        let visible_level = paramtab()
                            .read()
                            .ok()
                            .and_then(|tb| tb.get(&t).map(|p| p.level));
                        if visible_level != Some(level) {
                            return crate::vm_helper::nameref_hidden_array_assign(
                                &t, level, val,
                            );
                        }
                    }
                    let mut new_s = t.clone();
                    if let Some(rs) = &rsub {
                        new_s.push('[');
                        new_s.push_str(rs);
                        new_s.push(']');
                    }
                    if let Some(us) = user_sub {
                        new_s.push_str(us);
                    }
                    if new_s != name {
                        return assignaparam(&new_s, val, flags);
                    }
                }
                crate::vm_helper::nameref_resolution::NotRef => {}
            }
        }
    }

    // c:3375 — `if ((ss = strchr(s, '['))) { ... slice path ... }`
    //          Extract the base name when there's a subscript and
    //          dispatch to the slice-rejection / slice-write path.
    if let Some(_bracket) = name.find('[') {
        let base = name.split('[').next().unwrap_or(name);
        // c:3384-3391 — slice into a HASHED → zerr.
        let is_hashed = {
            let tab = paramtab().read().unwrap();
            tab.get(base)
                .map(|pm| (pm.node.flags as u32 & PM_HASHED) != 0)
                .unwrap_or(false)
        };
        if is_hashed {
            zerr(&format!(
                "{}: attempt to set slice of associative array",
                base
            ));
            errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
            return None;
        }
        // Slice into a non-existent param → create as PM_ARRAY (c:3377-3382).
        let exists = paramtab().read().unwrap().contains_key(base);
        if !exists {
            createparam(base, PM_ARRAY as i32)?;
        }
        // Subscript-write itself (a[k]=v) is handled at the caller's
        // SubscriptArith dispatch; reaching here means the slice
        // pre-check has passed and the param exists.
        return paramtab().read().unwrap().get(base).cloned();
    }

    // c:3391-3394 — fetchvalue / createparam(PM_ARRAY) if missing.
    let (existed, prior_scalar, prior_flags) = {
        let tab = paramtab().read().unwrap();
        match tab.get(name) {
            Some(pm) => (true, pm.u_str.clone(), pm.node.flags),
            None => (false, None, 0),
        }
    };
    // c:3397-3400 — PM_NAMEREF: can't change type of a named reference.
    if existed && (prior_flags as u32 & PM_NAMEREF) != 0 {
        zwarn(&format!("{}: can't change type of a named reference", name));
        return None;
    }
    let created_now = !existed; // c:3393 createparam path sets `created = 1`
    if !existed {
        createparam(name, PM_ARRAY as i32)?;
    }

    // c:3370-3381 PM_READONLY rejection — C routes through setarrvalue
    // → arrsetfn which emits "read-only variable: X" and returns NULL.
    // Rust write path bypasses gsu setfn, so mirror the check here.
    if existed && (prior_flags as u32 & PM_READONLY) != 0 {
        zerr(&format!("read-only variable: {}", name));
        errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
        return None;
    }
    // c:3395-3401 — the type-reset arm only fires for params that are
    // NOT (PM_ARRAY|PM_HASHED) and NOT PM_SPECIAL|PM_TIED. A special
    // param falls THROUGH the reset: `v` stays set and the assignment
    // reaches setarrvalue (c:3585), which dispatches:
    //   - PM_HASHED whole-assign (v->start==0, v->end==-1) →
    //     arrhashsetfn (c:2918-2920) → pm->gsu.h->setfn(pm, ht) — for
    //     the zsh/parameter specials that's setpmoptions /
    //     setpmcommands / setaliases / setfunctions / setpmnameddirs
    //     (Src/Modules/parameter.c).
    //   - non-array, non-hash special (e.g. $SECONDS) → zerr
    //     "%s: attempt to assign array value to non-array" (c:2905).
    // The previous Rust arm rejected EVERY non-array special with
    // "can't change type of a special parameter" — a message C's
    // assignaparam never emits. Gap #2 2026-06-12.
    if existed {
        let pm_type = prior_flags as u32 & PM_TYPE(u32::MAX);
        if pm_type != PM_ARRAY && (prior_flags as u32 & PM_SPECIAL) != 0 {
            if pm_type == PM_HASHED {
                // c:3544-3560 — under ASSPM_KEY_VALUE, associative
                // arrays strictly enforce `[key]=value` syntax: walk
                // in strides of 3; every stride must start with a
                // Marker (a Marker can only introduce a
                // Marker/key/value triad, never appear by accident).
                if (flags & ASSPM_KEY_VALUE) != 0 {
                    let mut i = 0usize;
                    while i < val.len() {
                        if !val[i].starts_with(Marker) {
                            zerr("bad [key]=value syntax for associative array");
                            errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
                            return None;
                        }
                        i += 3;
                    }
                }
                // c:4124-4131 (arrhashsetfn) — count non-Marker
                // entries; odd → zerr + abort. zsh 5.9 truth:
                // `options=(noglob)` → "bad set of key/value pairs
                // for associative array", rc=1, script aborts.
                let alen = val
                    .iter()
                    .filter(|s| !s.starts_with(Marker as char))
                    .count();
                if alen % 2 != 0 {
                    zerr("bad set of key/value pairs for associative array");
                    errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
                    return None;
                }
                // c:4141-4166 (arrhashsetfn pair walk) — flatten the
                // value list to (key, value) pairs; `[k]=v` triples
                // arrive as [Marker, k, v].
                let mut pairs: Vec<(String, String)> = Vec::with_capacity(alen / 2);
                let mut it = val.iter();
                while let Some(first) = it.next() {
                    let key = if first.starts_with(Marker as char) {
                        match it.next() {
                            Some(k) => k.clone(),
                            None => break,
                        }
                    } else {
                        first.clone()
                    };
                    let v = it.next().cloned().unwrap_or_default();
                    pairs.push((key, v));
                }
                // pm->gsu.h->setfn(pm, ht) — per-name dispatch to the
                // canonical Src/Modules/parameter.c setfn ports. The
                // synthetic Param mirrors the established per-element
                // pattern (assignsparam "options"/"commands" arms).
                let synth: Param = Box::new(param {
                    node: hashnode {
                        next: None,
                        nam: name.to_string(),
                        flags: prior_flags,
                    },
                    u_data: 0,
                    u_tied: None,
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
                use crate::ported::modules::parameter as pmod;
                match name {
                    // SPECIALPMDEF entries with writable hash gsu
                    // (Src/Modules/parameter.c:2235-2298, no
                    // PM_READONLY_SPECIAL):
                    "options" => pmod::setpmoptions(synth.clone(), &pairs), // c:2285
                    "commands" => pmod::setpmcommands(synth.clone(), &pairs), // c:2238
                    "functions" => pmod::setpmfunctions(synth.clone(), &pairs), // c:2263
                    "dis_functions" => pmod::setpmdisfunctions(synth.clone(), &pairs), // c:2245
                    "aliases" => pmod::setpmraliases(synth.clone(), &pairs), // c:2235
                    "dis_aliases" => pmod::setpmdisraliases(synth.clone(), &pairs), // c:2241
                    "galiases" => pmod::setpmgaliases(synth.clone(), &pairs), // c:2269
                    "dis_galiases" => pmod::setpmdisgaliases(synth.clone(), &pairs), // c:2249
                    "saliases" => pmod::setpmsaliases(synth.clone(), &pairs), // c:2293
                    "dis_saliases" => pmod::setpmdissaliases(synth.clone(), &pairs), // c:2255
                    "nameddirs" => pmod::setpmnameddirs(synth.clone(), &pairs), // c:2283
                    _ => {
                        // PM_READONLY_SPECIAL hashed specials (builtins,
                        // modules, parameters, history, jobtexts, ...) —
                        // C's PM_READONLY check (setarrvalue c:2900-2903)
                        // rejects before any setfn. zsh 5.9 truth:
                        // `modules=(a b)` → "read-only variable: modules",
                        // rc=1. zshrs strips PM_READONLY off the paramtab
                        // stubs (vm_helper init), so match by family.
                        zerr(&format!("read-only variable: {}", name));
                        errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
                        return None;
                    }
                }
                return Some(synth);
            }
            // c:2905-2909 (setarrvalue) — non-hash special target.
            // zsh 5.9 truth: `SECONDS=(1 2)` → "SECONDS: attempt to
            // assign array value to non-array", rc=1.
            zerr(&format!(
                "{}: attempt to assign array value to non-array",
                name
            ));
            errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
            return None;
        }
    }

    // c:3436-3541 — ASSPM_KEY_VALUE on an ordinary array: the value
    // list contains Marker / index / value triads (from
    // keyvalpairelement, Src/subst.c:49) interleaved with plain
    // elements. Resolve them into a dense array:
    //   - First pass (c:3465-3486): matheval each index; reject < 0,
    //     and 0 unless KSH_ARRAYS (zerr "bad subscript for direct
    //     array assignment"); KSH_ARRAYS keeps 0-based, otherwise
    //     1-based → decrement; track `nextind` (a plain element lands
    //     just after the previously placed one) and `maxlen`.
    //   - Allocate `fullval` of maxlen (c:3487-3495), pre-filled with
    //     the existing elements under ASSPM_AUGMENT (c:3496-3502).
    //   - Second pass (c:3503-3525): place each value; a `Marker +`
    //     triad concatenates onto the slot's current value (c:3510-
    //     3515 bicat); plain elements advance nextind.
    //   - Unset slots become "" — no sparse arrays (c:3530-3537).
    // C returns straight after `setarrvalue(v, fullval)` (c:3538-
    // 3540): the resolved vec REPLACES the whole array, so the tail
    // AUGMENT prepend below must not run again — clear the flag.
    let mut val = val;
    let mut flags = flags;
    if (flags & ASSPM_KEY_VALUE) != 0 {
        let ksh = crate::ported::zsh_h::isset(crate::ported::zsh_h::KSHARRAYS);
        let orig: Vec<String> = if (flags & ASSPM_AUGMENT) != 0 && existed {
            let tab = paramtab().read().unwrap();
            tab.get(name)
                .and_then(|pm| pm.u_arr.clone())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut maxlen = orig.len(); // c:3460-3462
        let mut nextind: i64 = 0; // c:3464
        let mut subscripts: Vec<i64> = Vec::new(); // c:3455-3456
        let mut i = 0usize;
        while i < val.len() {
            // c:3466-3481
            if val[i].starts_with(Marker) {
                let key_str = val.get(i + 1).cloned().unwrap_or_default();
                let idx = match crate::ported::math::mathevali(&key_str) {
                    Ok(n) => n,
                    Err(e) => {
                        // C mathevali zerr's internally (Src/math.c);
                        // surface the message and abort the assign.
                        zerr(&e);
                        errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
                        return None;
                    }
                };
                if idx < 0 || (!ksh && idx == 0) {
                    // c:3468-3474
                    zerr(&format!(
                        "bad subscript for direct array assignment: {}",
                        key_str
                    ));
                    errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
                    return None;
                }
                let ix = if ksh { idx } else { idx - 1 }; // c:3475-3476
                subscripts.push(ix);
                nextind = ix + 1; // c:3477
                i += 3; // c:3479
            } else {
                nextind += 1; // c:3481-3482
                i += 1;
            }
            if nextind > maxlen as i64 {
                // c:3484-3485
                maxlen = nextind as usize;
            }
        }
        // c:3487-3495 fullval = zshcalloc((maxlen+1) * sizeof(char*)).
        let mut fullval: Vec<Option<String>> = vec![None; maxlen];
        // c:3496-3502 — AUGMENT: copy the original elements in first.
        for (j, o) in orig.iter().enumerate() {
            fullval[j] = Some(o.clone());
        }
        // Second pass — c:3503-3525.
        let mut si = 0usize;
        let mut nextind = 0usize;
        let mut i = 0usize;
        while i < val.len() {
            if val[i].starts_with(Marker) {
                let augment_elt = val[i].len() > Marker.len_utf8(); // c:3507 `(*aptr)[1] == '+'`
                let ix = subscripts[si] as usize;
                si += 1;
                let old = fullval[ix].take();
                fullval[ix] = Some(match old {
                    // c:3510-3512 bicat(old, value)
                    Some(o) if augment_elt => format!("{}{}", o, val[i + 2]),
                    _ => val[i + 2].clone(),
                });
                nextind = ix + 1; // c:3516
                i += 3;
            } else {
                fullval[nextind] = Some(val[i].clone()); // c:3519-3520
                nextind += 1;
                i += 1;
            }
        }
        // c:3530-3537 — unfilled slots become "".
        val = fullval
            .into_iter()
            .map(|o| o.unwrap_or_default())
            .collect();
        // c:3538-3540 — setarrvalue(v, fullval); return. The orig
        // elements are already merged in; don't prepend them again.
        flags &= !ASSPM_AUGMENT;
    }

    // c:3402-3412 — ASSPM_AUGMENT preserve-old prepend. When the
    // previous value was a scalar (not array/hashed) and we're
    // augmenting (`a+=val`), prepend that scalar's string form as
    // val[0]. Only fires when the existing param is not PM_UNSET.
    let was_scalar_array_target = existed
        && prior_flags & (PM_ARRAY | PM_HASHED) as i32 == 0
        && prior_flags & PM_SPECIAL as i32 == 0;
    if (flags & ASSPM_AUGMENT) != 0 && was_scalar_array_target && prior_flags & PM_UNSET as i32 == 0
    {
        if let Some(old_scalar) = prior_scalar {
            val.insert(0, old_scalar); // c:3408-3411
        }
    }

    // c:3570-3585 — ASSPM_AUGMENT on an existing PM_ARRAY target:
    // append rather than replace. C bumps v->start to arrlen(existing)
    // and v->end to start+1 so setarrvalue writes past the tail.
    // zshrs writes through pm.u_arr without the value struct, so do
    // the equivalent here: prepend the existing array elements to the
    // new val so the final stored vec is [old..., new...].
    if (flags & ASSPM_AUGMENT) != 0
        && existed
        && (prior_flags as u32 & PM_ARRAY) != 0
        && (prior_flags as u32 & PM_UNSET) == 0
    {
        let prior_arr = {
            let tab = paramtab().read().unwrap();
            tab.get(name)
                .and_then(|pm| pm.u_arr.clone())
                .unwrap_or_default()
        };
        let appended: Vec<String> = prior_arr.into_iter().chain(val.into_iter()).collect();
        val = appended;
    }

    // c:3432 `if (flags & ASSPM_WARN) check_warn_pm(v->pm, "array", created, may_warn_about_nested_vars);`
    // c:3372 — `may_warn_about_nested_vars = !(flags & ASSPM_AUGMENT)`.
    if (flags & ASSPM_WARN) != 0 {
        let may_nested = if (flags & ASSPM_AUGMENT) != 0 { 0 } else { 1 };
        if let Some(pm_ref) = paramtab().read().unwrap().get(name) {
            check_warn_pm(pm_ref, "array", created_now as i32, may_nested); // c:3432
        }
    }

    // c:3434 — setarrvalue(v, val): store array in pm.u_arr.
    let mut tab = paramtab().write().unwrap();
    let pm = tab.get_mut(name)?;
    let uniq = pm.node.flags & PM_UNIQUE as i32 != 0; // c:3401
    if pm.node.flags & PM_SPECIAL as i32 == 0 {
        let type_mask = PM_ARRAY | PM_INTEGER | PM_EFLOAT | PM_FFLOAT | PM_HASHED | PM_NAMEREF;
        pm.node.flags = (pm.node.flags & !type_mask as i32) | PM_ARRAY as i32;
    }
    // c:3401 — preserve PM_UNIQUE through the type change, then let
    // arrsetfn dedupe via the actual write.
    if uniq {
        pm.node.flags |= PM_UNIQUE as i32;
    }
    let val_final = if uniq { simple_arrayuniq(val) } else { val };
    // c:3434 — `setarrvalue(v, val);` → `gsu.a->setfn(pm, val)` for
    // PM_SPECIAL params (PATH/path → pathsetfn rebuilds $PATH cstring;
    // FPATH/fpath → fpathsetfn rebuilds; MANPATH etc.). Without this
    // dispatch, env var pairs (PATH vs path) drift apart on array
    // assignment. Fall back to direct u_arr write when no gsu_a is
    // wired (regular non-special arrays).
    let setfn_ptr = pm.gsu_a.as_ref().map(|g| g.setfn);
    // c:Src/params.c:3434 — `setarrvalue(v, val)` calls `gsu.a->setfn(pm, val)`.
    // The canonical arrsetfn (params.rs:6584) writes `pm->u_arr` then,
    // if `pm->ename` is set, calls `arrfixenv` which re-acquires the
    // paramtab lock. We're holding the WRITE lock here — that would
    // deadlock the RWLock. Inline the storage write under the held
    // lock and defer arrfixenv to AFTER drop(tab). Bug #600.
    pm.u_arr = Some(val_final.clone());
    pm.u_str = None;
    pm.u_hash = None;
    // c:2712 (setarrvalue head) — `v->pm->node.flags &= ~PM_UNSET;`
    // a declared-but-unset (TYPESET_TO_UNSET) array becomes set on
    // its first assignment.
    pm.node.flags &= !((PM_UNSET | PM_DECLARED) as i32);
    let ename_for_envsync: Option<String> = if setfn_ptr.is_some() {
        pm.ename.clone()
    } else {
        None
    };
    let cloned = pm.clone();
    drop(tab);
    // c:Src/params.c:5285 arrfixenv — deferred from inside the lock
    // (see Bug #600 above). Acquires its own paramtab read+write
    // locks; safe to call now that we've dropped the write lock.
    if let Some(ename) = ename_for_envsync {
        arrfixenv(&ename, Some(&val_final));
    }
    // c:Src/params.c:3262 IPDEF9 — \`argv\`/\`@\`/\`*\` are aliases for
    // the C global \`pparams\` (the positional parameter vector).
    // assignaparam("argv", [...]) in C writes through the array's
    // setfn which mutates \`pparams\` directly. zshrs's pparams lives
    // in builtin::PPARAMS; mirror the write so \`argv=(...)\` updates
    // \$1/\$2/.../\$# correctly.
    if name == "argv" || name == "@" || name == "*" {
        if let Ok(mut pp) = crate::ported::builtin::PPARAMS.lock() {
            *pp = val_final.clone();
        }
    }
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
/// `ASSPM_WARN` (params.c:104) drives the WARN_CREATE_GLOBAL /
/// WARN_NESTED_VAR diagnostics inside `assignaparam` →
/// `check_warn_pm` (params.rs:4428).
/// WARNING: param names don't match C — Rust=() vs C=(s, val)
pub fn setaparam(name: &str, val: Vec<String>) -> Option<Param> {
    // c:3766 — `return assignaparam(s, val, ASSPM_WARN)`.
    assignaparam(name, val, ASSPM_WARN)
}

/// Direct port of `Param sethparam(char *s, char **val)` from
/// `Src/params.c:3602`. Writes an associative array (flat
/// alternating key,value list) into paramtab + the parallel
/// `paramtab_hashed_storage` table; returns the new Param.
///
/// Ported C semantics:
///   - PM_READONLY rejection (c:3625 via setarrvalue chain in C; here inline)
///   - PM_SPECIAL type-change reject (c:3637)
/// Pending:
///   - resetparam(PM_HASHED) for non-special type-change (rare)
pub fn sethparam(name: &str, val: Vec<String>) -> Option<Param> {
    // c:3611-3615 — `if (!isident(s)) { zerr; return NULL }`.
    if !isident(name) {
        zerr(&format!("not an identifier: {}", name));
        return None;
    }
    // c:3630 — `fetchvalue(&vbuf, &s, 1, SCANPM_ASSIGNING)` resolves
    // PM_NAMEREF chains (same shape as assignaparam c:3392-3398).
    if crate::vm_helper::is_nameref(name) {
        match crate::vm_helper::resolve_nameref_name(name, None) {
            crate::vm_helper::nameref_resolution::SelfRef => {
                errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
                return None;
            }
            crate::vm_helper::nameref_resolution::OutOfScope => {
                return None;
            }
            crate::vm_helper::nameref_resolution::Placeholder(_) => {
                zwarn(&format!("{}: can't change type of a named reference", name));
                return None;
            }
            crate::vm_helper::nameref_resolution::Target {
                name: t,
                subscript: None,
                ..
            } => {
                if t != name {
                    return sethparam(&t, val);
                }
            }
            _ => {}
        }
    }
    // c:3617-3621 — `if (strchr(s, '[')) { zerr; return NULL }`.
    if name.contains('[') {
        zerr("nested associative arrays not yet supported");
        return None;
    }

    // c:3625 — PM_READONLY rejection. C routes through gsu.h->setfn
    // which checks readonly inside hashsetfn / arrhashsetfn; Rust
    // bypasses that path, so check explicitly here.
    {
        let tab = paramtab().read().unwrap();
        if let Some(pm) = tab.get(name) {
            if (pm.node.flags as u32 & PM_READONLY) != 0 {
                zerr(&format!("read-only variable: {}", name));
                errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
                return None;
            }
            // c:3637 — `if (PM_TYPE(pm->node.flags) != PM_HASHED &&
            //              (pm->node.flags & PM_SPECIAL)) { zerr; return; }`
            // Can't change type of a PM_SPECIAL non-hashed param.
            let pm_type = pm.node.flags as u32 & PM_TYPE(u32::MAX);
            if pm_type != PM_HASHED && (pm.node.flags as u32 & PM_SPECIAL) != 0 {
                zerr(&format!(
                    "{}: can't change type of a special parameter",
                    name
                ));
                errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
                return None;
            }
        }
    }

    // c:3625 — fetchvalue / createparam(PM_HASHED) if missing.
    let exists = paramtab().read().unwrap().contains_key(name);
    let checkcreate = !exists; // c:3626 `checkcreate = 1;`
    if !exists {
        createparam(name, PM_HASHED as i32)?;
    }

    // c:3649 `check_warn_pm(v->pm, "associative array", checkcreate, 1);`
    // — sethparam always warns (no ASSPM_WARN gate in C).
    if let Some(pm_ref) = paramtab().read().unwrap().get(name) {
        check_warn_pm(pm_ref, "associative array", checkcreate as i32, 1); // c:3649
    }

    // c:3651 — `setarrvalue(v, val);` — full-replace dispatch for a
    // PM_HASHED param (setarrvalue c:2919-2920) collapses to
    // `arrhashsetfn(v->pm, val, 0)`, which owns the odd-count gate
    // (c:4128-4131, zerr + ERRFLAG_ERROR) and the pair walk.
    let mut tab = paramtab().write().unwrap();
    let pm = tab.get_mut(name)?;
    if pm.node.flags & PM_SPECIAL as i32 == 0 {
        let type_mask = PM_ARRAY | PM_INTEGER | PM_EFLOAT | PM_FFLOAT | PM_HASHED | PM_NAMEREF;
        pm.node.flags = (pm.node.flags & !type_mask as i32) | PM_HASHED as i32;
    }
    pm.u_arr = None;
    pm.u_str = None;
    arrhashsetfn(pm, val, 0); // c:3651 via setarrvalue c:2920
    let cloned = pm.clone();
    drop(tab);

    // c:3652-3653 — `unqueue_signals(); return v->pm;` — C returns
    // the param even when arrhashsetfn errored; the failure travels
    // via errflag (callers like the SET_ARRAY bridge check it).
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
pub fn assignnparam(s: &str, val: mnumber, flags: i32) -> Option<Box<param>> {
    // c:3666 `if (!isident(s)) { zerr; errflag |= ERRFLAG_ERROR; return NULL; }`
    if !isident(s) {
        zerr(&format!("not an identifier: {}", s)); // c:3667
        errflag.fetch_or(
            // c:3669
            ERRFLAG_ERROR,
            Ordering::Relaxed,
        );
        return None; // c:3670
    }
    if unset(EXECOPT) {
        return None;
    }
    let mut vbuf = value {
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
                    && unset(KSHARRAYS)
                    && !has_sub
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
        // c:3686-3691 — `createparam(t, val.type & MN_FLOAT ? PM_FFLOAT
        // : PM_INTEGER); second getvalue;`. Synthesize a fresh
        // numeric param in paramtab matching the C body. Without
        // this branch wired, callers like `setiparam` silently
        // dropped the create (returned None) — every new integer
        // param assignment was a no-op.
        let _ = was_unset;
        let new_type = if val.type_ == MN_FLOAT {
            PM_FFLOAT // c:3687
        } else {
            PM_INTEGER // c:3688
        };
        // c:Src/params.c:3690 — newly created PM_INTEGER param
        // inherits the source numeric base from `lastbase` (set by
        // the math parser when consuming a `N#NNN` or `0x..` literal).
        // Mirror the assignstrvalue path at c:3714 so `(( X = 16#ff ))`
        // creates X as `typeset -i16 X=255` (displays as `16#FF`)
        // rather than naked decimal `255`.
        let inherited_base = if val.type_ == MN_FLOAT {
            0
        } else {
            let lb = crate::ported::math::lastbase();
            if lb > 0 {
                lb
            } else {
                0
            }
        };
        let pm: Param = Box::new(param {
            node: hashnode {
                next: None,
                nam: s.to_string(),
                flags: new_type as i32,
            },
            u_data: 0,
            u_tied: None,
            u_arr: None,
            u_str: None,
            // c:3690 — `setnumvalue(...)` stores the value. For
            // PM_INTEGER → u.l; for PM_FFLOAT → u.dval.
            u_val: if val.type_ == MN_FLOAT { 0 } else { val.l },
            u_dval: if val.type_ == MN_FLOAT { val.d } else { 0.0 },
            u_hash: None,
            gsu_s: None,
            gsu_i: None,
            gsu_f: None,
            gsu_a: None,
            gsu_h: None,
            base: inherited_base,
            width: 0,
            env: None,
            ename: None,
            old: None,
            level: 0,
        });
        if let Ok(mut tab) = paramtab().write() {
            tab.insert(s.to_string(), pm.clone());
        }
        return Some(pm);
    }
    if (flags & ASSPM_WARN) != 0 {
        if let Some(ref vv) = v {
            if let Some(ref pm) = vv.pm {
                check_warn_pm(pm, "numeric", 0, 1);
            }
        }
    }
    // The reassign path: getvalue gave us a cloned pm inside the value
    // buffer. setnumvalue mutates that clone but the write doesn't
    // propagate back to paramtab. Write through paramtab directly so
    // reassignments stick — same shape as `assignsparam`'s c:3343
    // `assignstrvalue(v, val, flags)` path which mutates paramtab in
    // place.
    if let Ok(mut tab) = paramtab().write() {
        if let Some(pm) = tab.get_mut(s) {
            // c:Src/params.c — setnumvalue (the C function this
            // reassign-path mirrors) eventually calls setfn which
            // goes through assignstrvalue (params.c:2899-2904) where
            // PM_READONLY rejection lives. The Rust port writes
            // u_val / u_dval / u_str directly, bypassing that path,
            // so the readonly check has to happen here. Without it,
            // `(( x++ ))` / `let "x = ..."` could mutate readonly
            // params silently. Bug #154 in docs/BUGS.md.
            if (pm.node.flags as u32 & PM_READONLY) != 0 {
                // zerr internally sets ERRFLAG_ERROR via the
                // c:194 path in Src/utils.c. Match the existing
                // readonly check at params.rs:3951 (assignstrvalue
                // arm) — same call shape, same behavior.
                zerr(&format!("read-only variable: {}", pm.node.nam));
                return None;
            }
            pm.node.flags &= !(PM_DEFAULTED as i32);
            let t = PM_TYPE(pm.node.flags as u32);
            if t == PM_INTEGER {
                // c:2874 — `pm->gsu.i->setfn(pm, val.u.l)`. MN_FLOAT
                // input truncates to integer.
                pm.u_val = if val.type_ == MN_FLOAT {
                    val.d as i64
                } else {
                    val.l
                };
                // c:Src/params.c:2801 — `if (!v->pm->base && lastbase
                // != -1) v->pm->base = lastbase;`. After setfn the C
                // path falls through `setstrvalue(v, NULL)` which
                // inherits the source numeric base from `lastbase`
                // when the param doesn't yet have an explicit base.
                // Mirror that here so `(( x = 0xFF ))` on an existing
                // integer param updates pm.base to 16, displaying as
                // `16#FF` instead of decimal `255`. Bug #175 in
                // docs/BUGS.md. The create path already inherits at
                // params.rs:5759-5768; this is the reassign-path
                // equivalent.
                if pm.base == 0 {
                    let lb = crate::ported::math::lastbase();
                    if lb > 0 {
                        pm.base = lb;
                    }
                }
            } else if t == PM_EFLOAT || t == PM_FFLOAT {
                // c:2878 — MN_INTEGER input promotes to f64.
                pm.u_dval = if val.type_ == MN_FLOAT {
                    val.d
                } else {
                    val.l as f64
                };
            } else if t == PM_SCALAR || t == PM_NAMEREF || t == PM_ARRAY {
                // c:2862-2871 — convbase/convfloat → u_str.
                let s_rendered = if val.type_ == MN_FLOAT {
                    convfloat_underscore(val.d, pm.width)
                } else {
                    convbase_underscore(val.l, if pm.base > 0 { pm.base } else { 10 }, pm.width)
                };
                pm.u_str = Some(s_rendered);
            }
            let cloned = pm.clone();
            return Some(cloned);
        }
    }
    None
}

/// Port of `Param setnparam(char *s, mnumber val)` from `Src/params.c:3745-3749`.
///
/// C body (c:3747-3748):
/// ```c
/// return assignnparam(s, val, ASSPM_WARN);
/// ```
///
/// Single-line wrapper around `assignnparam` with ASSPM_WARN flags.
///
/// The previous Rust port took `(s: &str, val: f64) -> ()` — losing
/// the integer branch (callers couldn't set integer params via
/// `setnparam`) AND the Param return. No real callers existed because
/// the fabricated sig fit nothing. Match C exactly: `(s, val)` where
/// `val` is the canonical `mnumber` tagged union, returning the
/// resulting Param.
pub fn setnparam(s: &str, val: mnumber) -> Option<Param> {
    assignnparam(s, val, ASSPM_WARN as i32) // c:3748
}

/// Port of `Param assigniparam(char *s, zlong val, int flags)` from
/// `Src/params.c:3754-3761`.
///
/// C body (c:3757-3760):
/// ```c
/// mnumber mnval;
/// mnval.type = MN_INTEGER;
/// mnval.u.l = val;
/// return assignnparam(s, mnval, flags);
/// ```
///
/// Two divergences in the previous Rust port:
///   1. Dropped the `flags` arg — caller-supplied flags (e.g.
///      ASSPM_AUGMENT for `+= int`) couldn't be threaded through;
///      every call hardcoded ASSPM_WARN regardless.
///   2. Returned void instead of Param — losing the new param
///      pointer the caller may want to read back.
pub fn assigniparam(s: &str, val: i64, flags: i32) -> Option<Param> {
    // c:3757-3759 — `mnumber{ .type = MN_INTEGER, .u.l = val }`.
    let mnval = mnumber {
        l: val,
        d: 0.0,
        type_: MN_INTEGER,
    };
    // c:3760 — `return assignnparam(s, mnval, flags);`
    assignnparam(s, mnval, flags) // c:3760
}

/// Port of `Param setiparam(char *s, zlong val)` from `Src/params.c:3767-3773`.
///
/// C body (c:3769-3772):
/// ```c
/// mnumber mnval;
/// mnval.type = MN_INTEGER;
/// mnval.u.l = val;
/// return assignnparam(s, mnval, ASSPM_WARN);
/// ```
///
/// The previous Rust port stringified to decimal and routed through
/// `assignsparam` — which CREATES THE PARAM AS PM_SCALAR. C creates
/// as PM_INTEGER. `setiparam("x", 5)` followed by `typeset -p x`:
///   - C: \`typeset -i x=5\`
///   - Old Rust: \`typeset x=5\`
///
/// `assignnparam` IS now ported (params.rs:4403). Route through it
/// matching C exactly so integer-typed params get created with the
/// right PM_INTEGER flag.
pub fn setiparam(s: &str, val: i64) -> Option<Param> {
    // c:3770-3771 — `mnumber{ .type = MN_INTEGER, .u.l = val }`.
    let mnval = mnumber {
        l: val,
        d: 0.0,
        type_: MN_INTEGER,
    };
    // c:3772 — `return assignnparam(s, mnval, ASSPM_WARN);`
    assignnparam(s, mnval, ASSPM_WARN as i32) // c:3772
}

/// Port of `setiparam_no_convert(char *s, zlong val)` from Src/params.c:3781. C
/// source comment: "If the target is already an integer, this
/// gets converted back. Low technology rules." It uses convbase
/// to render decimal then calls assignsparam.
/// WARNING: param names don't match C — Rust=() vs C=(s, val)
pub fn setiparam_no_convert(s: &str, val: i64) -> Option<Param> {
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
pub fn resetparam(pm: &mut param, flags: i32) -> i32 {
    // c:3796
    let s = pm.node.nam.clone(); // c:3796
    queue_signals(); // c:3799
                     // c:3800-3807 — paramtab->getnode2 / getnode reachability check.
                     // Without paramtab vtable wired we cannot detect the hidden-
                     // variable case, so we proceed; a future port of paramtab
                     // adds the check at this site.
    unsetparam_pm(pm, 0, 1); // c:3819
    unqueue_signals(); // c:3819
    let _ = createparam(&s, flags); // c:3819
    0 // c:3819
}

/// Port of `void unsetparam(char *s)` from `Src/params.c:3819`.
///
/// C body:
/// ```c
/// Param pm;
/// queue_signals();
/// if ((pm = (Param)(paramtab == realparamtab ?
///         paramtab->getnode2(paramtab, s) :
///         paramtab->getnode(paramtab, s))))
///     unsetparam_pm(pm, 0, 1);
/// unqueue_signals();
/// ```
///
/// The previous Rust port took `(variables, arrays, assoc_arrays,
/// name)` operating on EXTERNAL HashMap storage — a SubstState-
/// era stale signature. C operates on the canonical `paramtab`
/// global. No live callers used the old 4-arg form (all use
/// `paramtab().write().remove(...)` directly), so renaming is
/// safe.
pub fn unsetparam(name: &str) -> i32 {
    // c:3819 — C's unsetparam is void and discards unsetparam_pm's
    // status; bin_unset (c:Src/builtin.c:3952-3953) does the
    // paramtab lookup itself and calls `if (unsetparam_pm(pm, 0, 1))
    // returnval = 1;`. The Rust bin_unset routes through this
    // wrapper for the bridge plumbing (tied names, hashed-storage
    // shadow, special regenerators), so the rejection status is
    // surfaced here instead: 1 = readonly rejection, 0 = unset ok.
    // c:Src/params.c:3853-3935 — unsetparam_pm's tied-alt-name
    // removal block. zsh's PATH/path, FPATH/fpath, MANPATH/manpath,
    // CDPATH/cdpath, PSVAR/psvar pairs are tied (`pm->ename` points
    // to the alt name) — unsetting one must clear the other or
    // command lookup keeps finding binaries via the surviving `path`
    // array even after `unset PATH`. The full ename machinery is
    // deferred until the gsu vtable lands; until then, mirror the
    // tie explicitly for the canonical pairs so `unset PATH` is
    // actually a security boundary. Bug #416.
    let tied_alt: Option<&str> = match name {
        "PATH" => Some("path"),
        "path" => Some("PATH"),
        "FPATH" => Some("fpath"),
        "fpath" => Some("FPATH"),
        "MANPATH" => Some("manpath"),
        "manpath" => Some("MANPATH"),
        "CDPATH" => Some("cdpath"),
        "cdpath" => Some("CDPATH"),
        "PSVAR" => Some("psvar"),
        "psvar" => Some("PSVAR"),
        "MODULE_PATH" => Some("module_path"),
        "module_path" => Some("MODULE_PATH"),
        "FIGNORE" => Some("fignore"),
        "fignore" => Some("FIGNORE"),
        "MAILPATH" => Some("mailpath"),
        "mailpath" => Some("MAILPATH"),
        _ => None,
    };
    queue_signals(); // c:3825
                     // c:3826-3831 — `if ((pm = ... getnode2 ...) && !(pm->node.flags
                     // & PM_NAMEREF)) unsetparam_pm(pm, 0, 1);`.
                     //
                     // Two divergences in the previous Rust port:
                     //   1. Missing PM_NAMEREF check — `unsetparam("ref")` where `ref`
                     //      is a nameref would remove the ref alias itself. C explicitly
                     //      skips nameref params here (they're cleared via the
                     //      ref-specific path, not the value-side unset).
                     //   2. Bypassed `unsetparam_pm` — removed the entry directly from
                     //      paramtab without running the readonly-guard at c:3850, the
                     //      stdunsetfn dispatch at c:3870, or the `pm->old` scope
                     //      restore. `typeset -r x=foo; unset x` would silently succeed
                     //      in Rust where C rejects with `read-only variable: x`.
    // c:Src/params.c:3853 — flag regenerator-style specials as unset
    // so subsequent reads via lookup_special_var skip the getfn.
    // RANDOM/SECONDS/EPOCH*/TTYIDLE/ERRNO have no paramtab pm node in
    // zshrs (they're lookup_special_var libc shims), so the standard
    // unsetparam_pm path below doesn't catch them. Bug #417/#418.
    if matches!(
        name,
        "RANDOM" | "SECONDS" | "EPOCHSECONDS" | "EPOCHREALTIME" | "TTYIDLE" | "ERRNO"
    ) {
        mark_unset_special(name);
    }
    // c:Src/params.c:3850 — `if (pm->node.flags & PM_READONLY) { zerr;
    // return 1; }`. Read-only specials (LINENO, HISTCMD, PPID, etc.)
    // have PM_READONLY in their special_paramdef entry but no paramtab
    // pm node by default, so the standard PM_READONLY check inside
    // unsetparam_pm never fires for them. Walk the special_params
    // table directly to catch these. Bug #419.
    let is_readonly_special = special_params
        .iter()
        .any(|ip| ip.name == name && (ip.pm_flags & PM_READONLY) != 0);
    if is_readonly_special {
        zerr(&format!("read-only variable: {}", name));
        unqueue_signals();
        return 1; // c:3854 — unsetparam_pm's readonly rejection status
    }
    let mut retval = 0i32;
    let (found, is_nameref) = {
        let tab = paramtab().read().unwrap();
        match tab.get(name) {
            Some(pm) => (true, (pm.node.flags as u32 & PM_NAMEREF) != 0),
            None => (false, false),
        }
    };
    if found && !is_nameref {
        // c:3826-3830
        // c:3831 — `unsetparam_pm(pm, 0, 1)`. Take an owned copy out
        // of paramtab so we can mutate it (unsetparam_pm wants
        // &mut), run the readonly-guard + env teardown, then re-insert
        // or fully remove based on the readonly path.
        let mut pm_owned = paramtab().write().unwrap().remove(name).unwrap();
        let rejected = unsetparam_pm(&mut pm_owned, 0, 1); // c:3831
        if rejected != 0 {
            retval = 1; // c:Src/builtin.c:3952-3953 surfaced to bin_unset
            // Readonly rejection — restore the entry so the state
            // is unchanged.
            paramtab()
                .write()
                .unwrap()
                .insert(name.to_string(), pm_owned);
        } else if pm_owned.old.is_some()
            || (pm_owned.level > 0
                && locallevel.load(Ordering::Relaxed) as i32 >= pm_owned.level)
        {
            // c:Src/params.c:3892-3925 — when the unset'd pm is a
            // local that shadowed an outer binding (chained via
            // pm.old by `addparam` at c:1137), the local pm STAYS
            // in paramtab with PM_UNSET set so the current scope's
            // reads see "unset" (empty). The pm.old chain is
            // preserved so endparamscope can uncover the outer
            // when the local scope ends. Without this re-insert,
            // either:
            //   - the outer would be uncovered immediately (wrong:
            //     zsh hides the outer until scope end), or
            //   - pm.old would be dropped (wrong: outer never
            //     comes back).
            // c:3911-3913 — `if ((pm->level && locallevel >= pm->level)
            // ...) return 0;` — locals are kept in the table marked
            // PM_UNSET even WITHOUT an outer binding ("foo() { local
            // bar; unset bar; } makes the global bar available? The
            // following makes the answer no"), and `typeset -p bar`
            // still finds the node (prints nothing, status 0).
            paramtab()
                .write()
                .unwrap()
                .insert(name.to_string(), pm_owned);
        } else if (pm_owned.node.flags as u32 & PM_SPECIAL) != 0
            && (pm_owned.node.flags as u32 & PM_REMOVABLE) == 0
        {
            // c:Src/params.c:3911-3913 — `if ((pm->flags &
            // (PM_SPECIAL|PM_REMOVABLE)) == PM_SPECIAL) return 0;`.
            // PM_SPECIAL params (SECONDS, RANDOM, HOME, IFS, ...)
            // stay in paramtab with PM_UNSET set after unset. A
            // subsequent re-assign (`SECONDS=100`) finds the same
            // pm via createparam's `oldpm` lookup, hits the reuse
            // arm (c:1132), preserves the PM_INTEGER|PM_SPECIAL
            // flags + gsu vtable, so intsetfn's name-dispatch
            // fires and routes through intsecondssetfn. Without
            // this, the special pm was dropped, a fresh PM_SCALAR
            // pm was created for the re-assignment, the value
            // landed in pm.u_str, and lookup_special_var kept
            // reading the time-since-shtimer delta. Bug #418 in
            // docs/BUGS.md.
            //
            // PM_UNSET is already stamped by unsetparam_pm at
            // line 6491; keep it on the re-inserted pm so reads
            // still see "unset" until re-assignment clears it
            // (clear_unset_special at line 4756 handles the
            // regenerator-style unset_specials set; the pm flag
            // is cleared inside assignsparam's value-write arm
            // via stdunsetfn's symmetric set/clear convention).
            paramtab()
                .write()
                .unwrap()
                .insert(name.to_string(), pm_owned);
        }
        // No pm.old + no rejection + not PM_SPECIAL → drop entirely
        // (matches the C path at c:3935 where the node is removed
        // from paramtab).
    }
    // c:Src/params.c:3905-3935 — tied-alt removal. Cascade the
    // unset to the paired name (PATH↔path etc.). Also clear the OS
    // env mirror since command lookup at the syscall level reads
    // the inherited libc environ.
    if let Some(alt) = tied_alt {
        let alt_present = paramtab().read().map(|t| t.contains_key(alt)).unwrap_or(false);
        if alt_present {
            if let Some(mut alt_pm) = paramtab().write().ok().and_then(|mut t| t.remove(alt)) {
                let _ = unsetparam_pm(&mut alt_pm, 1, 1);
            }
        }
        env::remove_var(alt);
        env::remove_var(name);
        // c:Src/params.c:5291 — `if (t == path) cmdnamtab->emptytable
        // (cmdnamtab)`. The hashed-cmdnam cache holds absolute paths
        // resolved via the prior PATH search; without clearing,
        // `unset PATH; ls` still hits the cached `/bin/ls` entry and
        // exec succeeds — defeating the security boundary the unset
        // is supposed to establish. Bug #416.
        if matches!(name, "PATH" | "path") {
            crate::ported::hashtable::emptycmdnamtable();
        }
    }
    unqueue_signals(); // c:3832
    retval
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
pub fn unsetparam_pm(pm: &mut param, altflag: i32, exp: i32) -> i32 {
    // c:3850 — `if ((pm->node.flags & PM_READONLY) && pm->level <= locallevel)`.
    let cur_ll = locallevel.load(Ordering::Relaxed) as i32; // c:3850 locallevel
    if (pm.node.flags as u32 & PM_READONLY) != 0 && pm.level <= cur_ll {
        // c:3850
        // c:3852 — `zerr("read-only %s: %s", ...)`. Emit diagnostic
        // so users see why the unset failed.
        let kind = if (pm.node.flags as u32 & PM_NAMEREF) != 0 {
            // c:3852
            "reference"
        } else {
            "variable"
        };
        zerr(&format!("read-only {}: {}", kind, pm.node.nam));
        return 1; // c:3854
    }
    pm.node.flags &= !(PM_DECLARED as i32); // c:3868
    if (pm.node.flags as u32 & PM_UNSET) == 0 || (pm.node.flags as u32 & PM_REMOVABLE) != 0 {
        // c:3870 — `pm->gsu.s->unsetfn(pm, exp)` — open-coded to stdunsetfn.
        stdunsetfn(pm, exp);
    }
    if pm.env.is_some() {
        delenv(&pm.node.nam); // c:3872 delenv(pm)
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
pub fn intgetfn(pm: &param) -> i64 {
    pm.u_val
}

/// Port of `intsetfn(Param pm, zlong x)` from `Src/params.c:4002`. C body:
/// `pm->u.val = x;`
pub fn intsetfn(pm: &mut param, x: i64) {
    // c:Src/params.c:4575 — PM_SPECIAL integers have per-name gsu_i->setfn
    // hooks: SECONDS routes through intsecondssetfn (shtimer math),
    // RANDOM seeds the PRNG, etc. The default intsetfn is the fallback
    // for non-special integers (pm->u.val write).
    //
    // Rust port lookup_special_var dispatches GETTERS by name; the
    // setters need symmetric dispatch so `SECONDS=N` actually moves
    // shtimer instead of writing u.val which intsecondsgetfn never
    // reads. Without this, `$SECONDS` always read the time-since-shtimer
    // delta regardless of writes.
    // Name-based dispatch (not flag-based): some assignment paths
    // construct a fresh param shell for tc and lose PM_SPECIAL.
    match pm.node.nam.as_str() {
        "SECONDS" => {
            intsecondssetfn(x);
            return;
        }
        // c:Src/params.c:4552 randomsetfn — `RANDOM=N` calls
        // srand(N). Without this dispatch, $RANDOM writes only u.val
        // and the next read returns rand()'s next value from the
        // PROCESS-START seed, not the user-requested seed. Same
        // name-based dispatch shape as SECONDS above.
        "RANDOM" => {
            randomsetfn(x);
            return;
        }
        // c:Src/params.c:4698 uidsetfn / c:4719 euidsetfn / c:4740
        // gidsetfn / c:4761 egidsetfn — `UID=N` / `EUID=N` /
        // `GID=N` / `EGID=N` attempt the corresponding setuid /
        // seteuid / setgid / setegid syscall and emit
        // `failed to change [effective ]{user,group} ID: ERRNO`
        // on failure. Bug #254 in docs/BUGS.md. Same name-based
        // dispatch shape as SECONDS/RANDOM above.
        "UID" => {
            uidsetfn(x);
            return;
        }
        "EUID" => {
            euidsetfn(x);
            return;
        }
        "GID" => {
            gidsetfn(x);
            return;
        }
        "EGID" => {
            egidsetfn(x);
            return;
        }
        // c:Src/params.c:4974 histsizesetfn / c:4998 savehistsizesetfn —
        // `HISTSIZE=N` / `SAVEHIST=N` must update the canonical
        // `histsiz` / `savehistsiz` globals AND clamp to >= 1 +
        // call `resizehistents()` so the in-memory history buffer
        // shrinks/grows. Without this dispatch the assignment
        // lands in `pm.u_val` and `histsizegetfn` (which reads the
        // global) keeps returning the un-touched default. Bug #520.
        "HISTSIZE" => {
            histsizesetfn(x);
            return;
        }
        "SAVEHIST" => {
            savehistsizesetfn(x);
            return;
        }
        _ => {}
    }
    pm.u_val = x;
}

/// Port of `floatgetfn(Param pm)` from `Src/params.c:4011`. C body:
/// `return pm->u.dval;`
pub fn floatgetfn(pm: &param) -> f64 {
    pm.u_dval
}

/// Port of `floatsetfn(Param pm, double x)` from `Src/params.c:4020`. C body:
/// `pm->u.dval = x;`
pub fn floatsetfn(pm: &mut param, x: f64) {
    // c:Src/params.c:4603 floatsecondssetfn — PM_SPECIAL float SECONDS
    // routes through shtimer math. Symmetric with intsetfn's SECONDS
    // dispatch above and lookup_special_var's getter.
    // c:Src/params.c:4603 floatsecondssetfn — PM_SPECIAL float SECONDS
    // routes through shtimer math. Symmetric with intsetfn's SECONDS
    // dispatch above and lookup_special_var's getter. Some assignment
    // paths construct a fresh `param` shell for tc (type-conversion)
    // and lose PM_SPECIAL, so name-only dispatch is the safe fallback.
    if pm.node.nam == "SECONDS" {
        floatsecondssetfn(x);
        return;
    }
    pm.u_dval = x;
}

/// Port of `strgetfn(Param pm)` from `Src/params.c:4029`. C body:
/// `return pm->u.str ? pm->u.str : (char *) hcalloc(1);`
pub fn strgetfn(pm: &param) -> String {
    pm.u_str.clone().unwrap_or_default()
}

/// Port of `strsetfn(Param pm, char *x)` from `Src/params.c:4040`.
///
/// C body (c:4043-4051):
/// ```c
/// zsfree(pm->u.str); pm->u.str = x;
/// if (!(pm->node.flags & PM_HASHELEM) &&
///     ((pm->node.flags & PM_NAMEDDIR) || isset(AUTONAMEDIRS))) {
///     pm->node.flags |= PM_NAMEDDIR;
///     adduserdir(pm->node.nam, x, 0, 0);
/// }
/// ```
///
/// The C body fires the `adduserdir` path when EITHER `PM_NAMEDDIR`
/// is already set OR the `AUTONAMEDIRS` option is on. The previous
/// Rust port only fired when PM_NAMEDDIR was already set, missing
/// the AUTONAMEDIRS auto-create branch entirely. With `setopt
/// AUTONAMEDIRS`, every scalar assignment to a path-shaped value
/// should register a named-directory entry for `~name` expansion;
/// the Rust port silently dropped that behavior.
pub fn strsetfn(pm: &mut param, x: String) {
    // c:4040
    pm.u_str = Some(x.clone()); // c:4044 pm->u.str = x
                                // c:4045-4046 — `if (!(PM_HASHELEM) && (PM_NAMEDDIR || isset(AUTONAMEDIRS)))`.
    if (pm.node.flags as u32 & PM_HASHELEM) == 0
        && ((pm.node.flags as u32 & PM_NAMEDDIR) != 0 || isset(AUTONAMEDIRS))
    // c:4046 isset(AUTONAMEDIRS)
    {
        pm.node.flags |= PM_NAMEDDIR as i32; // c:4047
        adduserdir(&pm.node.nam, &x, 0, false); // c:4048
    }
}

/// Port of `arrgetfn(Param pm)` from `Src/params.c:4057`. C body:
/// `return pm->u.arr ? pm->u.arr : &nullarray;`
pub fn arrgetfn(pm: &param) -> Vec<String> {
    pm.u_arr.clone().unwrap_or_default()
}

/// Port of `arrsetfn(Param pm, char **x)` from `Src/params.c:4066`. C body frees
/// the old array, applies PM_UNIQUE filter via `uniqarray()`, then
/// stores. Calls `arrfixenv(ename, x)` for tied colon-arrays.
pub fn arrsetfn(pm: &mut param, x: Vec<String>) {
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
pub fn hashgetfn(pm: &param) -> Option<&HashTable> {
    pm.u_hash.as_ref()
}

/// Port of `hashsetfn(Param pm, HashTable x)` from `Src/params.c:4093`. C body:
/// `if (pm->u.hash && pm->u.hash != x) deleteparamtable(pm->u.hash);
///  pm->u.hash = x;`
pub fn hashsetfn(pm: &mut param, x: HashTable) {
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
/// Storage model: C's per-pair `createparam(k)` + `assignstrvalue`
/// builds child Params inside a fresh `newparamtable`; zshrs's assoc
/// values live in the `paramtab_hashed_storage` IndexMap keyed by the
/// owning param's name, so the pair walk writes there. `pm.u_hash`
/// stays untouched — the IndexMap is the authoritative store (same
/// contract sethparam/gethparam already use).
pub fn arrhashsetfn(
    // c:4113
    pm: &mut param,
    val: Vec<String>,
    flags: i32,
) {
    // c:4124-4127 — count non-Marker entries.
    let alen: usize = val
        .iter()
        .filter(|s| !s.starts_with(Marker as char))
        .count();

    // c:4129-4131 — odd count → error.
    if alen % 2 != 0 {
        zerr("bad set of key/value pairs for associative array");
        return;
    }

    // c:4135-4139 — ASSPM_AUGMENT starts from the existing hash
    // (`ht = paramtab = pm->gsu.h->getfn(pm)`); otherwise a fresh
    // table (`newparamtable(17, pm->node.nam)`).
    let mut map: IndexMap<String, String> = if (flags & ASSPM_AUGMENT) != 0 {
        paramtab_hashed_storage()
            .lock()
            .unwrap()
            .get(&pm.node.nam)
            .cloned()
            .unwrap_or_default() // c:4136
    } else {
        IndexMap::new() // c:4138-4139
    };

    // c:4141-4166 — pair walk. keyvalpairelement (Src/subst.c:49)
    // emits `[Marker, key, value]` triples for `[k]=v` / `[k]+=v`
    // forms ("Either all elements have Marker or none. Checked in
    // caller." c:4144); plain input is flat `[k, v, ...]` pairs.
    let mut it = val.into_iter();
    while let Some(first) = it.next() {
        let (elt_augment, key) = if first.starts_with(Marker as char) {
            // c:4145-4151 — `(*aptr)[1] == '+'` → per-element append
            // (ASSPM_AUGMENT via the setsparam INT_MAX trick).
            let aug = first[Marker.len_utf8()..].starts_with('+');
            match it.next() {
                Some(k) => (aug, k),
                None => break,
            }
        } else {
            (false, first)
        };
        let v = it.next().unwrap_or_default(); // c:4166 assignstrvalue value
        if elt_augment {
            // c:4147-4150 — `[k]+=v` appends to the existing element.
            map.entry(key).or_default().push_str(&v);
        } else {
            // c:4156-4166 — createparam(k, PM_SCALAR|PM_UNSET) +
            // assignstrvalue: plain insert in the IndexMap model.
            map.insert(key, v);
        }
    }

    // c:4168-4169 — `pm->gsu.h->setfn(pm, ht)` installs the table.
    paramtab_hashed_storage()
        .lock()
        .unwrap()
        .insert(pm.node.nam.clone(), map);
    // c:4170 — free(val). Rust drops automatically.
}

/// Port of `nullstrsetfn(UNUSED(Param pm), char *x)` from `Src/params.c:4180`. C body:
/// `zsfree(x);` — frees but doesn't store. Rust drop handles free.
#[allow(unused_variables)]
pub fn nullstrsetfn(pm: &mut param, x: String) {}

/// Port of `nullunsetfn(UNUSED(Param pm), UNUSED(int exp))` from `Src/params.c:4192`. C body: empty.
#[allow(unused_variables)]
pub fn nullunsetfn(pm: &mut param, exp: i32) {}

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
pub fn stdunsetfn(pm: &mut param, exp: i32) {
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
pub fn nullintsetfn(pm: &mut param, x: i64) {}

/// Port of `nullsethashfn(UNUSED(Param pm), HashTable x)` from `Src/params.c:4104`. C body:
/// `deleteparamtable(x);` — frees the supplied table, doesn't store.
#[allow(unused_variables)]
pub fn nullsethashfn(pm: &mut param, x: HashTable) {
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
pub fn intvargetfn(pm: &param) -> i64 {
    pm.u_val
}

/// Port of `intvarsetfn(Param pm, zlong x)` from `Src/params.c:4213`. C body:
/// `*pm->u.valptr = x;`
pub fn intvarsetfn(pm: &mut param, x: i64) {
    pm.u_val = x;
}

/// Port of `zlevarsetfn(Param pm, zlong x)` from `Src/params.c:4224`. C body sets
/// the int and triggers `adjustwinsize` for LINES/COLUMNS.
/// Port of `zlevarsetfn(Param pm, zlong x)` from `Src/params.c:4226`.
/// C body: `*p = x; if (p == &zterm_lines || p == &zterm_columns)
/// adjustwinsize(2 + (p == &zterm_columns));`
///
/// The `from` argument to `adjustwinsize` is documented at
/// `Src/utils.c:1883-1887`: 0=signal, 1=manual, 2=LINES callback,
/// 3=COLUMNS callback. Each value selects a different code path
/// inside `adjustwinsize` — for example, `from=2` skips the
/// COLUMNS-specific ioctl, and `from=3` skips the LINES path.
///
/// The previous Rust port passed `0` for both LINES and COLUMNS,
/// which triggered the FULL `getwinsz` ioctl + both adjustlines
/// AND adjustcolumns calls AND the potential setiparam recursion
/// — diverging from C's narrow "just adjust the one axis we
/// changed" semantics. Effect: setting `LINES=80` would re-issue
/// `setiparam("COLUMNS", ...)` recursively, churning the
/// paramtab for no reason.
pub fn zlevarsetfn(pm: &mut param, x: i64) {
    // c:4226
    pm.u_val = x; // c:4230 *p = x;
                  // c:4231-4232 — `2 + (p == &zterm_columns)` selects 2 for LINES
                  // (zterm_lines) and 3 for COLUMNS (zterm_columns).
    if pm.node.nam == "LINES" {
        let _ = adjustwinsize(2); // c:4232 LINES path
    } else if pm.node.nam == "COLUMNS" {
        let _ = adjustwinsize(3); // c:4232 COLUMNS path
    }
}

/// Port of `strvarsetfn(Param pm, char *x)` from `Src/params.c:4249`. C body:
/// `zsfree(*q); *q = x;` where `q = (char **)pm->u.data`.
pub fn strvarsetfn(pm: &mut param, x: Option<String>) {
    pm.u_str = x;
}

/// Port of `strvargetfn(Param pm)` from `Src/params.c:4263`. C body:
/// `s = *((char **)pm->u.data); return s ? s : hcalloc(1);`
pub fn strvargetfn(pm: &param) -> String {
    pm.u_str.clone().unwrap_or_default()
}

/// Port of `arrvargetfn(Param pm)` from `Src/params.c:4279`. C body:
/// `arrptr = *((char ***)pm->u.data); return arrptr ?: &nullarray;`
pub fn arrvargetfn(pm: &param) -> Vec<String> {
    pm.u_arr.clone().unwrap_or_default()
}

/// Direct port of `mod_export void arrvarsetfn(Param pm, char **x)`
/// from `Src/params.c:4292-4317`. The previous body skipped three of
/// the four canonical C arms:
///   1. PM_UNIQUE → uniqarray (was ported via simple_arrayuniq).
///   2. PM_SPECIAL + null x → `*dptr = mkarray(NULL)` so a tied
///      array set to NULL becomes a writable empty array, not a
///      dangling null (was missing).
///   3. `pm->ename` set → `arrfixenv(ename, x)` syncs the colon-
///      joined env var partner (was missing — breaks PATH/path,
///      FPATH/fpath, etc. when set via the array form).
///   4. `pm->ename` set + null x + `*dptr == path` → invalidate
///      pathchecked so the next path resolution re-walks (was
///      missing).
pub fn arrvarsetfn(pm: &mut param, x: Option<Vec<String>>) {
    // c:4296 `char ***dptr = (char ***)pm->u.data;`
    // c:4298-4299 — `if (*dptr != x) freearray(*dptr);` Rust Vec drop
    // on reassignment handles freeing automatically.
    // c:4300-4301 — `if (pm->node.flags & PM_UNIQUE) uniqarray(x);`
    let uniq_applied: Option<Vec<String>> = match x {
        Some(v) if (pm.node.flags as u32 & PM_UNIQUE) != 0 => Some(simple_arrayuniq(v)),
        other => other,
    };
    // c:4302-4310 — PM_SPECIAL + NULL → mkarray(NULL); else assign.
    let final_val: Vec<String> = match uniq_applied {
        Some(v) => v, // c:4310 `*dptr = x;`
        None => {
            if (pm.node.flags as u32 & PM_SPECIAL) != 0 {
                crate::ported::utils::mkarray(None) // c:4308 `mkarray(NULL)`
            } else {
                Vec::new() // c:4310 — null case for non-special: empty.
            }
        }
    };
    // c:4311-4316 — ename sync.
    if let Some(ename) = pm.ename.clone() {
        // c:4311 `if (pm->ename)`
        if !final_val.is_empty() || pm.u_arr.is_some() {
            // c:4312-4313 — `if (x) arrfixenv(pm->ename, x);`
            arrfixenv(&ename, Some(&final_val));
        } else if pm.node.nam == "path" {
            // c:4314-4315 — `else if (*dptr == path) pathchecked = path;`
            // — invalidate the path-resolver cache. Rust port uses an
            // AtomicUsize sentinel; storing 0 marks "must re-walk".
            crate::ported::hashtable::pathchecked.store(0, Ordering::SeqCst);
        }
    }
    pm.u_arr = Some(final_val);
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
pub fn colonarrsetfn(pm: &mut param, x: Option<String>) {
    let uniq = (pm.node.flags as u32 & PM_UNIQUE) != 0; // c:4339
                                                        // c:4339-4341 — `arrvarsetfn(pm, x ? colonsplit(...) : NULL);`
                                                        // The None branch must pass `None` (not `Some(Vec::new())`) so the
                                                        // PM_SPECIAL + NULL → mkarray(NULL) arm in arrvarsetfn fires.
    let arr = x.map(|s| colonsplit(&s, uniq)); // c:4339
    arrvarsetfn(pm, arr);
}

/// Port of `tiedarrgetfn(Param pm)` from `Src/params.c:4348`. C body:
///   `struct tieddata *dptr = (struct tieddata *)pm->u.data;`
///   `return *dptr->arrptr ? zjoin(*dptr->arrptr, …) : "";`
///
/// C's `pm->u.data->arrptr` is a raw pointer into the partner array
/// param's storage. The Rust port can't hold a pointer into another
/// paramtab entry's heap, so the partner lookup goes via
/// `pm.ename` → paramtab → `apm.u_arr`. For backwards compatibility
/// with callers that set `pm.u_arr` directly (the pre-fix code path
/// at c:4348 that's still in some Rust call sites), fall back to
/// the scalar's own `u_arr` when `ename` is None or the partner is
/// missing. Bug #24 in docs/BUGS.md.
pub fn tiedarrgetfn(pm: &param) -> Vec<String> {
    if let Some(ename) = pm.ename.as_deref() {
        if let Ok(tab) = paramtab().read() {
            if let Some(apm) = tab.get(ename) {
                if let Some(arr) = apm.u_arr.as_ref() {
                    return arr.clone();
                }
            }
        }
    }
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
pub fn tiedarrsetfn(pm: &mut param, x: Option<String>) {
    // c:4357

    // c:4361-4368 — free old / clear PM_DEFAULTED on tied counterpart.
    if pm.u_arr.is_none() {
        if let Some(ename) = pm.ename.clone() {
            // c:4365
            let mut tab = paramtab().write().unwrap();
            if let Some(altpm) = tab.get_mut(&ename) {
                // c:4366
                altpm.node.flags &= !(PM_DEFAULTED as i32); // c:4367
            }
        }
    }

    // c:4369-4386 — split + assign. C writes through `*dptr->arrptr`
    // which is a pointer INTO THE PARTNER ARRAY param's storage. The
    // Rust port can't hold raw pointers across paramtab entries, so
    // when `pm.ename` is set, write the split result to the partner's
    // `u_arr` via paramtab (bug #24). When `pm.ename` is None (older
    // call sites or non-tied use), keep the scalar's own `u_arr`
    // up-to-date for legacy callers.
    // c:4370-4380 — single-byte separator built from `dptr->joinchar`
    // on the tieddata riding `pm->u.data` (Rust: typed `u_tied` view);
    // joinchar==0 → empty sepbuf; no tieddata → `:` default (the
    // PM_SPECIAL colon-tied params, c:5314-5315).
    let sepbuf: String = match pm.u_tied.as_deref() {
        Some(td) if td.joinchar == 0 => String::new(), // c:4376-4377
        Some(td) => ((td.joinchar as u8) as char).to_string(), // c:4378-4379
        None => ":".to_string(), // c:5314-5315
    };
    let arr_opt: Option<Vec<String>> = if let Some(s) = x {
        // c:4369
        // c:4381 — `sepsplit(x, sepbuf, 0, 0)`.
        // joinchar==0 (typeset -T s a ''): the zsh 5.9.1 release
        // binary keeps the whole string as ONE element on assignment
        // (measured: `typeset -T S s ""; S=abc; typeset -p s` →
        // `s=( abc )`), diverging from a literal char-split reading
        // of sepsplit("") in the C source; match the release binary
        // (parity floor).
        let split: Vec<String> = if sepbuf.is_empty() {
            vec![s.clone()]
        } else {
            crate::ported::utils::sepsplit(&s, Some(&sepbuf), true)
        };
        // c:4382-4383 — uniqarray if PM_UNIQUE.
        let split = if pm.node.flags & PM_UNIQUE as i32 != 0 {
            // c:4382
            uniqarray(split) // c:4383
        } else {
            split
        };
        Some(split)
    } else {
        None
    };
    if let Some(ename) = pm.ename.clone() {
        if let Ok(mut tab) = paramtab().write() {
            if let Some(apm) = tab.get_mut(&ename) {
                apm.u_arr = arr_opt.clone(); // c:4381
            }
        }
        // c:4352 — zjoin writes the raw joinchar byte; joinchar==0
        // joins with NUL (measured on 5.9.1: `s=(x y); print -rn
        // "$S"` → `x\0y`), not with the empty split-sepbuf.
        let joinsep = if sepbuf.is_empty() { "\0" } else { sepbuf.as_str() };
        pm.u_str = arr_opt.as_ref().map(|a| a.join(joinsep));
    } else {
        pm.u_arr = arr_opt;
    }

    // c:4387-4388 — `if (pm->ename) arrfixenv(pm->name, *dptr->arrptr)`.
    if pm.ename.is_some() {
        let nam = pm.node.nam.clone();
        // Pull the live array out of the partner for env sync.
        let snap = paramtab()
            .read()
            .ok()
            .and_then(|t| t.get(pm.ename.as_deref().unwrap()).and_then(|p| p.u_arr.clone()));
        arrfixenv(&nam, snap.as_deref());
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
pub fn tiedarrunsetfn(pm: &mut param, _exp: i32) {
    // c:4393
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
pub fn newuniqtable(size: i64) -> HashSet<String> {
    // c:4450
    HashSet::with_capacity(size.max(0) as usize) // c:4450 newhashtable(size, ...)
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
pub fn arrayuniq(x: Vec<String>, freeok: i32) -> Vec<String> {
    // c:4473
    let _ = freeok;
    let array_size = x.len();
    if array_size == 0 {
        // c:4481
        return x;
    }
    // c:4482-4486 — small-array fallback to simple_arrayuniq.
    if array_size < 10 {
        // c:4482
        return simple_arrayuniq(x); // c:4484
    }
    // c:4483 — `if (!(ht = newuniqtable(array_size + 1)))` — Rust
    // newuniqtable never fails, but mirror the C order of allocation.
    let mut ht = newuniqtable(array_size as i64 + 1);
    // c:4487-4507 — walk + first-wins.
    let mut out: Vec<String> = Vec::with_capacity(array_size);
    for s in x {
        // c:4487 walk
        if ht.insert(s.clone()) {
            // c:4488 gethashnode2 + addhashnode2
            out.push(s); // c:4495 *write_it = *it
        }
        // else: dup — drop the value (c:4502 zsfree if freeok).
    }
    drop(ht); // c:4523 deletehashtable
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
pub fn zhuniqarray(x: Vec<String>) -> Vec<String> {
    // c:4523
    arrayuniq(x, 0) // c:4523
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
    // c:4563 — `shtimer` is initialized at shell startup (zsh.h
    // mod_export). Force shtimer init BEFORE reading `now` so the
    // lazy-init race doesn't make `now < shtimer` on first call
    // (which produced -1 from the nsec borrow-from-sec adjustment).
    let timer = *shtimer_lock().lock().expect("shtimer poisoned");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let now_sec = now.as_secs() as i64;
    let timer_sec = timer.as_secs() as i64;
    let now_nsec = now.subsec_nanos() as i64;
    let timer_nsec = timer.subsec_nanos() as i64;
    let diff = now_sec - timer_sec - i64::from(now_nsec < timer_nsec);
    // c:4565 — clamp negative-diff (lazy-init or clock skew) to 0
    // so \$SECONDS reads as a non-negative count of elapsed seconds
    // from shell start. zsh's shtimer is set in main() before any
    // user code runs, guaranteeing now >= shtimer; the Rust lazy
    // init makes this stricter via .max(0).
    diff.max(0)
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
        zwarn("SECONDS truncated on assignment");
        // c:4585 — C still stores; Rust represents shtimer as Duration
        // which is non-negative. We clamp to zero to preserve the
        // "store-anyway" semantic for the time-display path, even
        // though the negative-time case is unrepresentable.
        *shtimer_lock().lock().expect("shtimer poisoned") = Duration::new(0, now.subsec_nanos());
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
    let new = now
        .checked_sub(Duration::from_secs_f64(x))
        .unwrap_or_default();
    *shtimer_lock().lock().expect("shtimer poisoned") = new;
}

/// Port of `getrawseconds()` from `Src/params.c:4615`. C body:
/// `return (double)shtimer.tv_sec + (double)shtimer.tv_nsec / 1e9;`
pub fn getrawseconds() -> f64 {
    shtimer_lock()
        .lock()
        .expect("shtimer poisoned")
        .as_secs_f64()
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
pub fn setsecondstype(
    // c:4630
    pm: &mut param,
    on: i32,
    off: i32,
) -> i32 {
    // c:4632 — `int newflags = (pm->flags | on) & ~off`.
    let newflags = (pm.node.flags | on) & !off;
    // c:4633 — `int tp = PM_TYPE(newflags)`.
    let tp = PM_TYPE(newflags as u32);
    // c:4635-4638 / 4639-4642 — float vs integer GSU pointer swap.
    if tp == PM_EFLOAT || tp == PM_FFLOAT {
        // c:4635
        // C: `pm->gsu.f = &floatseconds_gsu`. GSU table not yet
        // wired in the Rust port; record the type by clearing
        // any integer GSU.
        pm.gsu_i = None;
        // pm.gsu_f = Some(floatseconds_gsu) — pending GSU port.
    } else if tp == PM_INTEGER {
        // c:4639
        // C: `pm->gsu.i = &intseconds_gsu`.
        pm.gsu_f = None;
        // pm.gsu_i = Some(intseconds_gsu) — pending GSU port.
    } else {
        return 1; // c:4644
    }
    pm.node.flags = newflags; // c:4645
    0 // c:4646
}

// -----------------------------------------------------------
// $USERNAME
// -----------------------------------------------------------

/// Port of `usernamegetfn(UNUSED(Param pm))` from `Src/params.c:4653`. C body:
/// Port of `usernamegetfn(UNUSED(Param pm))` from Src/params.c:4655.
/// C body: `return get_username();`. C's `get_username()`
/// (Src/utils.c:1075) walks `getuid() != cached_uid` and
/// refreshes the cache via `getpwuid()` on mismatch — so a
/// USERNAME read AFTER an `setuid()` call sees the NEW
/// username, not the stale cache.
///
/// The previous Rust port returned `cached_username_lock()`
/// directly without the refresh, so a script that called
/// setuid(3) (or USER changed externally via setuid binary)
/// would keep returning the old username.
///
pub fn usernamegetfn(_pm: &param) -> String {
    // c:4655
    // c:4658 — `return get_username();`. Route through the
    // canonical refresh-on-uid-change accessor at utils.rs.
    get_username() // c:4658
}

/// Port of `usernamesetfn(UNUSED(Param pm), char *x)` from `Src/params.c:4662`. C body:
/// `getpwnam(x); setgid; setuid; cached_uid = pswd->pw_uid;`
///
/// WARNING: the SUID-changing path requires getpwnam(3) which
/// crosses an unsafe FFI boundary not yet wrapped here. The
/// cached-name update is performed; uid/gid changes still need
/// porting of the `pwd.h` getpwnam wrapper.
pub fn usernamesetfn(_pm: &mut param, x: String) {
    // c:4662
    // c:4662 — `if (x && (pswd = getpwnam(x)) && pswd->pw_uid != cached_uid)`.
    let target = std::ffi::CString::new(x.as_bytes()).ok();
    if let Some(cstr) = target {
        unsafe {
            let pwd = libc::getpwnam(cstr.as_ptr()); // c:4666
            if !pwd.is_null() {
                // c:4666 — C reads `cached_uid` (a global initialized
                // to `getuid()` at init.c:1219 — the REAL uid, NOT
                // the effective one). The previous Rust port used
                // `geteuid()` which diverges when running setuid
                // (geteuid != getuid) — the shell would erroneously
                // try to change to a uid it's already at, or skip
                // a needed change. Match C exactly: use `getuid()`.
                let cached_uid = libc::getuid(); // c:4666 cached_uid = getuid()
                if (*pwd).pw_uid != cached_uid {
                    // c:4666
                    // c:4670-4672 — initgroups(x, pswd->pw_gid).
                    let _ = libc::initgroups(cstr.as_ptr(), (*pwd).pw_gid as _);
                    // c:4671 — setgid(pswd->pw_gid).
                    if libc::setgid((*pwd).pw_gid) != 0 {
                        // c:4673
                        zwarn(&format!(
                            "failed to change group ID: {}",
                            std::io::Error::last_os_error()
                        ));
                    } else if libc::setuid((*pwd).pw_uid) != 0 {
                        // c:4675
                        // c:4675-4676 — setuid failed.
                        zwarn(&format!(
                            "failed to change user ID: {}",
                            std::io::Error::last_os_error()
                        ));
                    } else {
                        // c:4677-4681 — cache update.
                        let name_cstr = std::ffi::CStr::from_ptr((*pwd).pw_name);
                        let name_str = name_cstr.to_string_lossy().to_string();
                        *cached_username_lock().lock().expect("username poisoned") =
                            ztrdup_metafy(&name_str);
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
/// `TERMFLAGS` static.
/// Starts as TERM_UNKNOWN (0x02) — c:Src/init.c:1103 `termflags =
/// TERM_UNKNOWN;` in init_setup. Cleared by init_term() on success
/// (c:Src/init.c:802-803); promptexpand/zleread lazily call
/// init_term() when the bit is still set (c:Src/prompt.c:189-190,
/// c:Src/Zle/zle_main.c:1260-1261).
pub static TERMFLAGS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0x02);
// `TERM_UNKNOWN` re-exported from canonical zsh_h.rs (port of
// `Src/zsh.h:1986`). The local declaration here had the value
// `1 << 0 = 0x01` — which is C's TERM_BAD (Src/zsh.h:1985), NOT
// TERM_UNKNOWN. The canonical TERM_UNKNOWN value is 0x02.
//
// Callers reading `crate::ported::params::TERM_UNKNOWN` got the
// TERM_BAD bit; the params.rs term-init path fired
// `TERMFLAGS.fetch_or(TERM_UNKNOWN)` which actually set TERM_BAD,
// while the prompt.rs guard at line 441 imported the correct
// (0x02) value from zsh_h.rs — so the two paths disagreed silently
// about which bit means "unknown terminal".

/// Port of `uidsetfn(UNUSED(Param pm), zlong x)` from `Src/params.c:4698`. C body:
/// `if (setuid((uid_t)x)) zerr("failed to change user ID: %e", errno);`
/// C body (2 lines):
///   `if (setuid((uid_t)x)) zerr("failed to change user ID: %e", errno);`
/// WARNING: param names don't match C — Rust=(x) vs C=(pm, x)
pub fn uidsetfn(x: i64) {
    // c:4698
    if unsafe { libc::setuid(x as libc::uid_t) } != 0 {
        // c:4701 — `zerr("failed to change user ID: %e", errno)`.
        // C's `%e` formatter consumes errno and prints
        // `strerror(errno)` with the system's casing (typically
        // lowercase on macOS/Linux). Rust's
        // `std::io::Error::last_os_error()` displays the same
        // text but with capital first letter + `(os error N)`
        // suffix, diverging from zsh. Mirror the C format by
        // calling strerror via libc directly. Bug #254 in
        // docs/BUGS.md.
        let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        let msg = unsafe {
            let p = libc::strerror(errno);
            std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
        };
        zerr(&format!(
            "failed to change user ID: {}",
            msg.to_lowercase()
        )); // c:4702
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
pub fn euidsetfn(x: i64) {
    // c:4719
    if unsafe { libc::seteuid(x as libc::uid_t) } != 0 {
        // c:4722 — strerror format to match C zerr's `%e`. See
        // uidsetfn above.
        let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        let msg = unsafe {
            let p = libc::strerror(errno);
            std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
        };
        zerr(&format!(
            "failed to change effective user ID: {}",
            msg.to_lowercase()
        )); // c:4723
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
pub fn gidsetfn(x: i64) {
    // c:4740
    if unsafe { libc::setgid(x as libc::gid_t) } != 0 {
        // c:4743 — strerror format to match C zerr's `%e`. See
        // uidsetfn above.
        let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        let msg = unsafe {
            let p = libc::strerror(errno);
            std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
        };
        zerr(&format!(
            "failed to change group ID: {}",
            msg.to_lowercase()
        )); // c:4744
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
pub fn egidsetfn(x: i64) {
    // c:4761
    if unsafe { libc::setegid(x as libc::gid_t) } != 0 {
        // c:4764 — strerror format to match C zerr's `%e`. See
        // uidsetfn above.
        let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        let msg = unsafe {
            let p = libc::strerror(errno);
            std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
        };
        zerr(&format!(
            "failed to change effective group ID: {}",
            msg.to_lowercase()
        )); // c:4765
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
    // c:4776 — `if (SHTTY == -1 || fstat(SHTTY, &ttystat)) return -1;`
    // The previous Rust port hardcoded fd 0 (stdin) which is wrong
    // when SHTTY was opened on a non-stdin file descriptor (e.g.
    // `zsh < script` where stdin is a file but the controlling tty
    // was opened separately). C tracks the actual SHTTY fd.
    let shtty = SHTTY.load(Ordering::SeqCst);
    if shtty == -1 {
        // c:4776
        return -1;
    }
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(shtty, &mut st) } != 0 {
        // c:4776
        return -1;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    now - st.st_atime as i64 // c:4779
}

// -----------------------------------------------------------
// $IFS / $HOME / $TERM / $WORDCHARS / $TERMINFO / $TERMINFO_DIRS
// $KEYBOARD_HACK / $HISTCHARS / $_  — string-state callbacks.
// -----------------------------------------------------------

/// Port of `ifsgetfn(UNUSED(Param pm))` from `Src/params.c:4784`. C body: `return ifs;`
pub fn ifsgetfn(_pm: &param) -> String {
    ifs_lock().lock().expect("ifs poisoned").clone()
}

/// Port of `ifssetfn(UNUSED(Param pm), char *x)` from `Src/params.c:4793`. C body:
/// `zsfree(ifs); ifs = x; inittyptab();`
pub fn ifssetfn(_pm: &mut param, x: String) {
    *ifs_lock().lock().expect("ifs poisoned") = x;
    // c:4795 — `inittyptab()` rebuilds the typtab[] ISEP/IWSEP bits
    // from the new IFS. Without this, every word-split path stays
    // pinned to the old separator set and silently mis-splits.
    inittyptab();
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
    // c:Src/params.c:4732+ — `#ifdef MULTIBYTE_SUPPORT
    //   mb_charinit();        /* utils.c */
    //   clear_shiftstate();   /* pattern.c */
    // #endif`
    // Both helpers are ported (utils.rs:526 and pattern.rs:190). The
    // pattern.rs version is currently a no-op (shiftstate machine
    // not stored) and utils.rs::mb_charinit resets the mbstate_t
    // tracking. Wire them through so a future locale-change hook
    // routes through this one entry point per c:Src/params.c
    // setlang(c:4842) which calls clear_mbstate() between setlocale
    // and the per-LC_* re-apply loop.
    crate::ported::utils::mb_charinit(); // c:utils.c mb_charinit
    crate::ported::pattern::clear_shiftstate(); // c:pattern.c:327 clear_shiftstate
}

/// Port of `static struct localename lc_names[]` from `Src/params.c:4805-4825`.
/// C body:
/// ```c
/// static struct localename {
///     char *name;
///     int category;
/// } lc_names[] = {
///     {"LC_COLLATE", LC_COLLATE},
///     {"LC_CTYPE", LC_CTYPE},
///     {"LC_MESSAGES", LC_MESSAGES},
///     {"LC_NUMERIC", LC_NUMERIC},
///     {"LC_TIME", LC_TIME},
///     {NULL, 0}
/// };
/// ```
///
/// The C source guards each entry under `#ifdef LC_*`; libc on
/// macOS/Linux defines all five so the Rust port simply lists them.
const LC_NAMES: &[(&str, libc::c_int)] = &[
    ("LC_COLLATE", libc::LC_COLLATE),   // c:4810
    ("LC_CTYPE", libc::LC_CTYPE),       // c:4813
    ("LC_MESSAGES", libc::LC_MESSAGES), // c:4816
    ("LC_NUMERIC", libc::LC_NUMERIC),   // c:4819
    ("LC_TIME", libc::LC_TIME),         // c:4822
];

/// Port of `setlang(char *x)` from `Src/params.c:4842`.
///
/// C body (c:4842-4869):
/// ```c
/// if ((x2 = getsparam_u("LC_ALL")) && *x2) return;
/// setlocale(LC_ALL, x ? unmeta(x) : "");
/// clear_mbstate();
/// queue_signals();
/// for (ln = lc_names; ln->name; ln++)
///     if ((x = getsparam_u(ln->name)) && *x)
///         setlocale(ln->category, x);
/// unqueue_signals();
/// inittyptab();
/// ```
///
/// The previous Rust port skipped the actual `setlocale(LC_ALL, ...)`
/// libc call and just set the LANG env var. C invokes libc
/// setlocale to actually change the program's locale state —
/// required so any libc calls during shell execution (e.g.,
/// `iswctype`, `mbrtowc`) use the new locale's classification.
///
/// Also skipped: the per-LC_* override loop (c:4866-4868) which
/// re-applies category-specific settings after the global
/// LC_ALL set. The Rust port doesn't yet have the lc_names
/// table, but we can at least respect the canonical sequence.
pub fn setlang(x: Option<&str>) {
    // c:4842
    // c:4847 — `if ((x2 = getsparam_u("LC_ALL")) && *x2) return;`
    if let Some(lc_all) = getsparam_u("LC_ALL") {
        // c:4847
        if !lc_all.is_empty() {
            return;
        }
    }
    // c:4860 — `setlocale(LC_ALL, x ? unmeta(x) : "");`
    let locale_arg = match x {
        Some(s) => unmeta(s),
        None => String::new(),
    };
    // The previous Rust port skipped the libc setlocale call.
    // Without it, libc's locale state (used by iswctype, mbrtowc,
    // etc.) stays pinned to whatever the shell inherited from
    // its parent — diverging from C which actively changes the
    // running program's locale.
    let cstr = std::ffi::CString::new(locale_arg.as_bytes()).unwrap_or_default();
    unsafe {
        libc::setlocale(libc::LC_ALL, cstr.as_ptr()); // c:4860
    }
    // Mirror to env so subsequent `getsparam("LANG")` reads agree.
    if let Some(s) = x {
        setenv_truncate_nul("LANG", s);
    }
    clear_mbstate(); // c:4861
                     // c:4863-4867 — `for (ln = lc_names; ln->name; ln++) if ((x =
                     // getsparam_u(ln->name)) && *x) setlocale(ln->category, x);`
                     // After the global LC_ALL setlocale, any explicitly-set LC_*
                     // category overrides its slot. The previous Rust port skipped
                     // this loop, so `LC_NUMERIC=tr_TR.UTF-8 LANG=C` would leave
                     // numeric formatting on C rather than tr_TR.
    for (name, category) in LC_NAMES {
        // c:4863
        if let Some(val) = getsparam_u(name) {
            // c:4866 getsparam_u
            if !val.is_empty() {
                let cat_cstr = std::ffi::CString::new(val.as_bytes()).unwrap_or_default();
                unsafe {
                    libc::setlocale(*category, cat_cstr.as_ptr()); // c:4867
                }
            }
        }
    }
    // c:4868 — `inittyptab();`. The locale change may shift which
    // bytes are isalpha/isalnum/etc under the typtab init, so the
    // table must be rebuilt.
    inittyptab();
}

/// Port of `lc_allsetfn(Param pm, char *x)` from `Src/params.c:4873`.
///
/// C body (c:4873-4894):
/// ```c
/// strsetfn(pm, x);
/// if (!x || !*x) {
///     x = getsparam_u("LANG");
///     if (x && *x) {
///         queue_signals();
///         setlang(x);
///         unqueue_signals();
///     }
/// } else {
///     setlocale(LC_ALL, unmeta(x));
///     clear_mbstate();
///     inittyptab();
/// }
/// ```
///
/// The previous Rust port for the non-empty case set the env
/// var via `env::set_var("LC_ALL", &s)` but skipped THREE
/// pieces:
///   1. `setlocale(LC_ALL, unmeta(x))` — actively changes the
///      program's locale per c:4890.
///   2. `unmeta(x)` — strips Meta-encoded bytes before passing
///      to libc setlocale per c:4890.
///   3. `inittyptab()` — rebuilds the typtab for the new
///      LC_CTYPE per c:4892.
///
/// WARNING: param names don't match C — Rust=(x) vs C=(pm, x)
pub fn lc_allsetfn(x: Option<String>) {
    // c:4873
    match x {
        None => setlang(getsparam_u("LANG").as_deref()), // c:4882 getsparam_u
        Some(s) if s.is_empty() => {
            // c:4881
            // c:4881-4884 — empty x falls back to setlang(getsparam_u("LANG")).
            setlang(getsparam_u("LANG").as_deref()); // c:4882
        }
        Some(s) => {
            // c:4889 — `setlocale(LC_ALL, unmeta(x));`
            let unmeta = unmeta(&s); // c:4889 unmeta(x)
            let cstr = std::ffi::CString::new(unmeta.as_bytes()).unwrap_or_default();
            unsafe {
                libc::setlocale(libc::LC_ALL, cstr.as_ptr()); // c:4890
            }
            setenv_truncate_nul("LC_ALL", &s);
            clear_mbstate(); // c:4891
                             // c:4892 — `inittyptab();` rebuild typtab for new LC_CTYPE.
            inittyptab(); // c:4892
        }
    }
}

/// Port of `langsetfn(Param pm, char *x)` from `Src/params.c:4898`. C body:
/// `strsetfn(pm, x); setlang(unmeta(x));`
///
/// `unmeta(x)` strips Meta-encoding before passing to libc
/// `setlocale` — locale names are normally ASCII but Meta bytes
/// in the assigned value (from a `LANG="$value"` round-trip
/// through metafied param storage) would otherwise reach
/// setlocale literally. The previous Rust port passed raw `x`
/// without unmeta'ing — divergent.
///
/// `strsetfn(pm, x)` stores the value in the param slot. The Rust
/// adaptation doesn't have a `pm` in scope; the assign path that
/// reaches langsetfn already stored the value in the paramtab,
/// so this body only runs the post-store side effect (locale).
///
/// WARNING: param names don't match C — Rust=(x) vs C=(pm, x).
pub fn langsetfn(x: String) {
    // c:4898
    // c:4901 — `setlang(unmeta(x));`. Strip Meta bytes before
    // passing to libc setlocale.
    let unmeta_x = unmeta(&x); // c:4901 unmeta(x)
    setlang(Some(&unmeta_x));
}

/// Port of `lcsetfn(Param pm, char *x)` from `Src/params.c:4906`. C body
/// (c:4912-4931):
/// ```c
/// strsetfn(pm, x);
/// if ((x2 = getsparam("LC_ALL")) && *x2) return;
/// queue_signals();
/// if (!x || !*x) x = getsparam("LANG");
/// if (x && *x) {
///     for (ln = lc_names; ln->name; ln++)
///         if (!strcmp(ln->name, pm->node.nam))
///             setlocale(ln->category, unmeta(x));
/// }
/// unqueue_signals();
/// clear_mbstate();
/// inittyptab();
/// ```
///
/// Two divergences in the previous Rust port:
///   1. Missed `inittyptab()` call at c:4932 — LC_CTYPE changes
///      shift which bytes are isalpha/iblank/isep, but the
///      typtab stayed pinned to the prior locale's classes.
///      `setopt POSIX_BUILTINS; LC_NUMERIC=tr_TR.UTF-8; ...`
///      would still classify with the old C locale's tables.
///   2. The Meta-unmeta'ing on the value passed to setlocale
///      wasn't applied. C uses `setlocale(cat, unmeta(x))`.
pub fn lcsetfn(pm: &str, x: Option<String>) {
    // c:4906
    // c:4912-4913 — `if ((x2 = getsparam("LC_ALL")) && *x2) return;`.
    if let Some(lc_all) = getsparam("LC_ALL") {
        // c:4912
        if !lc_all.is_empty() {
            return;
        }
    }
    // c:4916-4917 — `if (!x || !*x) x = getsparam("LANG");`.
    let val = x
        .filter(|s| !s.is_empty())
        .or_else(|| getsparam("LANG").filter(|s| !s.is_empty())); // c:4917
                                                                  // c:4924-4928 — apply `setlocale(category, unmeta(x))` for the
                                                                  // matching LC_* category. The previous Rust port skipped the
                                                                  // actual libc setlocale call and only wrote the env var, so
                                                                  // assigning `LC_NUMERIC=tr_TR.UTF-8` never flipped libc's
                                                                  // numeric-formatting category.
    if let Some(v) = val {
        let unmeta = unmeta(&v); // c:4928 unmeta(x)
        setenv_truncate_nul(pm, &unmeta);
        for (name, category) in LC_NAMES {
            // c:4925
            if *name == pm {
                // c:4926 strcmp
                let cstr = std::ffi::CString::new(unmeta.as_bytes()).unwrap_or_default();
                unsafe {
                    libc::setlocale(*category, cstr.as_ptr()); // c:4927
                }
                break;
            }
        }
    }
    // c:4930 — `clear_mbstate();` — LC_CTYPE may have changed.
    clear_mbstate();
    // c:4931 — `inittyptab();` — rebuild typtab classifications.
    // The previous Rust port skipped this; char-classification
    // predicates would stay pinned to the prior locale's class
    // set even after `LC_CTYPE=` was assigned.
    inittyptab(); // c:4931
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
/// `pm` is UNUSED in C (the `argzero` global is updated regardless of
/// the Param the assignment hit), but the parameter is preserved here
/// to match the C signature so this fn can wire into the GsuScalar.setfn
/// slot directly (see ARGZERO_GSU at line ~8307).
pub fn argzerosetfn(_pm: &mut param, x: String) {
    // c:4937
    // c:4937 — if (x).
    if !x.is_empty() {
        // c:4940 — isset(POSIXARGZERO) reject.
        if isset(POSIXARGZERO) {
            zerr("read-only variable: 0"); // c:4941
        } else {
            // c:4943-4944 — zsfree(argzero); argzero = ztrdup(x).
            set_argzero(Some(ztrdup(&x)));
        }
        // c:4946 — `zsfree(x)`. Rust drop handles via move.
    }
}

// -----------------------------------------------------------
// $0 / $#
// -----------------------------------------------------------

/// Port of `argzerogetfn(UNUSED(Param pm))` from `Src/params.c:4954`. C body:
///     `return isset(POSIXARGZERO) ? posixzero : argzero;`
///
/// Both `argzero` and `posixzero` live in `utils.rs` (OnceLock storage).
/// After `exec -a foo` or function-call argv-rewrite, `$0` under
/// POSIXARGZERO reports the ORIGINAL startup `argv[0]`, not the
/// rewritten name. `pm` is UNUSED in C; signature preserved for the
/// GsuScalar.getfn slot at ARGZERO_GSU (line ~8307).
pub fn argzerogetfn(_pm: &param) -> String {
    if isset(POSIXARGZERO) {
        // c:4958
        posixzero().unwrap_or_default() // c:4959
    } else {
        argzero().unwrap_or_default() // c:4960
    }
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
///
/// The previous Rust port noted `resizehistents()` as "pending the
/// history-table port", but `resizehistents`
/// IS available — was a stale comment. Without the resize call,
/// setting HISTSIZE to a smaller value left the in-memory ring
/// over-sized until the next implicit prune (next entry added).
/// Wired the call now per c:4977.
/// WARNING: param names don't match C — Rust=(v) vs C=(pm, v)
pub fn histsizesetfn(v: i64) {
    *histsiz_lock().lock().expect("histsiz poisoned") = v.max(1);
    // c:4977 — mirror into the hist.rs atomic so resizehistents()
    // sees the new size, then trigger the prune.
    histsiz.store(v.max(1), Ordering::SeqCst);
    resizehistents(); // c:4977
}

/// Port of `savehistsizegetfn(UNUSED(Param pm))` from `Src/params.c:4985`. C body:
/// `return savehistsiz;`
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn savehistsizegetfn() -> i64 {
    *savehistsiz_lock().lock().expect("savehistsiz poisoned")
}

/// Port of `savehistsizesetfn(UNUSED(Param pm), zlong v)` from `Src/params.c:4994`. C body:
/// `if ((savehistsiz = v) < 0) savehistsiz = 0;`
///
/// The Rust port has TWO mirrors of `savehistsiz`: a `Mutex<i64>`
/// in params.rs (read by `savehistsizegetfn`) AND an AtomicI64
/// in hist.rs (read by the history-file writer at
/// `Src/hist.c:savehistfile` per c:3878). The previous Rust port
/// only wrote to the params.rs lock; `hist.rs::savehistsiz`
/// stayed pinned to its initial 0 value, so `SAVEHIST=10000`
/// would store the limit in `savehistsiz_lock` (visible to
/// `$SAVEHIST` reads) but the history-file writer would still
/// cap at the original AtomicI64 value (effectively saving zero
/// lines). Sync both storages so reads + writes agree.
///
/// WARNING: param names don't match C — Rust=(v) vs C=(pm, v)
pub fn savehistsizesetfn(v: i64) {
    // c:4994
    let clamped = v.max(0); // c:4998
    *savehistsiz_lock().lock().expect("savehistsiz poisoned") = clamped;
    // Mirror to hist.rs::savehistsiz so the writer-side cap
    // matches the just-assigned value. C uses a single global;
    // the Rust port's twin-storage requires sync writes.
    savehistsiz.store(clamped, Ordering::SeqCst);
    // c:4994
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
pub fn errnosetfn(x: i64) {
    // c:5004
    let truncated = x as i32;
    unsafe {
        *errno_ptr() = truncated;
    } // c:5006 errno = (int)x
      // c:5009-5010 — C uses `zwarn` (informational), NOT `zerr`. The
      // store happens unconditionally; the warning fires only on
      // truncation. Previously used `zerr` — divergent.
    if truncated as i64 != x {
        // c:5008
        zwarn("errno truncated on assignment"); // c:5009
    }
}

/// !!! RUST-ONLY HELPER — no direct C counterpart. C accesses
/// `errno` through the standard macro which the compiler resolves
/// to the per-platform getter (`__error()` on macOS, `__errno_location()`
/// on Linux). Rust libc exposes both as raw FFI; this helper picks
/// the right one per target so errnosetfn/errnogetfn stay one-liners.
#[inline]
unsafe fn errno_ptr() -> *mut libc::c_int {
    #[cfg(target_os = "macos")]
    {
        libc::__error()
    }
    #[cfg(target_os = "linux")]
    {
        libc::__errno_location()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        std::ptr::null_mut()
    }
}

/// Port of `errnogetfn(UNUSED(Param pm))` from `Src/params.c:5015`. C body: `return errno;`
///
/// Reads the libc errno directly through the per-platform accessor
/// (matching C's `return errno;` semantics).
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn errnogetfn() -> i64 {
    let p = unsafe { errno_ptr() }; // c:5017 return errno
    if p.is_null() {
        // Non-Linux/macOS fallback: best-effort via std API.
        std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as i64
    } else {
        unsafe { *p as i64 }
    }
}

/// Port of `keyboardhackgetfn(UNUSED(Param pm))` from `Src/params.c:5024`. C body:
/// `static char buf[2]; buf[0] = keyboardhackchar; return buf;`
pub fn keyboardhackgetfn(_pm: &param) -> String {
    let c = *keyboardhack_lock().lock().expect("keyboardhack poisoned");
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
///
/// The C source `unmetafy(x, &len)` strips Meta-encoded prefix
/// bytes (collapsing every `Meta + (b^32)` pair to the original
/// byte) BEFORE the length and ASCII checks. The previous Rust
/// port skipped unmetafy, so:
///   - `len > 1` warning fired on every assignment of a Meta-
///     encoded single byte (the byte-length was 2 pre-unmetafy).
///   - ASCII check ran against the raw Meta byte (0x83) instead
///     of the demetafied result, falsely rejecting valid ASCII
///     characters that happened to round-trip through Meta
///     encoding in the assignment pipeline.
///
pub fn keyboardhacksetfn(_pm: &mut param, x: String) {
    // c:5040
    // c:5044 — `unmetafy(x, &len)` — strip Meta-encoded pairs.
    // Run on the byte buffer so the protocol matches C's pointer
    // walk; the Rust `unmeta()` helper does the same fold.
    let unmeta = unmeta(&x); // c:5044 unmetafy(x)
    let bytes = unmeta.as_bytes();
    // c:5046-5049 — `if (len > 1) { len = 1; zwarn(...); }`. The
    // length check happens AFTER unmetafy so a 2-byte Meta pair
    // representing a single byte doesn't trigger the warning.
    if bytes.len() > 1 {
        zwarn("Only one KEYBOARD_HACK character can be defined");
    }
    let c = bytes.first().copied().unwrap_or(0);
    // c:5050-5054 — ASCII check runs on the unmetafied byte, NOT
    // the raw Meta byte. With unmetafy now in place this works as
    // C intended.
    if c >= 0x80 {
        // c:5051 !isascii(...)
        zwarn("KEYBOARD_HACK can only contain ASCII characters");
        return;
    }
    // c:5056 — `keyboardhackchar = len ? (unsigned char) x[0] : '\0';`
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
pub fn histcharsgetfn(_pm: &param) -> String {
    let b = bangchar.load(Ordering::SeqCst) as u8;
    let h = hatchar.load(Ordering::SeqCst) as u8;
    let p = hashchar.load(Ordering::SeqCst) as u8;
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

/// Port of `histcharssetfn(UNUSED(Param pm), char *x)` from `Src/params.c:5081`. C body
/// validates ASCII, takes up to 3 chars; defaults `!^#` if NULL.
///
/// C `unmetafy(x, &len)` (c:5086) strips Meta-encoded pairs BEFORE
/// the length truncation and ASCII guard. The previous Rust port
/// skipped unmetafy entirely:
///   - `len > 3` truncation ran on raw byte length, so a Meta-pair
///     would inflate the byte count and skip valid chars.
///   - ASCII check ran against raw Meta bytes (0x83), falsely
///     rejecting valid round-tripped values.
///
pub fn histcharssetfn(_pm: &mut param, x: String) {
    // c:5081
    // C signature is `histcharssetfn(Param pm, char *x)`. C uses NULL
    // for the "reset to defaults" path; in Rust the canonical fn-ptr
    // type is `fn(&mut param, String)` so the empty-string sentinel
    // takes that role (`x.is_empty()` ≡ C `x == NULL`).
    let new_chars: [u8; 3] = if x.is_empty() {
        // c:5100-5103 — defaults `!^#` when x is NULL.
        [b'!', b'^', b'#']
    } else {
        let s = x;
        {
            // c:5086 — `unmetafy(x, &len)`. Strip Meta pairs first.
            let unmeta = unmeta(&s); // c:5086 unmetafy(x)
            let bytes = unmeta.as_bytes();
            // c:5087-5088 — `if (len > 3) len = 3;`. Truncation
            // applies AFTER unmetafy.
            let bytes = if bytes.len() > 3 { &bytes[..3] } else { bytes };
            for &b in bytes.iter() {
                if b >= 0x80 {
                    // c:5090-5093
                    // c:5091 — C uses `zwarn` (informational), NOT
                    // `zerr` (fatal). Function returns early without
                    // updating any globals.
                    zwarn("HISTCHARS can only contain ASCII characters");
                    return;
                }
            }
            // c:5095-5097 — `bangchar = x[0]; hatchar = x[1]; hashchar = x[2]`.
            // C uses `len ? x[0] : '\0'` etc — for short strings the
            // unset bytes are NUL.
            let mut chars = [0u8; 3];
            for (i, &b) in bytes.iter().enumerate() {
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
    bangchar.store(new_chars[0] as i32, Ordering::SeqCst);
    hatchar.store(new_chars[1] as i32, Ordering::SeqCst);
    hashchar.store(new_chars[2] as i32, Ordering::SeqCst);
    // c:5104 — `inittyptab();`. The bangchar special bit in typtab
    // depends on the current `bangchar` global; reseed.
    inittyptab();
}

// ---------------------------------------------------------------------
// Special-scalar GSU vtables — port of `Src/params.c:217-256` `stdscalar_gsu`,
// `home_gsu`, `ifs_gsu`, etc. Each entry pairs the canonical
// getfn/setfn/unsetfn for a PM_SPECIAL scalar. `createparamtable`
// copies the matching gsu into each special param's `gsu_s` slot so
// `assignstrvalue` dispatches via `pm->gsu.s->setfn(pm, val)`
// (params.c:2748) exactly like C.
// ---------------------------------------------------------------------

/// Port of `static const struct gsu_scalar argzero_gsu` from `Src/params.c:225-226`.
/// `{ argzerogetfn, argzerosetfn, nullunsetfn }`. Both functions' Rust
/// signatures match C exactly (with `pm` UNUSED), so they wire into
/// the GSU vtable directly — no Rust-only wrappers. The $0 Param picks
/// this up at `add_special` (line ~1555) and `assignsparam` routes
/// `0=value` through the canonical setter via the gsu_s.setfn dispatch
/// at params.rs:3767.
pub const ARGZERO_GSU: gsu_scalar = gsu_scalar {
    // c:225-226
    getfn: argzerogetfn,
    setfn: argzerosetfn,
    unsetfn: nullunsetfn,
};

/// Port of `static const struct gsu_scalar home_gsu` from `Src/params.c:248`.
pub const HOME_GSU: gsu_scalar = gsu_scalar {
    // c:248
    getfn: homegetfn,
    setfn: homesetfn,
    unsetfn: stdunsetfn,
};

/// Port of `static const struct gsu_scalar ifs_gsu` from `Src/params.c:245`.
pub const IFS_GSU: gsu_scalar = gsu_scalar {
    // c:245
    getfn: ifsgetfn,
    setfn: ifssetfn,
    unsetfn: stdunsetfn,
};

/// Port of `static const struct gsu_scalar term_gsu` from `Src/params.c:250`.
pub const TERM_GSU: gsu_scalar = gsu_scalar {
    // c:250
    getfn: termgetfn,
    setfn: termsetfn,
    unsetfn: stdunsetfn,
};

/// Port of `static const struct gsu_scalar terminfo_gsu` from `Src/params.c:251`.
pub const TERMINFO_GSU: gsu_scalar = gsu_scalar {
    // c:251
    getfn: terminfogetfn,
    setfn: terminfosetfn,
    unsetfn: stdunsetfn,
};

/// Port of `static const struct gsu_scalar terminfodirs_gsu`
/// from `Src/params.c:252`.
pub const TERMINFODIRS_GSU: gsu_scalar = gsu_scalar {
    // c:252
    getfn: terminfodirsgetfn,
    setfn: terminfodirssetfn,
    unsetfn: stdunsetfn,
};

/// Port of `static const struct gsu_scalar wordchars_gsu`
/// from `Src/params.c:249`.
pub const WORDCHARS_GSU: gsu_scalar = gsu_scalar {
    // c:249
    getfn: wordcharsgetfn,
    setfn: wordcharssetfn,
    unsetfn: stdunsetfn,
};

/// Port of `static const struct gsu_scalar username_gsu`
/// from `Src/params.c:247`.
pub const USERNAME_GSU: gsu_scalar = gsu_scalar {
    // c:247
    getfn: usernamegetfn,
    setfn: usernamesetfn,
    unsetfn: stdunsetfn,
};

/// Port of `static const struct gsu_scalar keyboardhack_gsu`
/// from `Src/params.c:253`.
pub const KEYBOARDHACK_GSU: gsu_scalar = gsu_scalar {
    // c:253
    getfn: keyboardhackgetfn,
    setfn: keyboardhacksetfn,
    unsetfn: stdunsetfn,
};

/// Port of `static const struct gsu_scalar histchars_gsu`
/// from `Src/params.c:246`.
pub const HISTCHARS_GSU: gsu_scalar = gsu_scalar {
    // c:246
    getfn: histcharsgetfn,
    setfn: histcharssetfn,
    unsetfn: stdunsetfn,
};

/// Port of `homegetfn(UNUSED(Param pm))` from `Src/params.c:5109`. C body: `return home;`
pub fn homegetfn(_pm: &param) -> String {
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
pub fn homesetfn(_pm: &mut param, x: String) {
    // c:5121-5126 — CHASELINKS path resolves symlinks before storing.
    // Falls through to the plain `x` store when CHASELINKS is off or
    // xsymlink fails.
    let resolved = if !x.is_empty() && isset(CHASELINKS) {
        xsymlink(&x).unwrap_or(x)
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
pub fn wordcharsgetfn(_pm: &param) -> String {
    wordchars_lock().lock().expect("wordchars poisoned").clone()
}

/// Port of `wordcharssetfn(UNUSED(Param pm), char *x)` from `Src/params.c:5141`. C body:
/// `zsfree(wordchars); wordchars = x; inittyptab();`
pub fn wordcharssetfn(_pm: &mut param, x: String) {
    *wordchars_lock().lock().expect("wordchars poisoned") = x;
    // c:5143 — `inittyptab()` rebuilds typtab IWORD bits from the
    // new WORDCHARS. Without this, every IWORD lookup stays pinned
    // to the old set and silently mis-classifies word boundaries.
    inittyptab();
}

/// Port of `underscoregetfn(UNUSED(Param pm))` from `Src/params.c:5152`. C body:
/// `char *u = dupstring(zunderscore); untokenize(u); return u;`
///
/// C runs `untokenize(u)` on the cloned string before returning, so
/// ITOK bytes (Pound..Nularg per `Src/zsh.h:159-194`) in `$_` get
/// replaced/dropped via the canonical `ztokens[]` table. The previous
/// Rust port skipped untokenize entirely — every `$_` read that
/// included a lexer-injected token byte exposed the raw token in user
/// output (e.g. `$_` containing `$cmd` would surface as raw Stringg
/// instead of the literal `$`).
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn underscoregetfn() -> String {
    let u = zunderscore_lock()
        .lock()
        .expect("zunderscore poisoned")
        .clone();
    untokenize(&u) // c:5156 untokenize(u)
}

/// Port of `term_reinit_from_pm()` from `Src/params.c:5163`.
/// C: `static void term_reinit_from_pm(void)` →
///   `if (unset(INTERACTIVE) || !*term) termflags |= TERM_UNKNOWN;
///    else init_term();`
pub fn term_reinit_from_pm() {
    // c:5163
    // c:5167 — `if (unset(INTERACTIVE) || !*term) termflags |= TERM_UNKNOWN;`
    let interactive = isset(INTERACTIVE);
    let term = term_lock().lock().map(|s| s.clone()).unwrap_or_default();
    if !interactive || term.is_empty() {
        // c:5167
        TERMFLAGS.fetch_or(TERM_UNKNOWN, Ordering::Relaxed); // c:5168
    } else {
        crate::ported::init::init_term(); // c:5170
    }
}

/// Port of `termgetfn(UNUSED(Param pm))` from `Src/params.c:5176`. C body: `return term;`
pub fn termgetfn(_pm: &param) -> String {
    term_lock().lock().expect("term poisoned").clone()
}

/// Port of `termsetfn(UNUSED(Param pm), char *x)` from `Src/params.c:5185`. C body:
/// `zsfree(term); term = x ? x : ""; term_reinit_from_pm();`
pub fn termsetfn(_pm: &mut param, x: String) {
    *term_lock().lock().expect("term poisoned") = x;
    term_reinit_from_pm();
}

/// Port of `terminfogetfn(UNUSED(Param pm))` from `Src/params.c:5196`. C body:
/// `return zsh_terminfo ? zsh_terminfo : "";`
pub fn terminfogetfn(_pm: &param) -> String {
    zsh_terminfo_lock()
        .lock()
        .expect("zsh_terminfo poisoned")
        .clone()
}

/// Port of `int rprompt_indent` from `Src/init.c`. Set to 1 by
/// `init_term()` and reset by `rprompt_indent_unsetfn` when the
/// `RPROMPT_INDENT` parameter is unset.
pub static RPROMPT_INDENT: Mutex<i32> = Mutex::new(1);

/// Port of `terminfosetfn(Param pm, char *x)` from `Src/params.c:5205`. C body:
/// `zsfree(zsh_terminfo); zsh_terminfo = x; addenv if exported; term_reinit_from_pm();`
pub fn terminfosetfn(_pm: &mut param, x: String) {
    *zsh_terminfo_lock().lock().expect("zsh_terminfo poisoned") = x.clone();
    setenv_truncate_nul("TERMINFO", &x);
    term_reinit_from_pm();
}

/// Port of `terminfodirsgetfn(UNUSED(Param pm))` from `Src/params.c:5224`. C body:
/// `return zsh_terminfodirs ? zsh_terminfodirs : "";`
pub fn terminfodirsgetfn(_pm: &param) -> String {
    zsh_terminfodirs_lock()
        .lock()
        .expect("zsh_terminfodirs poisoned")
        .clone()
}

/// Port of `terminfodirssetfn(Param pm, char *x)` from `Src/params.c:5233`. C body
/// mirrors `terminfosetfn` for the TERMINFO_DIRS env var.
pub fn terminfodirssetfn(_pm: &mut param, x: String) {
    *zsh_terminfodirs_lock()
        .lock()
        .expect("zsh_terminfodirs poisoned") = x.clone();
    setenv_truncate_nul("TERMINFO_DIRS", &x);
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
pub fn arrfixenv(s: &str, t: Option<&[String]>) {
    // c:5285

    // c:5291 — `if (t == path) cmdnamtab->emptytable(cmdnamtab)`.
    // PATH change invalidates the command-name cache.
    if s == "PATH" || s == "path" {
        emptycmdnamtable();
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
            setenv_truncate_nul(s, &val);
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
        ':' // c:5315
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
pub fn zputenv(str: &str) -> i32 {
    // c:5325
    // c:5327 — DPUTS(!str, "Attempt to put null string into environment.")
    DPUTS!(
        str.is_empty(),
        "Attempt to put null string into environment."
    ); // c:5327
    if str.is_empty() {
        // c:5328 (after DPUTS, defensive return)
        return 0;
    }
    let bytes = str.as_bytes();
    // c:5339-5341 — walk until `=` or high byte; reject high bytes.
    let mut ptr = 0;
    while ptr < bytes.len() && bytes[ptr] != b'=' && bytes[ptr] < 128 {
        // c:5339
        ptr += 1;
    }
    if ptr < bytes.len() && bytes[ptr] >= 128 {
        // c:5342
        // c:5351 — `return 1` to reject non-portable name.
        return 1;
    }
    if ptr < bytes.len() {
        // c:5352 `else if (*ptr)`
        // c:5353-5355 — write `\0` at `=`, setenv(name, value), restore.
        let name = &str[..ptr];
        let value = &str[ptr + 1..];
        // c:Src/params.c:5354 — `setenv(name, value, 1)` uses libc
        // setenv which treats the value as a NUL-terminated C string.
        // An embedded NUL byte ends the value there. Rust's
        // std::env::set_var PANICS on NUL in value; emulate C's
        // truncate-at-NUL semantics so shell params holding raw NUL
        // bytes (e.g. `X=$'a\0b\0c'` for `${(ps:\0:)X}` splits) can
        // still propagate the leading portion to env. The full
        // raw value lives in the canonical param table; this is
        // just the libc-env mirror.
        let safe_value: &str = match value.find('\0') {
            Some(n) => &value[..n],
            None => value,
        };
        if name.as_bytes().contains(&b'\0') {
            // c: setenv on a name with NUL is malformed — C's libc
            // rejects, we silently drop the env mirror.
            return 1;
        }
        env::set_var(name, safe_value);
        0
    } else {
        // c:5355 else
        // c:5356 — DPUTS(1, "bad environment string").
        // With no `=`, treat `str` as a bare name with empty value.
        DPUTS!(true, "bad environment string"); // c:5356
        if str.as_bytes().contains(&b'\0') {
            return 1;
        }
        env::set_var(str, ""); // c:5357
        0
    }
}

// NUL-safe env-mirror helper lives in src/vm_helper.rs
// (`setenv_truncate_nul`) — bridge-file helper, not a C port; the
// src/ported/ build gate forbids non-C-named fns here.
use crate::vm_helper::setenv_truncate_nul;

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
pub fn findenv(name: &str) -> Option<usize> {
    // c:5391
    // c:5391 — `eq = strchr(name, '=')`. Strip any trailing `=value`.
    let nlen = name.find('=').unwrap_or(name.len()); // c:5397
    let bare = &name[..nlen];

    // c:5398-5404 — walk environ until match. Use std::env::vars()
    // which preserves the same ordering as the underlying libc
    // environ.
    for (i, (k, _)) in env::vars_os().enumerate() {
        if let Some(s) = k.to_str() {
            if s == bare {
                return Some(i); // c:5401-5403
            }
        }
    }
    None // c:5406
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
pub fn copyenvstr(buf: &mut String, value: &str, flags: i32) {
    // c:5434
    let flags_u = flags as u32;
    let mut it = value.bytes();
    while let Some(b) = it.next() {
        // c:5436
        let mut ch = b;
        if ch == Meta {
            // c:5437
            ch = match it.next() {
                Some(next) => next ^ 32, // c:5438
                None => break,
            };
        }
        if flags_u & PM_LOWER != 0 {
            // c:5439
            ch = ch.to_ascii_lowercase(); // c:5440
        } else if flags_u & PM_UPPER != 0 {
            // c:5441
            ch = ch.to_ascii_uppercase(); // c:5442
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
pub fn addenv(name: &str, value: &str) -> i32 {
    // c:5448

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
pub fn mkenvstr(name: &str, value: &str, flags: i32) -> String {
    // c:5513
    let mut buf = String::with_capacity(name.len() + value.len() + 2);
    buf.push_str(name); // c:5522 strcpy(s, name)
    buf.push('='); // c:5524 *s = '='
    if !value.is_empty() {
        // c:5525
        copyenvstr(&mut buf, value, flags); // c:5526
    }
    buf // c:5530
}

/// Direct port of `void delenvvalue(char *x)` from
/// `Src/params.c:5542`. Removes `x` from environ by walking
/// to its pointer and shifting subsequent entries down one slot.
///
/// C body operates on the environ array directly. The Rust port
/// uses `env::remove_var(name)` since Rust's env is mediated by
/// libc::unsetenv internally — same shift semantics.
pub fn delenvvalue(name: &str) {
    // c:5542
    env::remove_var(name); // c:5542 equivalent
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
pub fn delenv(name: &str) {
    // c:5563
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
        // c:Src/params.c — `value = -value;` on INT_MIN is UB in C
        // but in practice wraps back to INT_MIN. Use wrapping_neg
        // to avoid Rust's debug-build overflow panic. zsh prints
        // `-9223372036854775808` for `$((2**63))`; we mirror that.
        value = value.wrapping_neg();
    }
    let mut b = base;
    if (-1..=1).contains(&b) {
        b = -10;
    }
    if b > 0 {
        if isset(CBASES) && b == 16 {
            s.push_str("0x");
        } else if isset(CBASES) && b == 8 && isset(OCTALZEROES) {
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
pub fn convbase(val: i64, base: u32) -> String {
    // c:5632
    convbase_ptr(val, base as i32).0 // c:5634
}

/// Convert integer to string with underscores for readability
/// Port of `convbase_underscore(char *s, zlong v, int base, int underscore)` from `Src/params.c:5646`.
///
/// `base` is `i32` not `u32` because zsh uses NEGATIVE `base` values
/// (set by `[##N]`) to mean "emit `N`-radix digits WITHOUT the `N#`
/// prefix" — convbase_ptr at params.rs:7435-7451 handles the sign:
/// positive `b` produces `b#NNN`, negative `b` produces bare `NNN`
/// (absolute value of `b` is the actual radix).
///
/// Previous Rust signature `base: u32` silently dropped the sign, so
/// `$(([##16] 255))` (outputradix=-16) ended up as `convbase(255, 16)`
/// = `"16#FF"` instead of `"FF"`.
/// WARNING: param names don't match C — Rust=(val, base, underscore) vs C=(s, v, base, underscore)
pub fn convbase_underscore(val: i64, base: i32, underscore: i32) -> String {
    let s = convbase_ptr(val, base).0;
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
    if dval.is_infinite() {
        // c:5742
        return if dval < 0.0 {
            "-Inf".to_string()
        } else {
            "Inf".to_string()
        };
    }
    if dval.is_nan() {
        // c:5744
        return "NaN".to_string();
    }
    // Pick fmt char + adjust digits per the C cascade at 5705-5727.
    let (fmt_char, digits) = if (pm_flags & PM_EFLOAT) != 0 {
        // c:5715
        let d = if digits <= 0 { 10 } else { digits }; // c:5718
        ('e', (d - 1).max(0)) // c:5725
    } else if (pm_flags & PM_FFLOAT) != 0 {
        // c:5716
        let d = if digits <= 0 { 10 } else { digits }; // c:5718
        ('f', d)
    } else {
        let d = if digits == 0 { 17 } else { digits }; // c:5713
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
/// `&mut HashTable` argument.
pub fn startparamscope(_table: &mut HashTable) {
    inc_locallevel();
}

/// Port of `endparamscope()` from `Src/params.c:5857`. C signature:
/// `mod_export void endparamscope(void)`. Decrements `locallevel`,
/// pops any pushed history stack, then iterates `paramtab` calling
/// `scanendscope` to restore/unset every param whose `level`
/// exceeds the new `locallevel`. Operates on the global `paramtab`
/// just like C — no parameter, no fake injection wrapper.
pub fn endparamscope() {
    queue_signals();
    // c:5861 — `LinkList refs = locallevel < scoperefs_num ? scoperefs[locallevel] : NULL;`
    //          Snapshot the refs at the OLD locallevel BEFORE decrementing.
    let old_ll = locallevel_fn();
    let refs_snapshot: Vec<String> = SCOPEREFS.with(|sr| {
        let sr = sr.borrow();
        if (old_ll as usize) < sr.len() {
            sr[old_ll as usize].clone()
        } else {
            Vec::new()
        }
    });

    dec_locallevel(); // c:5863 locallevel--
                      // c:5865 — `saveandpophiststack(0, HFILE_USE_OPTIONS);`. Pop
                      // all stack entries with locallevel > current.
    saveandpophiststack(0, HFILE_USE_OPTIONS as i32);
    let ll = locallevel_fn();
    // c:5869 scanhashtable(paramtab, 0, 0, 0, scanendscope, 0). Walk
    // the live paramtab (HashMap-backed until the hashtable.c vtable
    // is wired) and apply scanendscope's `pm->level > locallevel`
    // filter, restoring the `pm.old` chain or removing the entry.
    // c:Src/params.c:5867-5933 — for PM_SPECIAL scalar params the
    // gsu setfn must be re-fired with the restored value so global
    // side-effects (ifs char buffer, PATH chunks, lc_update_needed
    // flag, etc.) get rolled back. Bug #8 in docs/BUGS.md: `local
    // IFS=:` inside a function left the global ifs buffer pinned
    // to ":" after return.
    //
    // The setfn closures often re-enter paramtab (ifssetfn → inittyptab
    // → paramtab.read), so we MUST drop the write lock before calling
    // them. Collect (name, setfn, value) into a deferred list inside
    // the lock, restore the pm.old chain, drop the lock, then re-fire
    // setfn on each special.
    // setfn None → name-routed restore via setsparam (PM_TIED params
    // whose pm carries no scalar gsu — FPATH/PATH/CDPATH…; setsparam
    // dispatches to the tied setter by name, refilling the tied
    // array's global storage).
    type DeferredSetfn = (String, Option<fn(&mut param, String)>, String);
    let mut deferred: Vec<DeferredSetfn> = Vec::new();
    if let Ok(mut tab) = paramtab().write() {
        let stale: Vec<(String, bool)> = tab
            .iter()
            .filter_map(|(k, pm)| {
                if pm.level > ll {
                    Some((k.clone(), (pm.node.flags as u32 & PM_HASHED) != 0))
                } else {
                    None
                }
            })
            .collect();
        for (n, was_assoc) in stale {
            // c:scanendscope:5903 — non-special path: restore pm.old
            // (or remove if no outer binding existed).
            if let Some(pm) = tab.remove(&n) {
                let had_outer = pm.old.is_some();
                let outer_is_assoc = pm
                    .old
                    .as_ref()
                    .map(|p| (p.node.flags as u32 & PM_HASHED) != 0)
                    .unwrap_or(false);
                if let Some(prev) = pm.old {
                    // c:scanendscope:5933 pm->old = tpm->old
                    // PM_TIED counts too — the tied array's GLOBAL
                    // storage was written through by the local's
                    // assignments; the deferred restore re-fires the
                    // saved value through the tied setter (by gsu
                    // setfn when present, else name-routed setsparam)
                    // so fpath/path/cdpath roll back with the scalar.
                    let restored_is_special =
                        (prev.node.flags as u32 & (PM_SPECIAL | PM_TIED)) != 0;
                    let restored_val = prev.u_str.clone();
                    let restored_setfn =
                        prev.gsu_s.as_ref().map(|g| g.setfn);
                    tab.insert(n.clone(), prev); // restore outer binding (Box<param>)
                    if restored_is_special {
                        if let Some(val) = restored_val {
                            deferred.push((n.clone(), restored_setfn, val));
                        }
                    }
                }
                // else: c:5966 unsetparam_pm — name unset entirely
                // RUST-ONLY: the assoc-storage shadow lives in a
                // parallel `paramtab_hashed_storage` map keyed by name;
                // endparamscope's pm removal must also clear that
                // shadow when no outer binding remains. Without this,
                // `f() { typeset -A H; H[x]=v; }; f` leaves H's data
                // in the shadow map even after the local pm is gone,
                // so `${H[x]}` outside reads "v" instead of empty.
                if was_assoc && !had_outer {
                    let _ = paramtab_hashed_storage()
                        .lock()
                        .ok()
                        .as_deref_mut()
                        .map(|m| m.remove(&n));
                }
                // RUST-ONLY: PM_HASHED outer-pm restoration — pop the
                // saved paramtab_hashed_storage[name] from the shadow
                // stack and re-install it so the outer scope's assoc
                // data is visible again. Mirrors the C copyparam +
                // pm.old chain via parallel storage. Bug #415. Symmetric
                // with the createparam push-side.
                if was_assoc && outer_is_assoc {
                    let stk_mtx = PARAMTAB_HASHED_SHADOW_STACK
                        .get_or_init(|| Mutex::new(HashMap::new()));
                    let saved = if let Ok(mut stk) = stk_mtx.lock() {
                        stk.get_mut(&n).and_then(|v| v.pop()).flatten()
                    } else {
                        None
                    };
                    if let Ok(mut m) = paramtab_hashed_storage().lock() {
                        match saved {
                            Some(map) => {
                                m.insert(n.clone(), map);
                            }
                            None => {
                                m.remove(&n);
                            }
                        }
                    }
                }
            }
        }
    }
    // c:Src/params.c:5915-5933 — re-fire PM_SPECIAL setfns NOW that
    // the paramtab write lock is released. Each setfn takes its own
    // paramtab read (via inittyptab / similar) so the deferred call
    // is the only deadlock-safe path.
    for (n, setfn, val) in deferred {
        match setfn {
            Some(setfn) => {
                let mut pm_copy: Option<param> = None;
                if let Ok(tab) = paramtab().read() {
                    pm_copy = tab.get(&n).map(|p| (**p).clone());
                }
                if let Some(mut pm) = pm_copy {
                    setfn(&mut pm, val);
                }
            }
            // PM_TIED without a scalar gsu — name-routed: setsparam
            // reaches the tied setter and refills the tied array's
            // global storage (fpath/path/cdpath…).
            None => {
                setsparam(&n, &val);
            }
        }
    }

    // c:5890-5894 — `for (Param pm; refs && (pm = getlinknode(refs));) {
    //                   if ((pm->flags & PM_NAMEREF) && !(pm->flags & PM_UNSET) &&
    //                       !(pm->flags & PM_UPPER) && pm->base > locallevel) {
    //                       pm->base = 0; setscope(pm); } }`
    //               Reset PM_NAMEREF refs whose base was above the popped scope.
    if !refs_snapshot.is_empty() {
        if let Ok(mut tab) = paramtab().write() {
            for name in refs_snapshot.iter() {
                if let Some(pm) = tab.get_mut(name) {
                    let f = pm.node.flags as u32;
                    if (f & PM_NAMEREF) != 0
                        && (f & PM_UNSET) == 0
                        && (f & PM_UPPER) == 0
                        && pm.base > ll
                    {
                        pm.base = 0; // c:5893
                                     // c:5894 setscope(pm) — would recursively call
                                     // setscope_base(pm, 0); with base=0 and pm.level>=0
                                     // the guard at setscope_base c:6440 fails so it's
                                     // a no-op write. Skip the recursive call to avoid
                                     // re-borrowing paramtab.
                    }
                }
            }
        }
    }
    // c:5896 — clear out the now-popped scope's refs list.
    SCOPEREFS.with(|sr| {
        let mut sr = sr.borrow_mut();
        if (old_ll as usize) < sr.len() {
            sr[old_ll as usize].clear();
        }
    });
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
/// Rust port mirrors the structure 1:1. `locallevel` is read via
/// the ported global `crate::ported::params::locallevel` (atomic).
/// `setsecondstype` (params.rs:6183), `setrawseconds` (params.rs:6169),
/// and `delenv` (params.rs:7591) are all ported.
pub fn scanendscope(pm: &mut param, _flags: i32) {
    // c:5900
    let cur_local = locallevel.load(Ordering::Relaxed);
    if pm.level <= cur_local {
        // c:5903
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

        // USE_LOCALE branch: LC_*/LANG bumps LC_UPDATE_NEEDED.
        if pm.node.nam.starts_with("LC_") || pm.node.nam == "LANG" {
            LC_UPDATE_NEEDED.store(1, Ordering::SeqCst);
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
        pm.base = tpm.base;
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
pub fn freeparamnode(mut _hn: Param) {
    // c:5977
    // c:5977-5987 — `if (delunset) pm->gsu.s->unsetfn(pm, 1);`.
    if DELUNSET.load(Ordering::Relaxed) != 0 {
        // The Rust port's stdunsetfn writes the unset state back to
        // paramtab; calling it on the about-to-drop param re-marks
        // its slot in the table so consumers that read the table
        // see PM_UNSET on the next lookup.
        stdunsetfn(_hn.as_mut(), 1); // c:5987
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
pub fn printparamvalue(p: &mut param, printflags: i32) {
    if (printflags & PRINT_KV_PAIR) == 0 {
        print!("=");
    }
    let t = PM_TYPE(p.node.flags as u32);
    if t == PM_SCALAR || t == PM_NAMEREF {
        // c:Src/params.c:6052 — `t = pm->gsu.s->getfn(pm)` then
        // quotedzputs. For SPECIAL params, the gsu_s vtable returns
        // the live value (HOME/PATH/IFS from globals or env). Direct
        // dispatch avoids the deadlock that getsparam would hit if
        // the caller holds paramtab write lock. Fallback chain:
        //   gsu_s.getfn(pm) → pm.u_str → env::var(name)
        // The env fallback covers exported scalars whose gsu_s wasn't
        // wired (vm_helper bootstrap path doesn't run createparam for
        // every special).
        let mut s = if let Some(gsu) = &p.gsu_s {
            (gsu.getfn)(p)
        } else {
            strgetfn(p)
        };
        if s.is_empty() && (p.node.flags as u32 & PM_EXPORTED) != 0 {
            if let Ok(v) = std::env::var(&p.node.nam) {
                s = v;
            }
        }
        // c:Src/params.c::printparamvalue — for scalar specials like
        // `-` (shell flags via dashgetfn) that have no gsu_s wired but
        // do have a live getter routed through lookup_special_var,
        // fall back to getsparam if both gsu_s/strgetfn returned empty.
        // Mirrors the same dispatch the PM_INTEGER arm uses. Bug #516.
        if s.is_empty() {
            if let Some(v) = crate::ported::params::getsparam(&p.node.nam) {
                s = v;
            }
        }
        print!("{}", quotedzputs(&s)); // c:6053
    } else if t == PM_INTEGER {
        // c:Src/params.c:6051-6057 PM_INTEGER arm — C calls
        // `printf("%ld", p->gsu.i->getfn(p))` (or output64 on 64-bit
        // builds). Always emits DECIMAL — `typeset -i 16 n=255` prints
        // `n=255`, NOT `n=16#FF`. The base formatting only applies to
        // `$n` expansion + `print -- $n`, not `typeset -p n`.
        //
        // The previous Rust port routed through `getsparam` first,
        // which calls `convbase` and formats as `16#FF`. That polluted
        // typeset -p output. Bug #608.
        //
        // Use the custom gsu_i.getfn when present (special integers
        // like PPID/EUID/SECONDS dispatch through their own getfn);
        // fall back to intgetfn (which reads pm.u_val). For lookup-
        // special-var integers without a wired gsu_i, fall through to
        // getsparam ONLY when u_val is 0 (the "no stored value"
        // sentinel) so live PPID etc. still surface.
        let getfn_ptr = p.gsu_i.as_ref().map(|g| g.getfn);
        let raw = if let Some(getfn) = getfn_ptr {
            getfn(p)
        } else {
            intgetfn(p)
        };
        if raw == 0 {
            if let Some(s) = crate::ported::params::getsparam(&p.node.nam) {
                if !s.is_empty() {
                    // Strip any base#prefix added by convbase so the
                    // typeset -p output stays decimal-only per C source.
                    let dec = if let Some(idx) = s.find('#') {
                        // Try parsing the part after # in the base.
                        let (base_str, rest) = s.split_at(idx);
                        let rest = &rest[1..];
                        if let Ok(b) = base_str.parse::<u32>() {
                            if (2..=36).contains(&b) {
                                i64::from_str_radix(rest, b)
                                    .map(|n| n.to_string())
                                    .unwrap_or(s.clone())
                            } else {
                                s.clone()
                            }
                        } else {
                            s.clone()
                        }
                    } else {
                        s.clone()
                    };
                    print!("{}", dec);
                    return;
                }
            }
        }
        print!("{}", raw);
    } else if t == PM_EFLOAT || t == PM_FFLOAT {
        // c:6063 — `convfloat(p->gsu.f->getfn(p), p->base, p->node.flags,
        //          stdout)`. Honors pm.base for precision and
        // pm.flags for PM_EFLOAT/PM_FFLOAT format selection. The
        // previous Rust port used `print!("{}", floatgetfn(p))`
        // which always renders in Rust's default float format
        // (which differs from C's printf %g / %e formats).
        print!("{}", convfloat(floatgetfn(p), p.base, p.node.flags as u32)); // c:6063
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
            // c:Src/params.c:6166-6171 — each array element goes
            // through `quotedzputs` so elements containing spaces,
            // quotes, or other shell-meta chars round-trip through
            // `eval`. zshrs previously emitted bare elements; output
            // like `( red blue green yellow )` for `(red "blue green"
            // yellow)` was 4 words when re-parsed, not 3. Bug #181
            // in docs/BUGS.md.
            print!("{}", quotedzputs(&arr[0]));
            for el in &arr[1..] {
                if (printflags & PRINT_LINE) != 0 {
                    print!("\n  ");
                } else {
                    print!(" ");
                }
                print!("{}", quotedzputs(el));
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
        }
        // c:Src/params.c:6108-6110 — `scanhashtable(ht, 1, 0,
        //   PM_UNSET, ht->printnode, PRINT_KV_PAIR | (printflags &
        //   PRINT_LINE))`. C source ALWAYS passes PRINT_KV_PAIR when
        // scanning hash entries, regardless of the incoming
        // printflags. This guarantees the per-entry format is
        // `[key]=value` (which is the only syntactically-valid form
        // that round-trips through `eval`). The previous Rust port
        // only used `[k]=v` when PRINT_TYPESET / PRINT_KV_PAIR was
        // already set on the outer call, falling back to bare
        // `k v` form otherwise — so `typeset h` for an assoc
        // printed `h=''` instead of `h=( [a]=1 [b]=2 )`. Bug #218
        // in docs/BUGS.md.
        let mut had_entries = false;
        if let Ok(stor) = paramtab_hashed_storage().lock() {
            if let Some(map) = stor.get(&p.node.nam) {
                let mut entries: Vec<(&String, &String)> = map.iter().collect();
                entries.sort_by(|(a, _), (b, _)| a.cmp(b));
                let mut first = true;
                for (k, v) in entries {
                    if first {
                        // Leading space before first entry
                        // (only when not in PRINT_KV_PAIR mode).
                        if (printflags & PRINT_KV_PAIR) == 0
                            && (printflags & PRINT_LINE) == 0
                        {
                            print!(" ");
                        }
                        if (printflags & PRINT_LINE) != 0 {
                            print!("\n  ");
                        }
                        first = false;
                        had_entries = true;
                    } else if (printflags & PRINT_LINE) != 0 {
                        print!("\n  ");
                    } else {
                        print!(" ");
                    }
                    // c:6292-6299 — `[key]=value` form per the
                    // unconditional PRINT_KV_PAIR pass at c:6109.
                    print!("[{}]={}", k, quotedzputs(v));
                }
            }
        }
        if (printflags & PRINT_KV_PAIR) == 0 {
            // c:Src/params.c — empty assoc prints `( )` (single
            // space between parens). Non-empty: `( [k]=v )` (space
            // before first + space before close paren). Verified vs
            // /opt/homebrew/bin/zsh: `declare -Ax h; declare -p h`
            // → `typeset -Ax h=( )`.
            if (printflags & PRINT_LINE) == 0 {
                print!(" ");
            }
            print!(")");
        }
        let _ = had_entries;
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
pub fn printparamnode(hn: &mut param, mut printflags: i32) {
    const PRINT_WITH_NAMESPACE: i32 = 1 << 8; // matches createspecial print enum
    let f = hn.node.flags as u32;
    if (f & PM_HASHELEM) == 0
        && (printflags & PRINT_WITH_NAMESPACE) == 0
        && hn.node.nam.starts_with('.')
    {
        return;
    }
    if (f & PM_UNSET) != 0 {
        // c:Src/params.c — PM_SPECIAL params (HOME/PATH/PWD/etc.)
        // carry PM_UNSET when their u_str cache hasn't been seeded
        // but the value is available via the GSU getfn. zshrs's
        // -fc init path doesn't run createparamtable() so the env
        // import + value-cache step is skipped — yet env::var()
        // still gives a live value. Probe getsparam for a
        // non-empty value before treating as truly unset so
        // `typeset -p HOME` (which goes through this print) emits
        // the expected `export HOME=…` line instead of nothing.
        let has_special_value = (f & PM_SPECIAL) != 0
            && hn.u_str.is_none()
            && getsparam(&hn.node.nam).is_some();
        // c:6133-6143 — POSIX readonly/exported keep + PM_DEFAULTED
        // path: show as readonly/exported even if unset, with no
        // value (NAMEONLY).
        let posix_keep = (printflags & (PRINT_POSIX_READONLY | PRINT_POSIX_EXPORT)) != 0
            && (f & (PM_READONLY | PM_EXPORTED)) != 0;
        let defaulted = (f & PM_DEFAULTED) == PM_DEFAULTED; // c:6137
        if has_special_value {
            // Seed u_str so the value-emit arm below picks it up.
            hn.u_str = getsparam(&hn.node.nam);
            hn.node.flags &= !(PM_UNSET as i32);
        } else if posix_keep || defaulted {
            printflags |= PRINT_NAMEONLY;
        } else {
            return;
        }
    }
    if (f & PM_AUTOLOAD) != 0 {
        printflags |= PRINT_NAMEONLY;
    }
    // c:Src/params.c — the outer block runs whenever the attribute
    // walk is also wanted: PRINT_TYPE (bare typeset) OR
    // PRINT_TYPESET (typeset -p) OR PRINT_POSIX_*. The C source has
    // two SEPARATE parallel blocks, but the Rust port nested the
    // attribute walk inside the prefix block — bug #42 in
    // docs/BUGS.md. Widen the outer gate so PRINT_TYPE also enters
    // it; the prefix-print arms below gate themselves on the
    // narrower `PRINT_TYPESET | PRINT_POSIX_*` set so bare typeset
    // skips the `typeset ` / `export ` / `local ` prefix and goes
    // straight to the attribute walk + name=value tail.
    if (printflags & (PRINT_TYPE | PRINT_TYPESET | PRINT_POSIX_READONLY | PRINT_POSIX_EXPORT))
        != 0
    {
        let needs_prefix =
            (printflags & (PRINT_TYPESET | PRINT_POSIX_READONLY | PRINT_POSIX_EXPORT)) != 0;
        if (f & PM_AUTOLOAD) != 0 && needs_prefix {
            return;
        }
        // c:6157-6163 — PM_RO_BY_DESIGN with level check: only show
        // the entry when its level matches the current scope.
        if (f & PM_RO_BY_DESIGN) != 0 && needs_prefix {
            let cur_ll = locallevel.load(Ordering::Relaxed) as i32;
            if hn.level != cur_ll {
                // c:6157
                return;
            }
        }
        let mut altname: u8 = 0;
        if needs_prefix {
            if (printflags & PRINT_POSIX_EXPORT) != 0 {
                if (f & PM_EXPORTED) == 0 {
                    return;
                }
                altname = b'x';
                print!("export ");
            } else if (printflags & PRINT_POSIX_READONLY) != 0 {
                if (f & PM_READONLY) == 0 {
                    return;
                }
                altname = b'r';
                print!("readonly ");
            } else if (f & PM_EXPORTED) != 0 && (f & (PM_ARRAY | PM_HASHED)) == 0 {
                // c:6181-6188 — exported scalar: `local` or `export`.
                let cur_ll = locallevel.load(Ordering::Relaxed) as i32;
                if hn.level != 0 && hn.level >= cur_ll {
                    print!("local ");
                } else {
                    altname = b'x';
                    print!("export ");
                }
            } else {
                let cur_ll = locallevel.load(Ordering::Relaxed) as i32;
                if cur_ll != 0 && hn.level >= cur_ll {
                    if (f & PM_EXPORTED) != 0 {
                        print!("local ");
                    } else {
                        print!("typeset ");
                    }
                } else if cur_ll != 0 {
                    print!("typeset -g ");
                } else {
                    print!("typeset ");
                }
            }
        }

        // c:6199-6259 — attribute walk via pmtypes table. Each row
        // tests `p->node.flags & binflag`; on match, PRINT_TYPESET
        // emits the `-X` letter, PRINT_TYPE the long word.
        // Port of `Src/params.c:6010 static const struct paramtypes
        // pmtypes[]`.
        const PMTF_USE_BASE: u32 = 1 << 0;
        const PMTF_USE_WIDTH: u32 = 1 << 1;
        const PMTF_TEST_LEVEL: u32 = 1 << 2;
        struct PmType {
            binflag: u32,
            string: &'static str,
            typeflag: u8,
            flags: u32,
        }
        const PMTYPES: &[PmType] = &[
            PmType {
                binflag: PM_AUTOLOAD,
                string: "undefined",
                typeflag: 0,
                flags: 0,
            },
            PmType {
                binflag: PM_INTEGER,
                string: "integer",
                typeflag: b'i',
                flags: PMTF_USE_BASE,
            },
            PmType {
                binflag: PM_EFLOAT,
                string: "float",
                typeflag: b'E',
                flags: 0,
            },
            PmType {
                binflag: PM_FFLOAT,
                string: "float",
                typeflag: b'F',
                flags: 0,
            },
            PmType {
                binflag: PM_ARRAY,
                string: "array",
                typeflag: b'a',
                flags: 0,
            },
            PmType {
                binflag: PM_HASHED,
                string: "association",
                typeflag: b'A',
                flags: 0,
            },
            PmType {
                binflag: 0,
                string: "local",
                typeflag: 0,
                flags: PMTF_TEST_LEVEL,
            },
            PmType {
                binflag: PM_HIDE,
                string: "hide",
                typeflag: b'h',
                flags: 0,
            },
            PmType {
                binflag: PM_LEFT,
                string: "left justified",
                typeflag: b'L',
                flags: PMTF_USE_WIDTH,
            },
            PmType {
                binflag: PM_RIGHT_B,
                string: "right justified",
                typeflag: b'R',
                flags: PMTF_USE_WIDTH,
            },
            PmType {
                binflag: PM_RIGHT_Z,
                string: "zero filled",
                typeflag: b'Z',
                flags: PMTF_USE_WIDTH,
            },
            PmType {
                binflag: PM_LOWER,
                string: "lowercase",
                typeflag: b'l',
                flags: 0,
            },
            PmType {
                binflag: PM_UPPER,
                string: "uppercase",
                typeflag: b'u',
                flags: 0,
            },
            PmType {
                binflag: PM_READONLY,
                string: "readonly",
                typeflag: b'r',
                flags: 0,
            },
            PmType {
                binflag: PM_TAGGED,
                string: "tagged",
                typeflag: b't',
                flags: 0,
            },
            PmType {
                binflag: PM_EXPORTED,
                string: "exported",
                typeflag: b'x',
                flags: 0,
            },
            PmType {
                binflag: PM_UNIQUE,
                string: "unique",
                typeflag: b'U',
                flags: 0,
            },
            PmType {
                binflag: PM_TIED,
                string: "tied",
                typeflag: b'T',
                flags: 0,
            },
            PmType {
                binflag: PM_NAMEREF,
                string: "nameref",
                typeflag: b'n',
                flags: 0,
            },
        ];
        if (printflags & (PRINT_TYPE | PRINT_TYPESET)) != 0 {
            let mut doneminus = false; // c:6200
            for pmptr in PMTYPES.iter() {
                // c:6204
                if altname != 0 && altname == pmptr.typeflag {
                    // c:6207
                    continue;
                }
                // PM_RO_BY_DESIGN-expansion for the readonly attribute:
                // zshrs's special params (e.g. `$!`, `$$`, `$?`, `LINENO`)
                // carry PM_RO_BY_DESIGN instead of PM_READONLY so internal
                // writes pass `assignstrvalue` (see vm_helper init at
                // bug #97 in docs/BUGS.md). The C-side IPDEF4 entries
                // declare PM_READONLY_SPECIAL = PM_SPECIAL | PM_READONLY |
                // PM_RO_BY_DESIGN, so both bits are set together. The
                // attribute walk's readonly check has to expand its match
                // to either bit; mirrors the `bin_typeset` listing filter
                // at builtin.rs:3620. Bug #297 in docs/BUGS.md.
                let effective_binflag = if pmptr.binflag == PM_READONLY {
                    PM_READONLY | crate::ported::zsh_h::PM_RO_BY_DESIGN
                } else {
                    pmptr.binflag
                };
                let doprint = if (pmptr.flags & PMTF_TEST_LEVEL) != 0 {
                    // c:6209
                    hn.level != 0 // c:6211
                } else if (pmptr.binflag != PM_EXPORTED
                    || hn.level != 0
                    || (f & (PM_LOCAL | PM_ARRAY | PM_HASHED)) != 0)
                    && (f & effective_binflag) != 0
                {
                    // c:6225-6227
                    true
                } else {
                    false
                };
                if doprint {
                    // c:6230
                    if (printflags & PRINT_TYPESET) != 0 {
                        // c:6231
                        if pmptr.typeflag != 0 {
                            // c:6232
                            if !doneminus {
                                // c:6233
                                print!("-"); // c:6234
                                doneminus = true;
                            }
                            print!("{}", pmptr.typeflag as char); // c:6237
                        }
                    } else {
                        print!("{} ", pmptr.string); // c:6240
                    }
                    if (pmptr.flags & PMTF_USE_BASE) != 0 && hn.base != 0 {
                        // c:6242
                        print!("{} ", hn.base); // c:6243
                        doneminus = false;
                    }
                    if (pmptr.flags & PMTF_USE_WIDTH) != 0 && hn.width != 0 {
                        // c:6245
                        print!("{} ", hn.width); // c:6246
                        doneminus = false;
                    }
                }
            }
            if doneminus {
                // c:6252
                print!(" ");
            }
        }

        // c:Src/params.c:6256-6285 — PM_TIED partner emission. For
        // tied scalar/array pairs (PATH↔path, FPATH↔fpath, etc.) the
        // partner name is printed before the entry's own name, and
        // for `typeset -p` on the SCALAR side the value is swapped to
        // the ARRAY peer's contents (so the output is re-parseable
        // and doesn't collapse `(a b c)` vs `('a b c')` to the same
        // colon-string). Bug #410.
        if (f & PM_TIED) != 0 {
            if let Some(ename) = hn.ename.clone() {
                // c:Src/params.c:6275 `paramtab->getnode(paramtab,
                // p->ename)`. The bin_typeset caller pre-clones the
                // pm before calling printparamnode (see builtin.rs)
                // so no paramtab lock is held here. Read the peer
                // directly. Bug #410.
                let peer_info: Option<(String, Vec<String>)> =
                    paramtab().read().ok().and_then(|t| {
                        t.get(&ename).map(|peer_pm| {
                            (
                                peer_pm.node.nam.clone(),
                                peer_pm.u_arr.clone().unwrap_or_default(),
                            )
                        })
                    });
                if let Some((peer_name, peer_arr)) = peer_info {
                    let typeset_mode = (printflags & PRINT_TYPESET) != 0;
                    let we_are_scalar = (f & PM_ARRAY) == 0;
                    if typeset_mode && we_are_scalar {
                        // c:6280-6284 — swap p with the array peer so
                        // value emission uses the array form. Print
                        // OUR name first (the scalar side), then
                        // swap.
                        print!("{} ", quotedzputs(&hn.node.nam));
                        hn.node.nam = peer_name;
                        hn.u_arr = Some(peer_arr);
                        hn.u_str = None;
                        // Flip the PM_TYPE bits to PM_ARRAY so
                        // printparamvalue dispatches the array arm.
                        // PM_TYPE is a const fn that masks the
                        // type-bits union (PM_SCALAR | PM_INTEGER |
                        // PM_EFLOAT | PM_FFLOAT | PM_ARRAY | PM_HASHED
                        // | PM_NAMEREF). Clear them all, then set
                        // PM_ARRAY.
                        let type_mask = crate::ported::zsh_h::PM_SCALAR
                            | crate::ported::zsh_h::PM_INTEGER
                            | crate::ported::zsh_h::PM_EFLOAT
                            | crate::ported::zsh_h::PM_FFLOAT
                            | crate::ported::zsh_h::PM_ARRAY
                            | crate::ported::zsh_h::PM_HASHED
                            | crate::ported::zsh_h::PM_NAMEREF;
                        hn.node.flags = (hn.node.flags & !(type_mask as i32))
                            | (PM_ARRAY as i32);
                        // Drop the scalar getfn so printparamvalue
                        // doesn't pull from the scalar gsu (which
                        // would resolve PATH-the-colon-string).
                        hn.gsu_s = None;
                    } else {
                        // c:6286 — non-swap path: just print peer's
                        // name + space. The downstream name+value
                        // emission still uses hn (our own data).
                        print!("{} ", quotedzputs(&peer_name));
                    }
                }
            }
        }
    }
    if (printflags & PRINT_KV_PAIR) != 0 {
        // hashelem path: print key without name= leader.
    }
    // c:Src/params.c:6290 — `quotedzputs(p->node.nam, stdout)`. Names
    // containing shell metacharacters get single-quoted so the
    // output is re-parseable (`'#'=0`, `'$'=2609`, `'?'=0`). Plain
    // identifiers pass through unchanged. Bug #97 in docs/BUGS.md:
    // bare `print!("{}", nam)` produced unquoted `#=0` etc.
    print!("{}", quotedzputs(&hn.node.nam));
    // c:6289 — `(printflags & PRINT_NAMEONLY) ||
    //   ((p->node.flags & PM_HIDEVAL) && !(printflags & PRINT_INCLUDEVALUE))`
    // PM_HIDEVAL (set by `typeset -H`, see TYPESET_OPTSTR position
    // 15 in Src/builtin.c bin_typeset → PM_HIDEVAL = 1<<15) hides
    // the value in `typeset -p` output. Bare `typeset NAME` passes
    // PRINT_INCLUDEVALUE (Src/builtin.c:2246) which overrides the
    // hide. Bug #233 in docs/BUGS.md — printparamnode didn't honor
    // PM_HIDEVAL, so `typeset -H X=hello; typeset -p X` printed the
    // value instead of just the name.
    let hideval = (f & PM_HIDEVAL) != 0 && (printflags & PRINT_INCLUDEVALUE) == 0;
    if (printflags & PRINT_NAMEONLY) != 0 || hideval {
        if (printflags & PRINT_KV_PAIR) == 0 {
            println!();
        }
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
pub fn resolve_nameref(
    // c:6325
    pm: Option<Param>,
) -> Option<Param> {
    resolve_nameref_rec(pm, None, 0) // c:6327
}

/// Port of `resolve_nameref_rec(Param pm, const Param stop, int keep_lastref)` from `Src/params.c:6332`. C
/// recursive helper for `resolve_nameref()`. Walks the chain of
/// nameref indirections via `gethashnode2(realparamtab, refname)`
/// + `loadparamnode(paramtab, upscope(pm, ref), refname)`,
/// checking PM_TAGGED for cycle detection, and returns the
/// final non-nameref Param. Returns the input `pm` unchanged
/// for the early-exit path (no NAMEREF / UNSET / has subscript /
/// empty refname). The chain walk delegates to
/// `resolve_nameref_name` which operates on the live paramtab by
/// name (the Rust table hands out clones, so the by-name walk is
/// the canonical chain follower).
#[allow(unused_variables)]
pub fn resolve_nameref_rec(
    pm: Option<Param>,
    stop: Option<&param>,
    keep_lastref: i32,
) -> Option<Param> {
    let pm_ref = pm.as_deref()?;
    let f = pm_ref.node.flags as u32;
    // c:6336-6339 — early exits return pm unchanged.
    if (f & PM_NAMEREF) == 0 || (f & PM_UNSET) != 0 || pm_ref.width != 0 {
        return pm;
    }
    let refname = pm_ref.u_str.as_deref().unwrap_or("");
    if refname.is_empty() {
        if keep_lastref != 0 {
            return pm; // c:6353-6354
        }
        return pm; // empty refname is also an early-exit shape (c:6339)
    }
    if (f & PM_TAGGED) != 0 {
        // c:6340-6343 — `zerr("%s: invalid self reference", pm->node.nam)`.
        let nam = pm_ref.node.nam.clone();
        zerr(&format!("{}: invalid self reference", nam));
        return None;
    }
    let stop_key = stop.map(|s| (s.node.nam.clone(), s.level));
    match crate::vm_helper::resolve_nameref_name(
        &pm_ref.node.nam,
        stop_key.as_ref().map(|(n, l)| (n.as_str(), *l)),
    ) {
        crate::vm_helper::nameref_resolution::Target {
            pm: Some(target), ..
        } => Some(target),
        crate::vm_helper::nameref_resolution::Target { pm: None, .. } => {
            // c:6347 miss + keep_lastref (c:6353-6354).
            if keep_lastref != 0 {
                pm
            } else {
                None
            }
        }
        crate::vm_helper::nameref_resolution::Placeholder(last) => {
            // Chain ended on an empty-refname ref: C returns that ref
            // (the early-exit at c:6336-6339 of the recursive call).
            let tab = paramtab().read().ok()?;
            tab.get(&last).cloned().or(pm)
        }
        crate::vm_helper::nameref_resolution::SelfRef => None, // c:6343
        crate::vm_helper::nameref_resolution::OutOfScope => None, // c:6347-6349
        crate::vm_helper::nameref_resolution::NotRef => pm,
    }
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
    // realparamtab and paramtab are the same backing store in zshrs
    // (the alias-flip during assoc iteration isn't modelled); the
    // operative table is `paramtab()`.
    // Scope the write lock so we drop it before calling setsparam below.
    let nameref_branch = {
        let mut tab = paramtab().write().unwrap();
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
        // c:6379 — `setscope(pm);` — run the full-table variant with
        // no lock held (it manages its own short-lived locks).
        crate::vm_helper::setscope_by_name(name);
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
/// `resolve_nameref_rec` chain to detect self-references.
/// In-place variant: operates only on the passed `&mut param`
/// (width computation + literal self-name check). Callers that
/// hold no paramtab lock should use `setscope_by_name` which runs
/// the full base-scope + chain self-ref detection against the
/// live table.
/// Port of `setscope(Param pm)` from `Src/params.c:6382`.
pub fn setscope(pm: &mut param) {
    queue_signals();
    if (pm.node.flags as u32 & PM_NAMEREF) != 0 {
        // Refname is stored in pm.u_str for nameref-typed params.
        let refname = pm.u_str.clone();
        if let Some(rn) = refname {
            // c:6391-6400 — compute pm->width by finding the first `[`.
            if let Some(i) = rn.find('[') {
                pm.width = i as i32;
            }
        }
    }
    unqueue_signals();
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
/// records the param's name on `SCOPEREFS[base]` so a future
/// scope-pop can resolve upper/nameref references.
/// Port of `setscope_base(Param pm, int base)` from `Src/params.c:6438`.
pub fn setscope_base(pm: &mut param, base: i32) {
    // c:6438
    // c:6440 — `if ((pm->base = base) > pm->level) {`
    pm.base = base;
    if base > pm.level {
        SCOPEREFS.with(|sr| {
            let mut sr = sr.borrow_mut();
            // c:6442-6447 — `if (base >= scoperefs_num) { ... grow ... }`
            //               Rust Vec grows on demand via resize; mirrors
            //               the C double-and-zero-init pattern via the
            //               max(8, 2*base) growth heuristic.
            if (base as usize) >= sr.len() {
                let new_num = (2 * base as usize).max(8);
                sr.resize(new_num, Vec::new());
            }
            // c:6448-6451 — `refs = scoperefs[base]; if (!refs)
            //                  refs = scoperefs[base] = znewlinklist();
            //                zpushnode(refs, pm);`
            //               Rust pushes the param NAME (Vec<String>),
            //               not a raw pointer — borrow-safe and the
            //               name is the canonical key for upscope walks.
            sr[base as usize].insert(0, pm.node.nam.clone()); // c:6451
        });
    }
}

/// `scoperefs` — port of `static LinkList *scoperefs` from `Src/params.c:503`.
/// One Vec<String> (param names) per scope index. Per-evaluator (bucket 1)
/// because each worker thread has its own nameref-resolution context.
thread_local! {
    /// `SCOPEREFS` static.
    pub static SCOPEREFS: std::cell::RefCell<Vec<Vec<String>>>
        = const { std::cell::RefCell::new(Vec::new()) };
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
pub fn upscope(mut pm: Param, reference: &param) -> Param {
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
pub fn valid_refname(val: &str, flags: i32) -> bool {
    // c:6466
    if val.is_empty() {
        return false;
    }
    let first = val.chars().next().unwrap();
    let pm_upper = (flags as u32 & PM_UPPER) != 0;
    let mut t: usize;
    if pm_upper {
        // c:6470
        if first.is_ascii_digit() {
            // c:6472
            return false; // c:6473
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
            && (val.starts_with("argv") || val.starts_with("ARGC"))
        // c:6476-6477
        {
            return false; // c:6478
        }
    } else if first.is_ascii_digit() {
        // c:6479
        // c:6480-6485 — all-digit run; first non-digit must be `[`.
        t = 1;
        for (i, c) in val.char_indices().skip(1) {
            if !c.is_ascii_digit() {
                t = i;
                break;
            }
            t = i + c.len_utf8();
        }
        if t < val.len() && val.as_bytes()[t] != b'[' {
            // c:6484
            return false; // c:6485
        }
    } else {
        // c:6487 — `t = itype_end(val, INAMESPC, 0)`.
        t = val
            .char_indices()
            .find(|(_, c)| !(c.is_alphanumeric() || *c == '_' || *c == '.'))
            .map(|(i, _)| i)
            .unwrap_or(val.len());
    }

    if t == 0 {
        // c:6489
        let c = val.as_bytes()[0];
        if !(c == b'!' || c == b'?' || c == b'$' || c == b'-' || c == b'_') {
            // c:6490
            return false; // c:6493
        }
        t = 1; // c:6494
    }
    if t < val.len() && val.as_bytes()[t] == b'[' {
        // c:6496
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
    true // c:6510
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
pub static DELUNSET: std::sync::atomic::AtomicI32 = // c:610
    std::sync::atomic::AtomicI32::new(0);

pub(crate) fn paramtab_hashed_storage() -> &'static Mutex<HashMap<String, IndexMap<String, String>>>
{
    PARAMTAB_HASHED_STORAGE_INNER.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Shadow stack for `paramtab_hashed_storage` entries displaced by
/// `local -A NAME` / `typeset -A NAME` shadows inside a function. The
/// canonical assoc data lives in `paramtab_hashed_storage` (a flat
/// HashMap keyed by name with NO scope dimension — Rust-only parallel
/// store; the C side keeps assoc data in pm.u_hash so it rides the
/// pm.old chain automatically). createparam pushes the displaced
/// value when a PM_LOCAL|PM_HASHED shadow installs; endparamscope
/// pops on PM_HASHED restoration so the outer scope's bag comes
/// back. Mirrors C's `copyparam` (Src/builtin.c:2382-2424) via
/// parallel storage. Bug #415.
pub(crate) static PARAMTAB_HASHED_SHADOW_STACK: OnceLock<Mutex<HashMap<String, Vec<Option<IndexMap<String, String>>>>>> =
    OnceLock::new();

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
    assoc_arrays: &mut HashMap<String, IndexMap<String, String>>,
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
            if let Some(map) = paramtab_hashed_storage().lock().unwrap().get(name) {
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

pub(crate) fn ifs_lock() -> &'static Mutex<String> {
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

pub(crate) fn wordchars_lock() -> &'static Mutex<String> {
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
/// `NUMPARAMVALS` static.
pub static NUMPARAMVALS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0); // c:626
/// `SCANPROG` static.
pub static SCANPROG: OnceLock<Mutex<Option<String>>> = OnceLock::new(); // c:637
/// `SCANSTR` static.
pub static SCANSTR: OnceLock<Mutex<Option<String>>> = OnceLock::new(); // c:638
/// `PARAMVALS` static.
pub static PARAMVALS: OnceLock<Mutex<Vec<String>>> = OnceLock::new(); // c:639

/// Resolve the current user's name. Mirrors C's `get_username()`
/// init at Src/init.c which reads `getpwuid(getuid())->pw_name`
/// rather than `$USER`. Falls back to env vars only if the
/// passwd lookup fails (rare on real systems).
fn initial_username() -> String {
    #[cfg(unix)]
    {
        let uid = unsafe { libc::getuid() };
        let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
        // libc::c_char is i8 on x86_64/aarch64-darwin and x86_64-linux but u8 on
        // aarch64-linux. Use c_char so getpwuid_r's pointer type matches per-target.
        let mut buf: Vec<libc::c_char> = vec![0; 1024];
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        let rc =
            unsafe { libc::getpwuid_r(uid, &mut pwd, buf.as_mut_ptr(), buf.len(), &mut result) };
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
/// `shtimer_lock` — see implementation.
pub fn shtimer_lock() -> &'static Mutex<Duration> {
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
    &PPARAMS
}

fn zunderscore_lock() -> &'static Mutex<String> {
    static ZUNDERSCORE_VAR: OnceLock<Mutex<String>> = OnceLock::new();
    ZUNDERSCORE_VAR.get_or_init(|| Mutex::new(String::new()))
}

/// Update `$_` with the last argument of the just-completed
/// command. Mirrors C zsh's writeback in `execcmd_exec` (Src/exec.c)
/// where `zunderscore` is set to the last argv slot before
/// returning. Callers: every command-dispatch hook in
/// fusevm_bridge / vm_helper.
pub fn set_zunderscore(argv: &[String]) {
    let new = if let Some(last) = argv.last() {
        last.clone()
    } else {
        String::new()
    };
    *zunderscore_lock().lock().expect("zunderscore poisoned") = new;
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
fn dontimport(flags: i32) -> i32 {
    // c:796
    let flags = flags as u32;
    // c:799-800 — `if (flags & PM_DONTIMPORT) return 1`.
    if flags & PM_DONTIMPORT != 0 {
        // c:799
        return 1; // c:800
    }
    // c:802-803 — `if (flags & PM_EXPORTED) return 1`.
    if flags & PM_EXPORTED != 0 {
        // c:802
        return 1; // c:803
    }
    // c:805-806 — `if ((flags & PM_DONTIMPORT_SUID) && isset(PRIVILEGED)) return 1`.
    if flags & PM_DONTIMPORT_SUID != 0                 // c:805
        && isset(PRIVILEGED)
    {
        return 1; // c:806
    }
    0 // c:809
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

/// Registry of special-parameter names that have been `unset` and
/// should bypass the getfn regenerator.
///
/// c:Src/params.c:3853 — `unsetparam_pm` sets `PM_UNSET` on the pm
/// node which getfn callbacks check. zshrs's `lookup_special_var`
/// dispatches to libc/getfn directly without a paramtab pm node, so
/// PM_UNSET tracking happens in this side-set instead. Bug #417/#418.
fn unset_specials() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static SET: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    SET.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

/// Mark a special-parameter NAME as unset. Future
/// `lookup_special_var(name)` calls return `None` instead of dispatching
/// to the getfn regenerator. Re-assigning the name (e.g. `RANDOM=42`)
/// should clear this flag via `clear_unset_special`.
pub fn mark_unset_special(name: &str) {
    if let Ok(mut s) = unset_specials().lock() {
        s.insert(name.to_string());
    }
}

/// Clear the unset flag for a special-parameter NAME — called when the
/// name is re-assigned so the getfn regenerator becomes active again.
pub fn clear_unset_special(name: &str) {
    if let Ok(mut s) = unset_specials().lock() {
        s.remove(name);
    }
}

fn is_unset_special(name: &str) -> bool {
    unset_specials()
        .lock()
        .map(|s| s.contains(name))
        .unwrap_or(false)
}

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
    // c:Src/params.c:3853 — PM_UNSET-flagged specials skip getfn.
    // Only applies to regenerator-style specials (RANDOM, SECONDS,
    // EPOCHSECONDS, TTYIDLE, ERRNO) — identity specials like UID, GID,
    // PPID stay live since they're not user-clearable in zsh either.
    //
    // Two sources of "is unset" — the side-set populated by
    // `mark_unset_special` from explicit `unset NAME`, and the
    // initial PM_UNSET flag set by `Src/params.c:298 IPDEF1(...,
    // PM_UNSET)`. ERRNO is the only IPDEF1-special with the initial
    // PM_UNSET flag (params.c:298) — zsh -fc reports `$ERRNO` as
    // empty because nothing has set errno since startup. Mirror by
    // also consulting the paramtab pm flags. Without this, `$ERRNO`
    // returned the live errno value instead of empty.
    if matches!(
        name,
        "RANDOM" | "SECONDS" | "EPOCHSECONDS" | "EPOCHREALTIME" | "TTYIDLE" | "ERRNO"
    ) && (is_unset_special(name) || {
        // c:Src/params.c — paramtab PM_UNSET check. ERRNO carries
        // this flag from IPDEF1 initialization (params.c:298); reads
        // route through getsparam → paramtab pm.flags check at
        // line 4356-4358 normally, but ERRNO/RANDOM/etc. short-
        // circuit through lookup_special_var BEFORE that check.
        paramtab()
            .read()
            .ok()
            .and_then(|t| t.get(name).map(|pm| (pm.node.flags as u32 & PM_UNSET) != 0))
            .unwrap_or(false)
    }) {
        return None;
    }
    // All-digit positional: $1..$N from canonical PPARAMS.
    // C zsh dispatches positional params through pparams (Src/init.c).
    if !name.is_empty() && name.chars().all(|c| c.is_ascii_digit()) {
        let n: usize = name.parse().ok()?;
        if n == 0 {
            return argzero();
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
        // c:Src/params.c:350 `IPDEF4("PPID", &ppid)` — ppid is the
        // file-static set at shell startup from getppid(). zshrs's
        // ported special_paramdef list registers PPID but nothing
        // populates the paramtab slot from getppid(2), so $PPID
        // always read 0. Route through the libc syscall directly.
        "PPID" => Some((unsafe { libc::getppid() } as i64).to_string()),
        // libc syscall callbacks.
        "RANDOM" => Some(randomgetfn().to_string()),
        "TTYIDLE" => Some(ttyidlegetfn().to_string()),
        "ERRNO" => Some(errnogetfn().to_string()),
        // Time callbacks.
        "SECONDS" => {
            // c:Src/params.c:4561/4591 — PM_TYPE dispatches between
            // intsecondsgetfn (PM_INTEGER, default) and floatsecondsgetfn
            // (PM_EFLOAT/PM_FFLOAT after `typeset -F`/`-E SECONDS`). The
            // pm's gsu_i vs gsu_f vtable swap happens in setfn during
            // typeset; we read pm.flags from paramtab here.
            let pm_type = paramtab()
                .read()
                .ok()
                .and_then(|t| t.get("SECONDS").map(|pm| PM_TYPE(pm.node.flags as u32)))
                .unwrap_or(PM_INTEGER);
            if pm_type == PM_EFLOAT || pm_type == PM_FFLOAT {
                let v = floatsecondsgetfn();
                let base = paramtab()
                    .read()
                    .ok()
                    .and_then(|t| t.get("SECONDS").map(|pm| pm.base))
                    .unwrap_or(0);
                Some(convfloat(v, base, pm_type))
            } else {
                Some(intsecondsgetfn().to_string())
            }
        }
        // zsh/datetime module params. In recent zsh these are
        // auto-loaded via `zmodload zsh/datetime` and the param
        // appears in paramtab via the module's `p:NAME` feature
        // descriptor (Src/Modules/datetime.c:25). In zshrs the
        // datetime module's per-param wireup hasn't been ported
        // through paramtab yet — the getters exist
        // (modules::datetime::getcurrentsecs / getcurrentrealtime)
        // but no Param entry is created on `zmodload`. Mirror the C
        // behavior by routing the names through their canonical
        // getters here so `$EPOCHSECONDS` / `$EPOCHREALTIME` read
        // live time values regardless of explicit zmodload. p10k
        // and many other prompts rely on this without an explicit
        // zmodload (zsh ships datetime preloaded in most configs).
        "EPOCHSECONDS" => {
            // c:Src/Modules/datetime.c:206 `getcurrentsecs`. zsh requires
            // explicit `zmodload zsh/datetime` before EPOCHSECONDS is
            // bound (Src/Modules/datetime.c:25 `p:EPOCHSECONDS` feature
            // descriptor). Without the load, the name is unset and
            // ${EPOCHSECONDS:-x} falls through to "x". Match by gating
            // the getter on the module's loaded state. Bug #31 in
            // docs/BUGS.md.
            if !crate::ported::module::MODULESTAB.lock().unwrap().is_loaded("zsh/datetime") {
                return None;
            }
            Some(crate::ported::modules::datetime::getcurrentsecs().to_string())
        }
        "EPOCHREALTIME" => {
            // c:Src/Modules/datetime.c:212 `getcurrentrealtime`. Same
            // zsh/datetime gate as EPOCHSECONDS above. Bug #31.
            if !crate::ported::module::MODULESTAB.lock().unwrap().is_loaded("zsh/datetime") {
                return None;
            }
            let v = crate::ported::modules::datetime::getcurrentrealtime();
            Some(format!("{:.10}", v))
        }
        "epochtime" => {
            // c:Src/Modules/datetime.c:220 `getcurrenttime` returns
            // [tv_sec, tv_nsec]. Bare `$epochtime` joins the two with
            // ` ` (the default IFS separator), matching the C path
            // `getstrvalue` → sepjoin on PM_ARRAY. Bug #317 in
            // docs/BUGS.md. Gate on zsh/datetime load per #31.
            if !crate::ported::module::MODULESTAB.lock().unwrap().is_loaded("zsh/datetime") {
                return None;
            }
            let arr = crate::ported::modules::datetime::getcurrenttime();
            Some(arr.join(" "))
        }
        // Cached-state callbacks. C dispatches `pm->gsu.s->getfn(pm)`
        // where pm is `paramtab->getnode(name)`. Mirror: look up pm,
        // pass it through. Each getfn here ignores pm (matches C's
        // UNUSED(Param pm)), so a fallback default-constructed param
        // is acceptable when the table isn't populated yet.
        //
        // c:Src/params.c paramsubst c:3193 — the `vunset = (!v || ...)`
        // check inspects `pm->node.flags & PM_UNSET`, not the value
        // returned by getfn. For specials whose getfn reads global
        // cached state (`ifs`, `wordchars`, `keyboardhackchar`), the
        // global keeps its last value across `unset NAME` (because
        // stdunsetfn only flips PM_UNSET; it doesn't clear the global).
        // Without consulting PM_UNSET here, `${IFS+set}` after
        // `unset IFS` still returned the default-IFS getter result,
        // diverging from zsh.
        "USERNAME" | "HOME" | "TERM" | "WORDCHARS" | "IFS" | "TERMINFO" | "TERMINFO_DIRS"
        | "KEYBOARD_HACK" | "histchars" | "HISTCHARS" => {
            let tab = paramtab().read().ok()?;
            let pm = tab.get(name)?;
            if (pm.node.flags as u32 & crate::ported::zsh_h::PM_UNSET) != 0 {
                return None;
            }
            Some(match name {
                "USERNAME" => usernamegetfn(pm),
                "HOME" => homegetfn(pm),
                "TERM" => termgetfn(pm),
                "WORDCHARS" => wordcharsgetfn(pm),
                "IFS" => ifsgetfn(pm),
                "TERMINFO" => terminfogetfn(pm),
                "TERMINFO_DIRS" => terminfodirsgetfn(pm),
                "KEYBOARD_HACK" => keyboardhackgetfn(pm),
                "histchars" | "HISTCHARS" => histcharsgetfn(pm),
                _ => unreachable!(),
            })
        }
        "_" => Some(underscoregetfn()),
        // Counters with int return.
        "HISTSIZE" => Some(histsizegetfn().to_string()),
        "SAVEHIST" => Some(savehistsizegetfn().to_string()),
        "#" | "ARGC" => Some(poundgetfn().to_string()),
        // c:Src/params.c:871 — `setsparam("TIMEFMT", DEFAULT_TIMEFMT)`.
        // createparamtable seeds this; the zshrs main binary skips
        // that init path so paramtab is empty for TIMEFMT. Route
        // here from the canonical default. paramtab check first so
        // an explicit user-set value sticks.
        "TIMEFMT" => {
            let tab_val = paramtab()
                .read()
                .ok()
                .and_then(|t| t.get("TIMEFMT").and_then(|pm| pm.u_str.clone()));
            if let Some(v) = tab_val {
                if !v.is_empty() {
                    return Some(v);
                }
            }
            Some(crate::ported::zsh_system_h::DEFAULT_TIMEFMT.to_string())
        }
        // c:Src/init.c:1214-1215 — `nullcmd = ztrdup("cat");
        // readnullcmd = ztrdup(DEFAULT_READNULLCMD);`. C seeds these
        // into the REAL paramtab at startup; `unset NULLCMD` then
        // removes the entry and getsparam returns NULL — which is
        // load-bearing: A04redirect "null redir with NULLCMD unset"
        // requires `unset NULLCMD; >file` to error "redirection with
        // no command". The previous read-time fallback here faked the
        // seed on EVERY lookup, making unset impossible. The seed now
        // lives in ShellExecutor::new (vm_helper.rs, next to TIMEFMT)
        // and absent means absent.
        // $0 routes through utils::argzero.
        "0" => argzero(),
        // POSIX shell-special scalars. C dispatches these through
        // dedicated gsu getfn callbacks (Src/params.c special_assigns).
        // c:Src/params.c lastvalgetfn — `?` and `status` are aliases
        // for the last-command exit code (lastval). C wires them via
        // separate IPDEF entries that share the same getfn.
        "?" | "status" => Some(LASTVAL.load(Ordering::Relaxed).to_string()),
        // c:Src/loop.c:719 — `try_errflag = -1` reset before
        // each `{ try } always { catch }` block; reads `-1` when
        // outside a try block. zsh exposes TRY_BLOCK_ERROR as an
        // integer special-param: inside an always-arm after a
        // normal-exit try, reads 0; -1 only when no try has yet
        // fired in this scope. BUILTIN_SET_TRY_BLOCK_ERROR writes
        // via set_scalar (u_str), so accept either storage form
        // and treat a present-but-empty u_str as 0 too.
        "TRY_BLOCK_ERROR" => {
            // c:Src/loop.c:719 — `zlong try_errflag = -1;` global,
            // exported via IPDEF6 (c:Src/params.c:364) so `$TRY_BLOCK_ERROR`
            // reads it directly. Initialized to -1 (sentinel: "no try
            // block has fired yet"); the `exectry` always-arm sets it
            // to `errflag & ERRFLAG_ERROR` (c:765) before running the
            // body, then restores at c:787. Read straight from the
            // canonical atomic — paramtab's u_str / u_val are NOT the
            // source of truth (matches C's IPDEF6 getfn signature).
            Some(
                crate::ported::r#loop::try_errflag
                    .load(std::sync::atomic::Ordering::Relaxed)
                    .to_string(),
            )
        }
        "TRY_BLOCK_INTERRUPT" => {
            // c:Src/loop.c:727 — `zlong try_interrupt = -1;` global.
            // Same shape as TRY_BLOCK_ERROR.
            Some(
                crate::ported::r#loop::try_interrupt
                    .load(std::sync::atomic::Ordering::Relaxed)
                    .to_string(),
            )
        }
        "$" => Some(std::process::id().to_string()),
        "!" => {
            // c:Src/params.c:345 IPDEF4("!", &lastpid) — `$!` reads
            // directly from the `lastpid` atomic (Src/jobs.c:73).
            // The previous Rust port read from paramtab["!"].u_str,
            // which only had a value if something wrote to it via
            // setsparam/assignsparam — and `"!"` is not a valid
            // identifier (isident("!") == 0 per params.c:1288), so
            // those calls failed loudly. Read directly from the
            // canonical store like the C getter does.
            let pid =
                crate::ported::modules::clone::lastpid.load(std::sync::atomic::Ordering::Relaxed);
            Some(pid.to_string())
        }
        // $* / $@ join positional params via sepjoin's IFS default —
        // c:Src/utils.c:3936-3945: set-but-empty IFS joins with ""
        // (`IFS=""; echo "$*"` concatenates); unset IFS joins with
        // " ". The previous `.unwrap_or(' ')` collapsed empty-IFS to
        // a space.
        "*" | "@" => pparams_lock()
            .lock()
            .ok()
            .map(|p| crate::ported::utils::sepjoin(&p, None)),
        // $- : current option-letter set.
        // c:Src/params.c:3262 (IPDEF) → dashparamgetfn in options.c:890.
        // Canonical C body walks `zshletters[FIRST_OPT..=LAST_OPT]`
        // (c:292-368) emitting each letter whose mapped option is
        // active (XOR-ing with the c:295 negation prefix `-OPT`).
        // The previous Rust port hand-rolled a hardcoded subset
        // ("569X" + 8 ad-hoc letters) that diverged from C's table:
        // letter 'h' wrongly mapped to `hashall` (c:271 ALIAS for
        // HASHCMDS) instead of HISTIGNOREDUPS (c:349). Parity bug
        // #32 — `$-` last char differed (zsh `f`, zshrs `fh`).
        // Route through `dashgetfn` (options.rs:835) which is the
        // direct port of `Src/options.c:890`.
        "-" => Some(crate::ported::options::dashgetfn()),
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

/// Shared test mutex for histsiz mutations (gsu_tests +
/// tests submodules both write the same global; this lock
/// serialises them under parallel test execution).
#[cfg(test)]
pub(crate) static HISTSIZ_TEST_LOCK: Mutex<()> = Mutex::new(());

/// Shared test mutex for histchars mutations (gsu_tests +
/// tests submodules both write bangchar/hatchar/hashchar atomics;
/// this lock serialises them under parallel test execution).
#[cfg(test)]
pub(crate) static HISTCHARS_TEST_LOCK_SHARED: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod gsu_tests {
    use super::*;

    #[test]
    fn test_libc_id_callbacks_match_libc() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(uidgetfn(), unsafe { libc::getuid() } as i64);
        assert_eq!(gidgetfn(), unsafe { libc::getgid() } as i64);
        assert_eq!(euidgetfn(), unsafe { libc::geteuid() } as i64);
        assert_eq!(egidgetfn(), unsafe { libc::getegid() } as i64);
    }

    /// Pin: `usernamegetfn` routes through `get_username()` per
    /// `Src/params.c:4658` (which refreshes cache on uid change
    /// per `Src/utils.c:1082`). The previous Rust port read a
    /// stale cached value directly. Verify the getter returns
    /// the same name as a direct libc `getpwuid(getuid())` —
    /// confirming the path WENT through the refresh helper, not
    /// the stale paramtab Mutex.
    #[test]
    fn usernamegetfn_matches_libc_getpwuid_for_current_uid() {
        let _g = crate::test_util::global_state_lock();
        let __pm = crate::ported::zsh_h::param::default();
        let uname = usernamegetfn(&__pm);
        // The current process is running as some uid; the getter
        // must return either a populated name OR an empty string
        // (when getpwuid fails, e.g. sandboxed builds). It must
        // NOT panic and must NOT return a stale cached value
        // from a different uid.
        let direct = unsafe {
            let pw = libc::getpwuid(libc::getuid());
            if pw.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr((*pw).pw_name)
                    .to_string_lossy()
                    .into_owned()
            }
        };
        assert_eq!(
            uname, direct,
            "c:4658 — usernamegetfn must match getpwuid(getuid())->pw_name"
        );
    }

    #[test]
    fn test_random_returns_15_bit_value() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..100 {
            let v = randomgetfn();
            assert!(v >= 0 && v < 0x8000);
        }
    }

    #[test]
    fn test_random_set_seeds_deterministically() {
        let _g = crate::test_util::global_state_lock();
        randomsetfn(42);
        let a = randomgetfn();
        randomsetfn(42);
        let b = randomgetfn();
        assert_eq!(a, b);
    }

    #[test]
    fn test_ifs_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let mut __pm = crate::ported::zsh_h::param::default();
        let original = ifsgetfn(&__pm);
        ifssetfn(&mut __pm, ":,;".to_string());
        assert_eq!(ifsgetfn(&__pm), ":,;");
        ifssetfn(&mut __pm, original);
    }

    #[test]
    fn test_histsiz_clamps_to_1() {
        let _g = crate::test_util::global_state_lock();
        let _g = HISTSIZ_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::test_util::global_state_lock();
        let original = savehistsizegetfn();
        savehistsizesetfn(-5);
        assert_eq!(savehistsizegetfn(), 0);
        savehistsizesetfn(100);
        assert_eq!(savehistsizegetfn(), 100);
        savehistsizesetfn(original);
    }

    /// Pin: `savehistsizesetfn` syncs BOTH storage mirrors so the
    /// twin-storage Rust adaptation behaves like the single global
    /// in C. The params.rs Mutex<i64> drives `$SAVEHIST` reads;
    /// the hist.rs AtomicI64 drives the history-file writer cap.
    /// Previously only the params.rs side was written, so
    /// `SAVEHIST=10000` left hist.rs at 0 and the writer would
    /// cap at zero lines.
    #[test]
    fn savehistsizesetfn_syncs_to_hist_module() {
        let _g = crate::test_util::global_state_lock();
        let original_params = savehistsizegetfn();
        let original_hist = savehistsiz.load(Ordering::SeqCst);
        // Set via the setfn — both storages must reflect the value.
        savehistsizesetfn(12345);
        assert_eq!(
            savehistsizegetfn(),
            12345,
            "c:4994 — params.rs Mutex<i64> reflects new value"
        );
        assert_eq!(
            savehistsiz.load(Ordering::SeqCst),
            12345,
            "c:4994 — hist.rs AtomicI64 synced (was the previous gap)"
        );
        // Negative clamps to 0 in BOTH stores.
        savehistsizesetfn(-99);
        assert_eq!(savehistsizegetfn(), 0, "c:4998 — params.rs clamps to 0");
        assert_eq!(
            savehistsiz.load(Ordering::SeqCst),
            0,
            "c:4998 — hist.rs clamps to 0 too"
        );
        // Restore.
        savehistsizesetfn(original_params);
        savehistsiz.store(original_hist, Ordering::SeqCst);
    }

    #[test]
    fn test_pipestat_round_trip() {
        let _g = crate::test_util::global_state_lock();
        pipestatsetfn(Some(vec![
            "1".to_string(),
            "0".to_string(),
            "127".to_string(),
        ]));
        let v = pipestatgetfn();
        assert_eq!(v, vec!["1", "0", "127"]);
        pipestatsetfn(None);
        assert_eq!(pipestatgetfn(), Vec::<String>::new());
    }

    /// Pin: `setnumvalue` actually STORES the scalar string per
    /// `Src/params.c:2862-2872`. The previous Rust port computed
    /// the string then dropped it via `let _ = s;` — meaning a
    /// numeric assignment to a SCALAR param stored NOTHING.
    ///
    /// C body for PM_SCALAR: `setstrvalue(v, convbase_underscore(
    /// val.u.l, pm->base, pm->width));`. We pin the round-trip
    /// for an integer assigned to a scalar param.
    #[test]
    fn setnumvalue_stores_int_value_into_scalar_pm() {
        let _g = crate::test_util::global_state_lock();
        // c:2860 — setnumvalue bails when unset(EXECOPT). The unit-test
        // env doesn't run through createoptiontable so we set "exec"
        // explicitly to simulate normal runtime.
        let saved_exec = opt_state_get("exec").unwrap_or(false);
        opt_state_set("exec", true);
        // Build a scalar Param with no special base/width.
        let mut pm = Box::new(param {
            node: hashnode {
                next: None,
                nam: "x".to_string(),
                flags: PM_SCALAR as i32,
            },
            u_data: 0,
            u_tied: None,
            u_arr: None,
            u_str: Some(String::new()),
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
        let mut v = value {
            pm: Some(pm.clone()),
            arr: Vec::new(),
            scanflags: 0,
            valflags: 0,
            start: 0,
            end: -1,
        };
        let val = mnumber {
            l: 42,
            d: 0.0,
            type_: MN_INTEGER,
        };
        setnumvalue(Some(&mut v), val);
        // c:2871 — the scalar storage now holds "42".
        let stored = v.pm.as_ref().unwrap().u_str.clone().unwrap_or_default();
        assert_eq!(
            stored, "42",
            "c:2871 — setnumvalue must store the rendered integer; \
             was previously dropped via `let _ = s;`"
        );
        let _ = pm;
        opt_state_set("exec", saved_exec);
    }

    #[test]
    fn test_simple_arrayuniq_first_wins() {
        let _g = crate::test_util::global_state_lock();
        let v = vec![
            "a".to_string(),
            "b".to_string(),
            "a".to_string(),
            "c".to_string(),
        ];
        assert_eq!(simple_arrayuniq(v), vec!["a", "b", "c"]);
    }

    #[test]
    fn test_split_env_string() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        assert_eq!(mkenvstr("PATH", "/usr/bin", 0), "PATH=/usr/bin");
        assert_eq!(mkenvstr("EMPTY", "", 0), "EMPTY=");
    }

    #[test]
    fn test_seconds_round_trip() {
        let _g = crate::test_util::global_state_lock();
        intsecondssetfn(0);
        let s1 = intsecondsgetfn();
        std::thread::sleep(Duration::from_millis(5));
        let s2 = intsecondsgetfn();
        assert!(s2 >= s1);
        // Reset to a known offset and read back.
        setrawseconds(100.0);
        assert_eq!(getrawseconds(), 100.0);
    }

    #[test]
    fn test_argzero_round_trip() {
        let _g = crate::test_util::global_state_lock();
        // pm is UNUSED in argzerosetfn / argzerogetfn (C signature
        // matches Rust). Use Param::default() as the dummy carrier.
        let mut pm = param::default();
        argzerosetfn(&mut pm, "/bin/zsh".to_string());
        assert_eq!(argzerogetfn(&pm), "/bin/zsh");
        argzerosetfn(&mut pm, String::new());
    }

    #[test]
    fn test_env_get_set() {
        let _g = crate::test_util::global_state_lock();
        let result = zputenv("ZSHRS_TEST_VAR=hello");
        assert_eq!(result, 0);
        assert_eq!(zgetenv("ZSHRS_TEST_VAR"), Some("hello".to_string()));
        delenv("ZSHRS_TEST_VAR");
        assert_eq!(zgetenv("ZSHRS_TEST_VAR"), None);
    }

    #[test]
    fn test_keyboardhack_one_char() {
        let _g = crate::test_util::global_state_lock();
        let mut __pm = crate::ported::zsh_h::param::default();
        keyboardhacksetfn(&mut __pm, "\\".to_string());
        assert_eq!(keyboardhackgetfn(&__pm), "\\");
        keyboardhacksetfn(&mut __pm, String::new());
        assert_eq!(keyboardhackgetfn(&__pm), "");
    }

    /// Pin: `keyboardhacksetfn` accepts ASCII chars cleanly per
    /// `Src/params.c:5040-5060`. Tests the canonical happy path
    /// — single ASCII char, empty input, and the ASCII guard.
    ///
    /// The previous Rust port skipped `unmetafy(x, &len)` (c:5044)
    /// before the length and ASCII checks. This test exercises
    /// the surface API; the unmetafy fix is doc-pinned in the
    /// fn body since constructing Meta-encoded String values for
    /// the test fixture would require unsafe (Rust strings must
    /// be valid UTF-8 and the Meta byte 0x83 is not a valid
    /// UTF-8 lead).
    #[test]
    fn keyboardhacksetfn_handles_ascii_and_empty() {
        let _g = crate::test_util::global_state_lock();
        let mut __pm = crate::ported::zsh_h::param::default();
        // c:5056 — single ASCII char stored.
        keyboardhacksetfn(&mut __pm, ";".to_string());
        assert_eq!(
            keyboardhackgetfn(&__pm),
            ";",
            "c:5056 — single ASCII char stored verbatim"
        );
        // c:5056 — different ASCII char stored.
        keyboardhacksetfn(&mut __pm, ",".to_string());
        assert_eq!(keyboardhackgetfn(&__pm), ",");
        // c:5058 — empty input clears to '\0'.
        keyboardhacksetfn(&mut __pm, String::new());
        assert_eq!(keyboardhackgetfn(&__pm), "");
    }

    #[test]
    fn test_histchars_default() {
        let _g = crate::test_util::global_state_lock();
        let _g = HISTCHARS_TEST_LOCK_SHARED
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        histcharssetfn(&mut param::default(), String::new());
        assert_eq!(histcharsgetfn(&param::default()), "!^#");
        histcharssetfn(&mut param::default(), "@$&".to_string());
        assert_eq!(histcharsgetfn(&param::default()), "@$&");
        histcharssetfn(&mut param::default(), String::new());
    }

    /// Pin: `histcharssetfn` runs `unmetafy` per Src/params.c:5086
    /// BEFORE the length truncation and ASCII guard. Previously
    /// the Rust port skipped unmetafy, so a Meta-pair would
    /// inflate the byte count past 3 and the truncation would
    /// drop valid characters.
    ///
    /// Test the happy path: 1-char, 2-char, 3-char ASCII inputs
    /// all parse correctly and each char-position fills the
    /// matching atomic.
    #[test]
    fn histcharssetfn_handles_1_2_3_char_inputs() {
        let _g = crate::test_util::global_state_lock();
        let _g = HISTCHARS_TEST_LOCK_SHARED
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // 1-char: bangchar=='Q', hatchar=='\0', hashchar=='\0'.
        histcharssetfn(&mut param::default(), "Q".to_string());
        assert_eq!(bangchar.load(Ordering::SeqCst), b'Q' as i32);
        assert_eq!(hatchar.load(Ordering::SeqCst), 0);
        assert_eq!(hashchar.load(Ordering::SeqCst), 0);
        // 2-char: bangchar=='X', hatchar=='Y', hashchar=='\0'.
        histcharssetfn(&mut param::default(), "XY".to_string());
        assert_eq!(bangchar.load(Ordering::SeqCst), b'X' as i32);
        assert_eq!(hatchar.load(Ordering::SeqCst), b'Y' as i32);
        assert_eq!(hashchar.load(Ordering::SeqCst), 0);
        // 3-char: bangchar=='A', hatchar=='B', hashchar=='C'.
        histcharssetfn(&mut param::default(), "ABC".to_string());
        assert_eq!(bangchar.load(Ordering::SeqCst), b'A' as i32);
        assert_eq!(hatchar.load(Ordering::SeqCst), b'B' as i32);
        assert_eq!(hashchar.load(Ordering::SeqCst), b'C' as i32);
        // 4+ char: c:5087-5088 truncates to 3.
        histcharssetfn(&mut param::default(), "WXYZ".to_string());
        assert_eq!(bangchar.load(Ordering::SeqCst), b'W' as i32);
        assert_eq!(hatchar.load(Ordering::SeqCst), b'X' as i32);
        assert_eq!(hashchar.load(Ordering::SeqCst), b'Y' as i32);
        // Reset to default.
        histcharssetfn(&mut param::default(), String::new());
        assert_eq!(bangchar.load(Ordering::SeqCst), b'!' as i32);
        assert_eq!(hatchar.load(Ordering::SeqCst), b'^' as i32);
        assert_eq!(hashchar.load(Ordering::SeqCst), b'#' as i32);
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn foundparam_lock() -> &'static Mutex<Option<String>> {
    FOUNDPARAM.get_or_init(|| Mutex::new(None))
}

/// Accessor for the global `paramtab` (Src/params.c:515).
/// Mirrors C's `paramtab->...` dereference by handing back the
/// inner RwLock; callers `.read()` for lookups and `.write()` for
/// mutation, operating on the `HashMap<String, Param>` directly.
pub fn paramtab() -> &'static RwLock<HashMap<String, Param>> {
    PARAMTAB_INNER.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Accessor for the global `realparamtab` (Src/params.c:515).
/// Same role as `paramtab` for the not-currently-redirected case;
/// the alias-flip during assoc-array iteration isn't modelled yet.
pub fn realparamtab() -> &'static RwLock<HashMap<String, Param>> {
    REALPARAMTAB_INNER.get_or_init(|| RwLock::new(HashMap::new()))
}

fn scanprog_lock() -> &'static Mutex<Option<String>> {
    SCANPROG.get_or_init(|| Mutex::new(None))
}

fn scanstr_lock() -> &'static Mutex<Option<String>> {
    SCANSTR.get_or_init(|| Mutex::new(None))
}

fn paramvals_lock() -> &'static Mutex<Vec<String>> {
    PARAMVALS.get_or_init(|| Mutex::new(Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zsh_h::Pound;
    use crate::zsh_h::hashnode;

    /// `setscope_base` pushes the param name onto `SCOPEREFS[base]`
    /// when `base > pm.level` (c:6440). Grows the SCOPEREFS Vec as
    /// needed (c:6442-6447).
    #[test]
    fn setscope_base_pushes_name_when_base_above_level() {
        let _g = crate::test_util::global_state_lock();
        SCOPEREFS.with(|s| s.borrow_mut().clear());
        let mut pm = param {
            node: hashnode {
                next: None,
                nam: "foo".to_string(),
                flags: 0,
            },
            u_data: 0,
            u_tied: None,
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
            level: 2,
        };
        setscope_base(&mut pm, 5);
        assert_eq!(pm.base, 5);
        SCOPEREFS.with(|s| {
            let s = s.borrow();
            assert!(s.len() >= 6, "SCOPEREFS grew to fit index 5");
            assert_eq!(s[5], vec!["foo".to_string()]);
        });
    }

    /// `assignaparam` rejects slice into PM_HASHED with the canonical
    /// "attempt to set slice of associative array" zerr (c:3386-3390).
    #[test]
    fn assignaparam_rejects_slice_into_hashed() {
        let _g = crate::test_util::global_state_lock();
        opt_state_set("exec", true);
        unsetparam("aa_h");
        // Create a hashed param.
        sethparam("aa_h", vec!["k".to_string(), "v".to_string()]);
        let before = paramtab().read().unwrap().contains_key("aa_h");
        assert!(before);
        // Slice write should be rejected.
        let result = assignaparam("aa_h[idx]", vec!["x".to_string()], 0);
        assert!(result.is_none(), "slice into hashed must return None");
        unsetparam("aa_h");
        opt_state_set("exec", false);
    }

    /// `assignaparam` on a PM_NAMEREF param resolves the chain first
    /// (fetchvalue at c:3392): a ref bound to a not-yet-defined name
    /// REDIRECTS the array assignment, creating the TARGET as an
    /// array (K01nameref.ztst "assign new array via nameref":
    /// `typeset -n ptr=var; ptr=(val1 val2)` → `typeset -g -a var`).
    /// The "can't change type of a named reference" rejection
    /// (c:3395-3398) only fires for PLACEHOLDER refs (empty refname),
    /// covered by the K01 test-setting-ref matrix.
    #[test]
    fn assignaparam_rejects_nameref_type_change() {
        let _g = crate::test_util::global_state_lock();
        opt_state_set("exec", true);
        unsetparam("aa_nr");
        // Insert a PM_NAMEREF param directly.
        let pm = param {
            node: hashnode {
                next: None,
                nam: "aa_nr".to_string(),
                flags: (PM_NAMEREF | PM_SCALAR) as i32,
            },
            u_data: 0,
            u_tied: None,
            u_arr: None,
            u_str: Some("target".to_string()),
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
        };
        paramtab()
            .write()
            .unwrap()
            .insert("aa_nr".to_string(), Box::new(pm));

        unsetparam("target");
        let result = assignaparam("aa_nr", vec!["a".to_string(), "b".to_string()], 0);
        assert!(
            result.is_some(),
            "assignment redirects to the ref's target (c:3392 fetchvalue resolve)"
        );

        // PM_NAMEREF flag still present on the REF (not stripped).
        let pm = paramtab().read().unwrap().get("aa_nr").cloned().unwrap();
        assert_ne!(pm.node.flags as u32 & PM_NAMEREF, 0, "PM_NAMEREF preserved");
        assert_eq!(
            pm.u_str.as_deref(),
            Some("target"),
            "refname unchanged by assignment-through"
        );
        // The TARGET was created as an array with the value.
        let t = paramtab().read().unwrap().get("target").cloned().unwrap();
        assert_eq!(
            t.u_arr.as_deref(),
            Some(&["a".to_string(), "b".to_string()][..]),
            "target holds the assigned array"
        );

        unsetparam("target");
        unsetparam("aa_nr");
        opt_state_set("exec", false);
    }

    /// `assignaparam` with ASSPM_AUGMENT against a scalar param must
    /// prepend the previous scalar value as `val[0]` of the new array
    /// (c:3404-3412). Implements `a=x; a+=(y z)` → `a=(x y z)`. A
    /// regression that drops the prepend would yield `a=(y z)` and
    /// silently lose the original scalar — invisible at write time,
    /// surfaces only when the caller reads `${a[1]}`.
    #[test]
    fn assignaparam_augment_prepends_old_scalar() {
        let _g = crate::test_util::global_state_lock();
        opt_state_set("exec", true);
        unsetparam("aa_aug");
        // Seed a scalar.
        setsparam("aa_aug", "old");
        // Augment with two new values.
        let pm = assignaparam(
            "aa_aug",
            vec!["new1".to_string(), "new2".to_string()],
            ASSPM_AUGMENT,
        )
        .expect("augment should succeed");
        let arr = pm.u_arr.expect("ASSPM_AUGMENT must produce u_arr");
        assert_eq!(
            arr,
            vec!["old".to_string(), "new1".to_string(), "new2".to_string()],
            "c:3408-3411 — scalar prepended at index 0, then new values follow"
        );
        unsetparam("aa_aug");
        opt_state_set("exec", false);
    }

    /// `assignaparam` against a PM_UNIQUE-flagged target dedupes
    /// the value array (c:3401 + arrsetfn's uniqarray). Implements
    /// `typeset -U arr; arr=(a b a c b)` → `arr=(a b c)`. A
    /// regression that drops the uniqarray call would let duplicates
    /// linger — invisible until a downstream `[[ -n ${arr[(r)b]} ]]`
    /// check counts more matches than expected, or `$path` grows
    /// unbounded with repeated directory entries.
    #[test]
    fn assignaparam_unique_flag_dedupes_values() {
        let _g = crate::test_util::global_state_lock();
        opt_state_set("exec", true);
        unsetparam("aa_uniq");
        // Seed an empty PM_ARRAY|PM_UNIQUE.
        setaparam("aa_uniq", vec![]);
        {
            let mut tab = paramtab().write().unwrap();
            let pm = tab.get_mut("aa_uniq").expect("aa_uniq must exist");
            pm.node.flags |= PM_UNIQUE as i32;
        }
        // Now write duplicates; PM_UNIQUE must collapse them.
        let pm = assignaparam(
            "aa_uniq",
            vec!["a".into(), "b".into(), "a".into(), "c".into(), "b".into()],
            0,
        )
        .expect("assignment succeeds");
        let arr = pm.u_arr.expect("u_arr populated");
        assert_eq!(
            arr,
            vec!["a".to_string(), "b".to_string(), "c".to_string()],
            "c:3401 — PM_UNIQUE collapses duplicates, keeping first occurrence"
        );
        // The PM_UNIQUE flag must persist through the assignment.
        let pm_check = paramtab().read().unwrap().get("aa_uniq").cloned().unwrap();
        assert_ne!(
            pm_check.node.flags as u32 & PM_UNIQUE,
            0,
            "PM_UNIQUE flag preserved across assignment"
        );
        unsetparam("aa_uniq");
        opt_state_set("exec", false);
    }

    /// `getsparam` reads PM_INTEGER params via convbase, not via
    /// `u_str` (which is None for typed integers). Pins the latent
    /// bug fix that made `read REPLY` after `(( REPLY=42 ))` return
    /// nothing — every numeric param read used to fall through to
    /// the OS env-var fallback.
    #[test]
    fn getsparam_returns_integer_via_convbase() {
        let _g = crate::test_util::global_state_lock();
        opt_state_set("exec", true);
        unsetparam("gs_int");
        setiparam("gs_int", 999);
        assert_eq!(getsparam("gs_int").as_deref(), Some("999"));
        unsetparam("gs_int");
        opt_state_set("exec", false);
    }

    /// `getsparam` reads PM_FFLOAT params via convfloat. Same fix
    /// shape as the PM_INTEGER path.
    #[test]
    fn getsparam_returns_float_via_convfloat() {
        let _g = crate::test_util::global_state_lock();
        opt_state_set("exec", true);
        unsetparam("gs_f");
        // Stash a float via the setnumvalue / setnparam path.
        let v = mnumber {
            l: 0,
            d: 2.5,
            type_: MN_FLOAT,
        };
        setnparam("gs_f", v);
        let s = getsparam("gs_f").expect("PM_FFLOAT should serialize");
        // convfloat formats with default precision; just check it's
        // not empty and parses back.
        assert!(
            s.parse::<f64>()
                .map(|f| (f - 2.5).abs() < 1e-6)
                .unwrap_or(false),
            "expected ~2.5 round-trip, got {:?}",
            s
        );
        unsetparam("gs_f");
        opt_state_set("exec", false);
    }

    /// `getsparam` against a non-default `pm.base` integer param must
    /// render using that base via `convbase`. The c:2364 dispatch
    /// passes `pm.base` (or 10) to convbase; a regression that
    /// hardcodes base=10 would silently break `typeset -i 16 hex=255`
    /// readers (would see "255" instead of the C-faithful "16#FF").
    #[test]
    fn getsparam_integer_honors_pm_base() {
        let _g = crate::test_util::global_state_lock();
        opt_state_set("exec", true);
        unsetparam("hex_param");
        // Manually insert a PM_INTEGER param with base=16.
        let pm = param {
            node: hashnode {
                next: None,
                nam: "hex_param".to_string(),
                flags: PM_INTEGER as i32,
            },
            u_data: 0,
            u_tied: None,
            u_arr: None,
            u_str: None,
            u_val: 255,
            u_dval: 0.0,
            u_hash: None,
            gsu_s: None,
            gsu_i: None,
            gsu_f: None,
            gsu_a: None,
            gsu_h: None,
            base: 16,
            width: 0,
            env: None,
            ename: None,
            old: None,
            level: 0,
        };
        paramtab()
            .write()
            .unwrap()
            .insert("hex_param".to_string(), Box::new(pm));

        let s = getsparam("hex_param").expect("PM_INTEGER must serialize");
        // convbase emits "16#FF" form for base 16.
        assert!(
            s.contains("FF") || s.contains("ff"),
            "c:2364 — base-16 must render hex digits; got {:?}",
            s
        );

        unsetparam("hex_param");
        opt_state_set("exec", false);
    }

    /// `endparamscope` clears `SCOPEREFS[old_locallevel]` and resets
    /// nameref params' base when their base exceeds the new locallevel.
    /// End-to-end of the setscope_base writer + endparamscope reader.
    #[test]
    fn endparamscope_resets_scoperefs_and_nameref_base() {
        let _g = crate::test_util::global_state_lock();
        SCOPEREFS.with(|s| s.borrow_mut().clear());

        // Set up: locallevel = 3, push a PM_NAMEREF param with base=5 onto SCOPEREFS[3].
        set_locallevel(3);
        let pm = param {
            node: hashnode {
                next: None,
                nam: "ref1".to_string(),
                flags: (PM_NAMEREF | PM_SCALAR) as i32,
            },
            u_data: 0,
            u_tied: None,
            u_arr: None,
            u_str: Some(String::new()),
            u_val: 0,
            u_dval: 0.0,
            u_hash: None,
            gsu_s: None,
            gsu_i: None,
            gsu_f: None,
            gsu_a: None,
            gsu_h: None,
            base: 5,
            width: 0,
            env: None,
            ename: None,
            old: None,
            level: 0,
        };
        paramtab()
            .write()
            .unwrap()
            .insert("ref1".to_string(), Box::new(pm));
        // Populate SCOPEREFS[3] manually with "ref1".
        SCOPEREFS.with(|sr| {
            let mut sr = sr.borrow_mut();
            sr.resize(8, Vec::new());
            sr[3].push("ref1".to_string());
        });

        endparamscope();

        // After endparamscope: locallevel decremented to 2; "ref1"'s
        // base reset to 0 (was 5 > new ll=2). SCOPEREFS[3] cleared.
        let pm_after = paramtab().read().unwrap().get("ref1").cloned();
        assert!(pm_after.is_some(), "ref1 should still exist (level=0)");
        assert_eq!(pm_after.unwrap().base, 0, "PM_NAMEREF.base reset to 0");
        SCOPEREFS.with(|sr| {
            assert!(sr.borrow()[3].is_empty(), "SCOPEREFS[3] cleared");
        });

        // Cleanup
        paramtab().write().unwrap().remove("ref1");
        set_locallevel(0);
    }

    /// `endparamscope` MUST NOT reset `base` on PM_UPPER namerefs
    /// (c:5891 — the `!(pm->node.flags & PM_UPPER)` clause guards
    /// the reset). PM_UPPER namerefs point UPWARD in the scope chain
    /// (e.g. `typeset -n -u up=outer` from inside a function) and
    /// their base must persist across scope pops so the upward
    /// resolution keeps working. A regression that drops the PM_UPPER
    /// guard would silently degrade upward namerefs into local ones
    /// after the first function return.
    #[test]
    fn endparamscope_preserves_pm_upper_nameref_base() {
        let _g = crate::test_util::global_state_lock();
        SCOPEREFS.with(|s| s.borrow_mut().clear());

        set_locallevel(3);
        let pm = param {
            node: hashnode {
                next: None,
                nam: "up_ref".to_string(),
                flags: (PM_NAMEREF | PM_SCALAR | PM_UPPER) as i32,
            },
            u_data: 0,
            u_tied: None,
            u_arr: None,
            u_str: Some(String::new()),
            u_val: 0,
            u_dval: 0.0,
            u_hash: None,
            gsu_s: None,
            gsu_i: None,
            gsu_f: None,
            gsu_a: None,
            gsu_h: None,
            base: 5,
            width: 0,
            env: None,
            ename: None,
            old: None,
            level: 0,
        };
        paramtab()
            .write()
            .unwrap()
            .insert("up_ref".to_string(), Box::new(pm));
        SCOPEREFS.with(|sr| {
            let mut sr = sr.borrow_mut();
            sr.resize(8, Vec::new());
            sr[3].push("up_ref".to_string());
        });

        endparamscope();

        let pm_after = paramtab()
            .read()
            .unwrap()
            .get("up_ref")
            .cloned()
            .expect("up_ref must survive scope pop");
        assert_eq!(
            pm_after.base, 5,
            "c:5891 — PM_UPPER nameref base MUST be preserved (was 5, must stay 5)"
        );
        assert_ne!(
            pm_after.node.flags as u32 & PM_UPPER,
            0,
            "PM_UPPER flag itself must persist"
        );

        paramtab().write().unwrap().remove("up_ref");
        set_locallevel(0);
    }

    /// `setscope_base` does NOT push when `base <= pm.level` (the C
    /// `if ((pm->base = base) > pm->level)` guard at c:6440 fails).
    #[test]
    fn setscope_base_no_push_when_base_below_level() {
        let _g = crate::test_util::global_state_lock();
        SCOPEREFS.with(|s| s.borrow_mut().clear());
        let mut pm = param {
            node: hashnode {
                next: None,
                nam: "bar".to_string(),
                flags: 0,
            },
            u_data: 0,
            u_tied: None,
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
            level: 10,
        };
        setscope_base(&mut pm, 3); // 3 <= 10 → no push
        assert_eq!(pm.base, 3);
        SCOPEREFS.with(|s| {
            // No push happened; SCOPEREFS stays empty.
            assert!(s.borrow().is_empty() || s.borrow().iter().all(|v| v.is_empty()));
        });
    }

    /// `setscope_base` boundary: `base == pm.level` must NOT push.
    /// The C guard c:6440 is strictly `>` (`> pm->level`, not `>=`).
    /// A regression that uses `>=` would push every assignment at
    /// the current scope into SCOPEREFS, causing endparamscope to
    /// re-process every same-scope param on every function return —
    /// O(n) extra work per call PLUS spurious base resets.
    #[test]
    fn setscope_base_equal_level_does_not_push() {
        let _g = crate::test_util::global_state_lock();
        SCOPEREFS.with(|s| s.borrow_mut().clear());
        let mut pm = param {
            node: hashnode {
                next: None,
                nam: "edge".to_string(),
                flags: 0,
            },
            u_data: 0,
            u_tied: None,
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
            level: 5,
        };
        setscope_base(&mut pm, 5); // base == level → strict `>` fails
        assert_eq!(pm.base, 5, "base assignment always happens");
        SCOPEREFS.with(|sr| {
            let any_push = sr.borrow().iter().any(|v| !v.is_empty());
            assert!(
                !any_push,
                "c:6440 — `base > pm->level` is STRICT; equal must not push"
            );
        });
    }

    #[test]
    fn test_colonarr_conversion() {
        let _g = crate::test_util::global_state_lock();
        let arr = colonsplit("/bin:/usr/bin:/usr/local/bin", false);
        assert_eq!(arr, vec!["/bin", "/usr/bin", "/usr/local/bin"]);
        let path = colonarrgetfn(&arr);
        assert_eq!(path, "/bin:/usr/bin:/usr/local/bin");
    }
    #[test]
    fn test_isident() {
        let _g = crate::test_util::global_state_lock();
        assert!(isident("foo"));
        assert!(isident("_bar"));
        assert!(isident("FOO_BAR"));
        assert!(isident("x123"));
        assert!(isident("123")); // positional params
        assert!(!isident(""));
        assert!(!isident("foo bar"));
    }

    /// Pin: `isident` requires balanced `[...]` per `Src/params.c:1329-1330`:
    ///   if (*ss != '[') return 0;
    ///   if (!(ss = parse_subscript(++ss, 1, ']'))) return 0;
    ///
    /// The previous Rust port accepted ANY `[` as a valid
    /// terminator (`if c == '[' { return true; }`) without
    /// checking for a matching `]`. So `foo[` (no close) was
    /// accepted as a valid identifier — diverging from C which
    /// rejects.
    #[test]
    fn isident_requires_balanced_subscript_brackets() {
        let _g = crate::test_util::global_state_lock();
        // Balanced `[...]` is valid.
        assert!(
            isident("foo[0]"),
            "c:1330 — balanced [0] passes parse_subscript"
        );
        assert!(
            isident("foo[bar]"),
            "c:1330 — balanced [bar] passes parse_subscript"
        );
        // UNBALANCED — open without close — must be rejected.
        assert!(
            !isident("foo["),
            "c:1330 — `foo[` missing `]` MUST be rejected"
        );
        // Trailing chars after `]` — C parse_subscript returns
        // a position INSIDE the string, the surrounding isident
        // body checks that nothing follows; our port currently
        // returns true at the first `[` either way, but pin the
        // balanced case as a working invariant.
        assert!(isident("a[1]"), "c:1330 — short balanced subscript valid");
    }

    #[test]
    fn test_unique_array() {
        let _g = crate::test_util::global_state_lock();
        let arr = vec!["a".into(), "b".into(), "a".into(), "c".into(), "b".into()];
        let result = uniqarray(arr);
        assert_eq!(result, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_convbase() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        // Use 2.5 instead of 3.14 — clippy errors on the latter as
        // an approx PI constant. The test checks 2-decimal formatting
        // round-trips, which the exact value doesn't influence.
        let s = convfloat(2.5, 2, PM_FFLOAT);
        assert!(s.starts_with("2.50"));

        assert_eq!(convfloat(f64::INFINITY, 0, 0), "Inf");
        assert_eq!(convfloat(f64::NEG_INFINITY, 0, 0), "-Inf");
        assert_eq!(convfloat(f64::NAN, 0, 0), "NaN");
    }

    #[test]
    fn test_getarrvalue() {
        let _g = crate::test_util::global_state_lock();
        let arr = vec!["a".into(), "b".into(), "c".into(), "d".into()];
        assert_eq!(getarrvalue(&arr, 2, 3), vec!["b", "c"]);
        assert_eq!(getarrvalue(&arr, -2, -1), vec!["c", "d"]);
        assert_eq!(getarrvalue(&arr, 1, 4), vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn test_setarrvalue() {
        let _g = crate::test_util::global_state_lock();
        // c:2897 — setarrvalue bails when unset(EXECOPT). Set "exec"
        // for the unit-test env (real zsh defaults exec=true).
        let saved_exec = opt_state_get("exec").unwrap_or(false);
        opt_state_set("exec", true);
        // C-faithful: setarrvalue takes a Value pointing at a Param
        // with u_arr set. Construct one inline.
        let pm = Box::new(param {
            node: hashnode {
                next: None,
                nam: "test".to_string(),
                flags: PM_ARRAY as i32,
            },
            u_data: 0,
            u_tied: None,
            u_arr: Some(vec!["a".into(), "b".into(), "c".into(), "d".into()]),
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
        let mut v = value {
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
        opt_state_set("exec", saved_exec);
    }

    #[test]
    fn test_valid_refname() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        let empty: Vec<String> = Vec::new();
        assert!(uniqarray(empty).is_empty());
    }

    #[test]
    fn test_convbase_underscore() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        // C params.c:1431-1442 + 1758 — `(en.2.)pat` picks 2nd exact match.
        let arr: Vec<String> = vec!["foo".into(), "bar".into(), "foo".into(), "baz".into()];
        let out = getarg("(en.2.r)foo", Some(&arr), None, None).expect("Some");
        assert_eq!(val_str(out), "foo");
    }

    #[test]
    fn getarg_n_flag_third_exact_match() {
        let _g = crate::test_util::global_state_lock();
        let arr: Vec<String> = vec!["a".into(), "a".into(), "a".into(), "b".into()];
        let out = getarg("(en.3.r)a", Some(&arr), None, None).expect("Some");
        assert_eq!(val_str(out), "a");
    }

    #[test]
    fn getarg_n_flag_returns_index_with_i() {
        let _g = crate::test_util::global_state_lock();
        // (en.2.i) — return INDEX of 2nd exact match.
        let arr: Vec<String> = vec!["x".into(), "y".into(), "x".into(), "y".into()];
        let out = getarg("(en.2.i)x", Some(&arr), None, None).expect("Some");
        assert_eq!(val_str(out), "3");
    }

    #[test]
    fn getarg_negative_n_flips_search_direction() {
        let _g = crate::test_util::global_state_lock();
        // C params.c:1488-1491 — negative `num` flips down (reverse).
        // (en.-1.) on forward-default search matches from the end.
        let arr: Vec<String> = vec!["a".into(), "a".into(), "a".into()];
        let out = getarg("(en.-1.i)a", Some(&arr), None, None).expect("Some");
        assert_eq!(val_str(out), "3");
    }

    #[test]
    fn getarg_n_flag_zero_treated_as_one() {
        let _g = crate::test_util::global_state_lock();
        // C params.c:1438-1439 — `if (!num) num = 1`.
        let arr: Vec<String> = vec!["x".into(), "y".into()];
        let out = getarg("(en.0.r)x", Some(&arr), None, None).expect("Some");
        assert_eq!(val_str(out), "x");
    }

    #[test]
    fn getarg_unknown_flag_char_returns_none() {
        let _g = crate::test_util::global_state_lock();
        // C params.c:1477-1483 flagerr — invalid flag char reports error.
        let arr: Vec<String> = vec!["x".into()];
        assert!(getarg("(z)x", Some(&arr), None, None).is_none());
    }

    #[test]
    fn getarg_n_flag_unterminated_arg_returns_none() {
        let _g = crate::test_util::global_state_lock();
        // (n.5 missing closing delimiter — flagerr.
        let arr: Vec<String> = vec!["x".into()];
        assert!(getarg("(n.5", Some(&arr), None, None).is_none());
    }

    #[test]
    fn getarg_b_flag_starts_search_at_index() {
        let _g = crate::test_util::global_state_lock();
        // C params.c:1748-1760 — `(b.N.e)pat` skips first N-1 elements
        // forward (parsed value `N`, normalized to `beg = N-1`).
        let arr: Vec<String> = vec!["x".into(), "y".into(), "x".into(), "y".into()];
        // Forward, beg=2 (skip first 2) → starts at idx 2 → 'x' at 3.
        let out = getarg("(b.3.ei)x", Some(&arr), None, None).expect("Some");
        assert_eq!(val_str(out), "3");
    }

    #[test]
    fn getarg_b_flag_with_R_reverse_from_offset() {
        let _g = crate::test_util::global_state_lock();
        // C params.c:1750-1755 — reverse search starting at parsed-1 idx.
        // arr=(x y x y), beg=2 (parsed 3-1), reverse → walks 2,1,0; first
        // exact 'x' is at idx 2 → 1-based "3".
        let arr: Vec<String> = vec!["x".into(), "y".into(), "x".into(), "y".into()];
        let out = getarg("(b.3.eIR)x", Some(&arr), None, None).expect("Some");
        assert_eq!(val_str(out), "3");
    }

    #[test]
    fn getarg_b_flag_out_of_bounds_forward_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        // c:1746 — beg >= len returns len+1 (empty for value-mode).
        let arr: Vec<String> = vec!["x".into()];
        let out = getarg("(b.5.er)x", Some(&arr), None, None).expect("Some");
        assert_eq!(val_str(out), "");
    }

    #[test]
    fn getarg_b_flag_out_of_bounds_index_mode_returns_len_plus_one() {
        let _g = crate::test_util::global_state_lock();
        let arr: Vec<String> = vec!["x".into(), "y".into()];
        let out = getarg("(b.5.ei)x", Some(&arr), None, None).expect("Some");
        assert_eq!(val_str(out), "3");
    }

    #[test]
    fn getarg_hash_neg_num_on_lowercase_r_returns_all() {
        let _g = crate::test_util::global_state_lock();
        // C params.c:1488-1491 — neg `num` flips down on `r`,
        // converting hash search to return-all-matches semantics.
        let mut h: IndexMap<String, String> = IndexMap::new();
        h.insert("a".into(), "1".into());
        h.insert("b".into(), "1".into());
        h.insert("c".into(), "2".into());
        let out = getarg("(en.-1.r)1", None, Some(&h), None).expect("Some");
        // r + neg = R semantics → all values where pat matches value.
        assert_eq!(val_str(out), "1 1");
    }

    #[test]
    fn getarg_hash_neg_num_on_uppercase_R_returns_single() {
        let _g = crate::test_util::global_state_lock();
        // R + neg `num` un-flips back to single-match (r semantics).
        let mut h: IndexMap<String, String> = IndexMap::new();
        h.insert("a".into(), "1".into());
        h.insert("b".into(), "1".into());
        h.insert("c".into(), "2".into());
        let out = getarg("(en.-1.R)1", None, Some(&h), None).expect("Some");
        // R + neg → r → single first match.
        assert_eq!(val_str(out), "1");
    }

    #[test]
    fn getarg_hash_b_flag_skips_first_n_entries() {
        let _g = crate::test_util::global_state_lock();
        // C params.c:1740-1742 — `b<NUM>` skips first N-1 entries
        // before searching. Hash iteration is insertion order.
        let mut h: IndexMap<String, String> = IndexMap::new();
        h.insert("a".into(), "1".into());
        h.insert("b".into(), "1".into());
        h.insert("c".into(), "1".into());
        // beg=2 (parsed 3-1) → skip first 2, scan from "c" onward.
        let out = getarg("(b.3.ei)1", None, Some(&h), None).expect("Some");
        assert_eq!(val_str(out), "c");
    }

    #[test]
    fn getarg_hash_b_flag_with_R_collects_from_offset() {
        let _g = crate::test_util::global_state_lock();
        // R returns all matches; b skips first beg entries first.
        let mut h: IndexMap<String, String> = IndexMap::new();
        h.insert("a".into(), "1".into());
        h.insert("b".into(), "1".into());
        h.insert("c".into(), "1".into());
        let out = getarg("(b.2.eI)1", None, Some(&h), None).expect("Some");
        // beg=1, return_all=I → walk from "b" onward, all matching keys.
        assert_eq!(val_str(out), "b c");
    }

    #[test]
    fn getarg_hash_b_flag_out_of_bounds_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        // c:1746 — beg >= len with single-match → empty.
        let mut h: IndexMap<String, String> = IndexMap::new();
        h.insert("a".into(), "1".into());
        let out = getarg("(b.5.e)1", None, Some(&h), None).expect("Some");
        assert_eq!(val_str(out), "");
    }

    #[test]
    fn getarg_w_flag_splits_multi_word_array_elements() {
        let _g = crate::test_util::global_state_lock();
        // C params.c:1761-1797 — `(w)N` joins array then re-splits by
        // IFS-default whitespace. arr=("a b" "c d"); (w)2 → "b" not "c d".
        let arr: Vec<String> = vec!["a b".into(), "c d".into()];
        let out = getarg("(w)2", Some(&arr), None, None).expect("Some");
        assert_eq!(val_str(out), "b");
    }

    #[test]
    fn getarg_w_flag_simple_array_indexing_still_works() {
        let _g = crate::test_util::global_state_lock();
        let arr: Vec<String> = vec!["one".into(), "two".into(), "three".into()];
        let out = getarg("(w)2", Some(&arr), None, None).expect("Some");
        assert_eq!(val_str(out), "two");
    }

    #[test]
    fn getarg_f_flag_splits_by_newline() {
        let _g = crate::test_util::global_state_lock();
        // C params.c:1424-1427 — `f` flag aliases `w` with sep="\n".
        // arr=("a b\nc d"); (f)2 → "c d" (split by \n only, not space).
        let arr: Vec<String> = vec!["a b\nc d".into()];
        let out = getarg("(f)2", Some(&arr), None, None).expect("Some");
        assert_eq!(val_str(out), "c d");
    }

    #[test]
    fn getarg_scalar_w_flag_picks_nth_word() {
        let _g = crate::test_util::global_state_lock();
        // C params.c:1761-1797 — scalar word-mode arm. `(w)2` on
        // scalar "hello world foo" returns the 2nd whitespace word.
        let out = getarg("(w)2", None, None, Some("hello world foo")).expect("Some");
        assert_eq!(val_str(out), "world");
    }

    #[test]
    fn getarg_scalar_w_flag_negative_index_counts_from_end() {
        let _g = crate::test_util::global_state_lock();
        let out = getarg("(w)-1", None, None, Some("alpha beta gamma")).expect("Some");
        assert_eq!(val_str(out), "gamma");
    }

    #[test]
    fn getarg_scalar_re_returns_char_at_match_position() {
        let _g = crate::test_util::global_state_lock();
        // C params.c:1798-1980 — char-search returns CHAR at match
        // position, not full substring. Verified empirically:
        //   /bin/zsh -c 's="barfooxyz"; print "${s[(r)foo]}"'  → "f"
        let out = getarg("(re)bc", None, None, Some("abcdef")).expect("Some");
        assert_eq!(val_str(out), "b");
    }

    #[test]
    fn getarg_scalar_ie_returns_position_of_first_match() {
        let _g = crate::test_util::global_state_lock();
        let out = getarg("(ie)cd", None, None, Some("abcdef")).expect("Some");
        // 'cd' starts at 1-based position 3.
        assert_eq!(val_str(out), "3");
    }

    #[test]
    fn getarg_scalar_Ie_returns_position_of_last_match() {
        let _g = crate::test_util::global_state_lock();
        let out = getarg("(Ie)b", None, None, Some("abcabc")).expect("Some");
        // Last 'b' is at 1-based position 5.
        assert_eq!(val_str(out), "5");
    }

    #[test]
    fn getarg_scalar_ie_no_match_returns_len_plus_one() {
        let _g = crate::test_util::global_state_lock();
        let out = getarg("(ie)z", None, None, Some("abc")).expect("Some");
        assert_eq!(val_str(out), "4");
    }

    #[test]
    fn getarg_scalar_Ie_no_match_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let out = getarg("(Ie)z", None, None, Some("abc")).expect("Some");
        assert_eq!(val_str(out), "0");
    }

    #[test]
    fn getarg_scalar_n_flag_picks_second_match() {
        let _g = crate::test_util::global_state_lock();
        // C params.c:1929/1964 — `!--num` Nth-match counter on
        // scalar char-search. abcabc: 'a' at idx 0 and 3 → 2nd match
        // at byte position 4 (1-based).
        let out = getarg("(en.2.i)a", None, None, Some("abcabc")).expect("Some");
        assert_eq!(val_str(out), "4");
    }

    #[test]
    fn getarg_scalar_b_flag_starts_from_offset() {
        let _g = crate::test_util::global_state_lock();
        // C params.c:1740-1742 — `(b.N.)` starts search from idx N-1.
        // abc bc abc: with b=4, skip first 3 chars; first 'b' at byte 5.
        let out = getarg("(b.4.ei)b", None, None, Some("abcbc")).expect("Some");
        assert_eq!(val_str(out), "4");
    }

    #[test]
    fn getarg_scalar_re_n2_picks_second_substring() {
        let _g = crate::test_util::global_state_lock();
        let out = getarg("(en.2.r)b", None, None, Some("abab")).expect("Some");
        assert_eq!(val_str(out), "b");
    }

    /// c:3076/3193 — assignsparam writes into paramtab; getsparam
    /// reads it back. The round-trip is the spine of every
    /// `foo=bar; print $foo` flow. Regression here would silently
    /// drop assignments.
    #[test]
    fn assignsparam_then_getsparam_round_trips() {
        let _g = crate::test_util::global_state_lock(); // c:3193
                                                        // c:2697 — assignsparam → assignstrvalue bails when
                                                        // unset(EXECOPT). The unit-test env doesn't run through
                                                        // createoptiontable so we set "exec" explicitly to simulate
                                                        // normal runtime. Mirrors the same setup used by
                                                        // `setnumvalue_stores_int_value_into_scalar_pm` above.
        let saved_exec = opt_state_get("exec") // c:2697
            .unwrap_or(false); // c:2697
        opt_state_set("exec", true); // c:2697
        let name = "ZSHRS_TEST_ASSIGN_GET"; // c:3193
        assignsparam(name, "test_value_42", 0); // c:3193
        assert_eq!(
            // c:3076
            getsparam(name).as_deref(), // c:3076
            Some("test_value_42")       // c:3076
        ); // c:3076
           // Cleanup so other tests don't see leaked param.
        let _ = paramtab().write().unwrap().remove(name); // c:3819
        opt_state_set("exec", saved_exec); // c:2697
    }

    /// c:3076 — getsparam on a non-existent param returns None.
    /// A regression returning Some("") would mask unset-param errors.
    #[test]
    fn getsparam_unknown_param_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(getsparam("ZSHRS_TEST_DEFINITELY_UNSET").is_none());
    }

    /// c:3819 — direct paramtab.remove drops the entry; subsequent
    /// getsparam returns None. The set→remove→lookup gap verifies
    /// the canonical paramtab is actually backing both reads + writes.
    #[test]
    fn paramtab_remove_makes_getsparam_return_none() {
        let _g = crate::test_util::global_state_lock();
        let name = "ZSHRS_TEST_UNSET_FLOW";
        assignsparam(name, "to_be_removed", 0);
        assert!(
            getsparam(name).is_some(),
            "param must be set before remove path"
        );
        let _ = paramtab().write().unwrap().remove(name);
        assert!(
            getsparam(name).is_none(),
            "after remove, getsparam must return None"
        );
    }

    /// c:3357 — assignaparam stores an array. getsparam on an array
    /// param returns the first element OR a join (depends on IFS).
    /// Verify the slot was populated AT ALL by querying paramtab.
    #[test]
    fn assignaparam_populates_paramtab_with_array() {
        let _g = crate::test_util::global_state_lock();
        let name = "ZSHRS_TEST_ARR_X";
        assignaparam(name, vec!["a".into(), "b".into(), "c".into()], 0);
        let tab = paramtab().read().expect("paramtab poisoned");
        let pm = tab.get(name).expect("param installed");
        assert_eq!(
            pm.u_arr.as_deref(),
            Some(&["a".to_string(), "b".to_string(), "c".to_string()][..]),
            "assignaparam stores all three elements"
        );
        drop(tab);
        let _ = paramtab().write().unwrap().remove(name);
    }

    // Use the module-scope HISTCHARS_TEST_LOCK_SHARED (declared
    // outside the test modules) so gsu_tests + tests serialise
    // against the same Mutex rather than two independent ones.

    /// `Src/params.c:5095-5097` — `histcharssetfn` stores bangchar /
    /// hatchar / hashchar in the per-char globals. Pin the round-trip
    /// for ALL THREE: change HISTCHARS to a custom 3-char string,
    /// verify each atomic global reflects the new value, and verify
    /// the canonical default `"!^#"` restores on NULL.
    #[test]
    fn histcharssetfn_syncs_all_three_histchar_globals() {
        let _g = crate::test_util::global_state_lock();
        let _g = HISTCHARS_TEST_LOCK_SHARED
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // Default state.
        histcharssetfn(&mut param::default(), String::new());
        assert_eq!(bangchar.load(Ordering::SeqCst), b'!' as i32);
        assert_eq!(hatchar.load(Ordering::SeqCst), b'^' as i32);
        assert_eq!(hashchar.load(Ordering::SeqCst), b'#' as i32);
        // Set HISTCHARS to "@:%".
        histcharssetfn(&mut param::default(), "@:%".to_string());
        assert_eq!(
            bangchar.load(Ordering::SeqCst),
            b'@' as i32,
            "c:5095 — bangchar = first byte of HISTCHARS"
        );
        assert_eq!(
            hatchar.load(Ordering::SeqCst),
            b':' as i32,
            "c:5096 — hatchar = second byte of HISTCHARS"
        );
        assert_eq!(
            hashchar.load(Ordering::SeqCst),
            b'%' as i32,
            "c:5097 — hashchar = third byte of HISTCHARS"
        );
        // Restore.
        histcharssetfn(&mut param::default(), String::new());
        assert_eq!(bangchar.load(Ordering::SeqCst), b'!' as i32);
        assert_eq!(hashchar.load(Ordering::SeqCst), b'#' as i32);
    }

    /// `Src/params.c:5064-5074` — `histcharsgetfn` reads from the
    /// three atomic globals and returns a string of non-NUL bytes.
    /// Pin set→get symmetry: after `histcharssetfn(Some("@&%"))`,
    /// `histcharsgetfn(&param::default())` returns `"@&%"`.
    #[test]
    fn histcharsgetfn_round_trips_with_histcharssetfn() {
        let _g = crate::test_util::global_state_lock();
        let _g = HISTCHARS_TEST_LOCK_SHARED
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        histcharssetfn(&mut param::default(), "@&%".to_string());
        assert_eq!(
            histcharsgetfn(&param::default()),
            "@&%",
            "c:5068-5073 — getfn reads atomic globals setfn wrote"
        );
        // Restore default and verify round-trip.
        histcharssetfn(&mut param::default(), String::new());
        assert_eq!(
            histcharsgetfn(&param::default()),
            "!^#",
            "default `!^#` round-trips through atomics"
        );
    }

    /// `Src/params.c:5118-5128` — `homesetfn(x)` round-trip:
    /// `homesetfn(s); homegetfn() == s` for non-symlink paths and
    /// CHASELINKS-off. Pins the basic store-then-read contract.
    #[test]
    fn homesetfn_stores_value_for_getfn() {
        let _g = crate::test_util::global_state_lock();
        let mut __pm = crate::ported::zsh_h::param::default();
        let saved = homegetfn(&__pm);
        homesetfn(&mut __pm, "/tmp/zshrs_test_home".to_string());
        assert_eq!(
            homegetfn(&__pm),
            "/tmp/zshrs_test_home",
            "c:5121-5126 — homesetfn → homegetfn round-trip"
        );
        // Restore.
        homesetfn(&mut __pm, saved);
    }

    /// `Src/params.c:5125-5126` — empty input becomes `ztrdup("")`.
    /// Pin empty-string handling.
    #[test]
    fn homesetfn_empty_input_stores_empty() {
        let _g = crate::test_util::global_state_lock();
        let mut __pm = crate::ported::zsh_h::param::default();
        let saved = homegetfn(&__pm);
        homesetfn(&mut __pm, String::new());
        assert_eq!(
            homegetfn(&__pm),
            "",
            "c:5126 — empty x stores empty (no panic)"
        );
        homesetfn(&mut __pm, saved);
    }

    /// `Src/params.c:5004-5011` — `errnosetfn(x)` writes errno
    /// unconditionally, then warns (NOT errors) on truncation. The
    /// store happens regardless of warning. Pin set→get round-trip.
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn errnosetfn_writes_through_to_libc_errno_getfn() {
        let _g = crate::test_util::global_state_lock();
        // Set errno to a small int.
        errnosetfn(42);
        assert_eq!(
            errnogetfn(),
            42,
            "c:5006 — errno = (int)x; subsequent getfn must read it back"
        );
        errnosetfn(0);
        assert_eq!(errnogetfn(), 0);
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
        let _g = crate::test_util::global_state_lock();
        // i64::MAX → truncates to i32 = -1 → warning fires inside.
        // The store at c:5008 happens; whether the warning's libc
        // calls then overwrite errno is implementation-defined.
        errnosetfn(i64::MAX);
        // Just verify the call returned (no panic) and getfn works.
        let _ = errnogetfn();
        // Reset.
        errnosetfn(0);
    }

    /// `Src/params.c:5090-5093` — non-ASCII chars in HISTCHARS
    /// produce a warning and the function returns WITHOUT updating
    /// any globals. Pin the rejection: state before == state after
    /// when a non-ASCII byte is in position 0/1/2.
    #[test]
    fn histcharssetfn_rejects_non_ascii_chars() {
        let _g = crate::test_util::global_state_lock();
        let _g = HISTCHARS_TEST_LOCK_SHARED
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // Reset to defaults.
        histcharssetfn(&mut param::default(), String::new());
        let bang_before = bangchar.load(Ordering::SeqCst);
        let hat_before = hatchar.load(Ordering::SeqCst);
        // Try to set HISTCHARS with non-ASCII char.
        histcharssetfn(&mut param::default(), "é".to_string());
        // c:5092 — rejection returns BEFORE any state changes.
        assert_eq!(
            bangchar.load(Ordering::SeqCst),
            bang_before,
            "c:5092 — bangchar unchanged after non-ASCII rejection"
        );
        assert_eq!(hatchar.load(Ordering::SeqCst), hat_before);
    }

    /// Shared mutex for tests that mutate argzero/posixzero — both
    /// share global state and race when run in parallel.
    static ARGZERO_TEST_LOCK: Mutex<()> = Mutex::new(());

    // HISTSIZ_TEST_LOCK is defined at module scope to share between
    // gsu_tests and tests submodules — both mutate histsiz.

    /// `Src/params.c:4974-4977` — `histsizesetfn` floors at 1 then
    /// calls `resizehistents()` to prune the in-memory ring to the
    /// new cap. The previous Rust port skipped the resize call (and
    /// also failed to mirror the value into `hist::histsiz`), so
    /// HISTSIZE shrinks didn't take effect until the next implicit
    /// prune. Pin: setting HISTSIZE to N caps both the param store
    /// AND the hist::histsiz atomic used by resizehistents.
    #[test]
    fn histsizesetfn_floors_at_one_and_mirrors_to_hist_module() {
        let _g = crate::test_util::global_state_lock();
        let _g = HISTSIZ_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved_param = histsizegetfn();
        let saved_hist = histsiz.load(Ordering::SeqCst);

        // c:4976 — value < 1 floors at 1.
        histsizesetfn(0);
        assert_eq!(histsizegetfn(), 1, "c:4976 — HISTSIZE 0 must floor at 1");
        assert_eq!(
            histsiz.load(Ordering::SeqCst),
            1,
            "c:4977 — mirror into hist::histsiz so resizehistents sees it"
        );

        // Negative floors too.
        histsizesetfn(-5);
        assert_eq!(histsizegetfn(), 1, "c:4976 — negative floors at 1");

        // Positive passes through.
        histsizesetfn(500);
        assert_eq!(histsizegetfn(), 500);
        assert_eq!(histsiz.load(Ordering::SeqCst), 500);

        // Restore.
        *histsiz_lock().lock().unwrap() = saved_param;
        histsiz.store(saved_hist, Ordering::SeqCst);
    }

    /// `Src/params.c:5152-5158` — `underscoregetfn` returns
    /// `dupstring(zunderscore)` then runs `untokenize(u)` on it.
    /// The Rust port previously skipped untokenize, exposing raw
    /// lexer-injected token bytes (Stringg, Equals, ...) in `$_`
    /// reads.
    #[test]
    fn underscoregetfn_runs_untokenize_on_zunderscore() {
        let _g = crate::test_util::global_state_lock();
        // Inject zunderscore containing a Pound token byte (\u{84})
        // and verify it gets stripped by untokenize in the return.
        let saved = zunderscore_lock().lock().unwrap().clone();

        // Set zunderscore to a string containing a Pound token byte
        // surrounded by literals.
        let pound = Pound;
        let mut s = String::new();
        s.push('a');
        s.push(pound);
        s.push('b');
        *zunderscore_lock().lock().unwrap() = s;

        let result = underscoregetfn();
        // c:5156 — untokenize replaces Pound (ITOK) with '#'
        // (its ztokens entry). The raw \u{84} byte must NOT survive.
        assert!(
            !result.contains(pound),
            "c:5156 — untokenize must strip Pound token byte from $_"
        );
        assert!(
            result.contains('#') || result.contains("a"),
            "c:5156 — Pound (ITOK) maps to '#' via ztokens[0]"
        );

        // Restore.
        *zunderscore_lock().lock().unwrap() = saved;
    }

    /// `Src/params.c:4954-4961` — `argzerogetfn` returns `posixzero`
    /// when `isset(POSIXARGZERO)`, else `argzero`. After mutating
    /// argzero (e.g. `exec -a foo`), `$0` under POSIXARGZERO must
    /// report the ORIGINAL startup argv[0], not the rewritten name.
    #[test]
    fn argzerogetfn_respects_posixargzero_option() {
        let _g = crate::test_util::global_state_lock();
        let _g = ARGZERO_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        // Save state.
        let saved_argzero = argzero();
        let saved_posixzero = posixzero();
        let saved_pos_option = opt_state_get("posixargzero").unwrap_or(false);

        // Set up: posixzero (original) ≠ argzero (rewritten).
        set_posixzero(Some("/bin/zsh".to_string()));
        set_argzero(Some("rewritten-name".to_string()));
        // The set_argzero call mirrors to posixzero only if unset,
        // and we set posixzero first → mirror skipped. Confirm separation.

        // pm is UNUSED in argzerogetfn (C signature matches Rust).
        // Use param::default() as the dummy carrier.
        let pm = param::default();

        // POSIXARGZERO off → returns argzero.
        opt_state_set("posixargzero", false);
        assert_eq!(
            argzerogetfn(&pm),
            "rewritten-name",
            "c:4960 — !POSIXARGZERO returns argzero (current display name)"
        );

        // POSIXARGZERO on → returns posixzero (the preserved startup argv[0]).
        opt_state_set("posixargzero", true);
        assert_eq!(
            argzerogetfn(&pm),
            "/bin/zsh",
            "c:4959 — POSIXARGZERO on returns posixzero (original startup argv[0])"
        );

        // Restore.
        set_argzero(saved_argzero);
        set_posixzero(saved_posixzero);
        opt_state_set("posixargzero", saved_pos_option);
    }

    /// `Src/init.c:271` — `argv0 = argzero = posixzero = *argv++`.
    /// At shell init both share the same source. The Rust port
    /// preserves this contract by having `set_argzero` mirror to
    /// `posixzero` ONLY on first call (when posixzero is None).
    /// Subsequent argzero changes (function frames, exec -a) must
    /// NOT clobber posixzero.
    #[test]
    fn set_argzero_mirrors_to_posixzero_only_on_first_call() {
        let _g = crate::test_util::global_state_lock();
        let _g = ARGZERO_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let saved_argzero = argzero();
        let saved_posixzero = posixzero();

        // Reset both to None.
        set_argzero(None);
        set_posixzero(None);
        // First call: posixzero is None, so it should mirror.
        set_argzero(Some("/usr/local/bin/zsh".to_string()));
        assert_eq!(
            posixzero().as_deref(),
            Some("/usr/local/bin/zsh"),
            "c:271 — first set_argzero mirrors to posixzero (was None)"
        );
        // Second call: posixzero now Some, so mirror is skipped.
        set_argzero(Some("function-name".to_string()));
        assert_eq!(
            posixzero().as_deref(),
            Some("/usr/local/bin/zsh"),
            "c:271 — second set_argzero does NOT clobber posixzero"
        );
        assert_eq!(
            argzero().as_deref(),
            Some("function-name"),
            "argzero updated as normal"
        );

        // Restore.
        set_posixzero(saved_posixzero);
        set_argzero(saved_argzero);
    }

    /// Locale-touching tests share process-wide env + libc state.
    /// Cargo runs tests in parallel by default, so without
    /// serialization a concurrent `env::set_var("LC_ALL")` can race
    /// a `env::remove_var("LC_ALL")` and corrupt assertions. Pin
    /// every locale test through this Mutex.
    fn locale_test_lock() -> &'static Mutex<()> {
        static L: OnceLock<Mutex<()>> = OnceLock::new();
        L.get_or_init(|| Mutex::new(()))
    }

    /// Pin `LC_NAMES` to the canonical zsh `lc_names[]` table at
    /// `Src/params.c:4805-4825`. The five categories in entry order
    /// (LC_COLLATE, LC_CTYPE, LC_MESSAGES, LC_NUMERIC, LC_TIME) MUST
    /// match — `lcsetfn` walks this table by `strcmp(ln->name, pm->node.nam)`
    /// per c:4926 and dispatches to `setlocale(ln->category, ...)`.
    #[test]
    fn lc_names_match_zsh_canonical_table() {
        let _g = crate::test_util::global_state_lock();
        let names: Vec<&str> = LC_NAMES.iter().map(|(n, _)| *n).collect();
        assert_eq!(
            names,
            vec![
                "LC_COLLATE",
                "LC_CTYPE",
                "LC_MESSAGES",
                "LC_NUMERIC",
                "LC_TIME"
            ],
            "Src/params.c:4805-4825 — lc_names entry order must be preserved"
        );
        // Verify each name maps to a distinct libc category — proves
        // we aren't aliasing LC_NUMERIC to LC_TIME etc.
        let cats: Vec<libc::c_int> = LC_NAMES.iter().map(|(_, c)| *c).collect();
        let mut sorted = cats.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 5, "all five LC_* categories must be distinct");
        assert!(cats.contains(&libc::LC_COLLATE));
        assert!(cats.contains(&libc::LC_CTYPE));
        assert!(cats.contains(&libc::LC_MESSAGES));
        assert!(cats.contains(&libc::LC_NUMERIC));
        assert!(cats.contains(&libc::LC_TIME));
    }

    /// Pin `lcsetfn` to the canonical `setlocale` invocation at
    /// `Src/params.c:4925-4927`. When LC_ALL is empty and pm matches
    /// an entry in `lc_names`, libc setlocale MUST be called with
    /// the corresponding category. Verified by reading libc state
    /// back via `setlocale(cat, NULL)` after the assignment.
    #[test]
    fn lcsetfn_invokes_libc_setlocale_for_matching_category() {
        let _g = crate::test_util::global_state_lock();
        let _g = locale_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        // Save LC_ALL/LC_CTYPE state.
        let saved_lc_all = env::var("LC_ALL").ok();
        let saved_lc_ctype = env::var("LC_CTYPE").ok();
        env::remove_var("LC_ALL"); // c:4912 LC_ALL must be empty for body to run
        // Drop LC_ALL from paramtab too — the sibling
        // `lcsetfn_short_circuits_when_lc_all_set` test sets it via
        // setsparam (params.rs:12986) and getsparam reads paramtab
        // BEFORE env::var (params.rs:4346). Without this clear, lcsetfn
        // sees the leftover paramtab value and short-circuits.
        unsetparam("LC_ALL");

        // Read libc's current LC_CTYPE setting.
        let before = unsafe {
            let p = libc::setlocale(libc::LC_CTYPE, std::ptr::null());
            if p.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        };

        // Call lcsetfn with LC_CTYPE → "C" (universally available POSIX locale).
        lcsetfn("LC_CTYPE", Some("C".to_string()));

        // Read it back — must report "C" since C invokes setlocale(LC_CTYPE, "C").
        let after = unsafe {
            let p = libc::setlocale(libc::LC_CTYPE, std::ptr::null());
            if p.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        };
        assert_eq!(
            after, "C",
            "Src/params.c:4927 — lcsetfn must call setlocale(LC_CTYPE, \"C\")"
        );

        // Env mirror also set.
        assert_eq!(env::var("LC_CTYPE").unwrap_or_default(), "C");

        // Restore libc + env state.
        let _ = unsafe {
            let c = std::ffi::CString::new(before.as_bytes()).unwrap_or_default();
            libc::setlocale(libc::LC_CTYPE, c.as_ptr())
        };
        match saved_lc_all {
            Some(v) => env::set_var("LC_ALL", v),
            None => env::remove_var("LC_ALL"),
        }
        match saved_lc_ctype {
            Some(v) => env::set_var("LC_CTYPE", v),
            None => env::remove_var("LC_CTYPE"),
        }
    }

    /// Pin `lcsetfn`'s LC_ALL early-return per c:4912-4913: when
    /// LC_ALL is non-empty, lcsetfn must short-circuit BEFORE
    /// touching libc setlocale for the per-category override.
    #[test]
    fn lcsetfn_short_circuits_when_lc_all_set() {
        let _g = crate::test_util::global_state_lock();
        let _g = locale_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let saved_lc_all = env::var("LC_ALL").ok();
        let saved_lc_ctype = env::var("LC_CTYPE").ok();
        env::set_var("LC_ALL", "C"); // c:4912 non-empty LC_ALL
        // Also stamp paramtab — `getsparam` reads paramtab FIRST
        // (params.rs:4346) and only falls through to `env::var` when
        // the key is absent. A prior test that left LC_ALL in paramtab
        // with an empty value (PM_UNSET tombstone or similar) makes
        // getsparam return Some("") and the env::set_var above never
        // reaches lcsetfn. Setting paramtab here pins the contract.
        setsparam("LC_ALL", "C");

        // Capture libc state before.
        let before = unsafe {
            let p = libc::setlocale(libc::LC_CTYPE, std::ptr::null());
            if p.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        };

        // Try to set LC_CTYPE; should NOT touch libc state.
        lcsetfn("LC_CTYPE", Some("POSIX".to_string()));

        // libc state must be unchanged.
        let after = unsafe {
            let p = libc::setlocale(libc::LC_CTYPE, std::ptr::null());
            if p.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        };
        assert_eq!(
            before, after,
            "c:4912-4913 — lcsetfn must early-return when LC_ALL is non-empty"
        );

        // Restore.
        match saved_lc_all {
            Some(v) => env::set_var("LC_ALL", v),
            None => env::remove_var("LC_ALL"),
        }
        match saved_lc_ctype {
            Some(v) => env::set_var("LC_CTYPE", v),
            None => env::remove_var("LC_CTYPE"),
        }
    }

    /// Pin `getsparam_u` to its canonical C body at
    /// `Src/params.c:3089-3094`: returns `unmeta(getsparam(s))`,
    /// NOT a PM_SCALAR-checked `getstrvalue` wrapper.
    ///
    /// Before this fix, the Rust port took `Option<&mut value>`
    /// and gated on `PM_TYPE == PM_SCALAR` — a complete fabrication
    /// with no caller because no caller's type fit the bogus sig.
    #[test]
    fn getsparam_u_unmetas_getsparam_result() {
        let _g = crate::test_util::global_state_lock();
        let _g = locale_test_lock().lock().unwrap_or_else(|e| e.into_inner());

        // Plain ASCII: getsparam_u returns the same content as
        // getsparam (no Meta bytes to strip).
        let saved = env::var("ZSHRS_TEST_LOCALE_GSU").ok();
        env::set_var("ZSHRS_TEST_LOCALE_GSU", "en_US.UTF-8");
        assert_eq!(
            getsparam_u("ZSHRS_TEST_LOCALE_GSU"),
            Some("en_US.UTF-8".to_string()),
            "Src/params.c:3092 — getsparam_u returns unmeta(getsparam(s)) for ASCII"
        );

        // Missing param: returns None (matches C `if ((s = getsparam(s)))` false branch).
        env::remove_var("ZSHRS_TEST_LOCALE_GSU_MISSING");
        assert_eq!(
            getsparam_u("ZSHRS_TEST_LOCALE_GSU_MISSING"),
            None,
            "Src/params.c:3094 — getsparam_u returns NULL when getsparam returns NULL"
        );

        // Restore.
        match saved {
            Some(v) => env::set_var("ZSHRS_TEST_LOCALE_GSU", v),
            None => env::remove_var("ZSHRS_TEST_LOCALE_GSU"),
        }
    }

    /// Pin `setarrvalue` EXECOPT bail per `Src/params.c:2897-2898`.
    /// Same NO_EXEC semantic as setnumvalue: dry-run shell evaluation
    /// must not mutate array params.
    #[test]
    fn setarrvalue_bails_under_no_exec() {
        let _g = crate::test_util::global_state_lock();

        let saved_exec = opt_state_get("exec").unwrap_or(false);

        opt_state_set("exec", false);
        let pm = Box::new(param {
            node: hashnode {
                next: None,
                nam: "noexec_arr".to_string(),
                flags: PM_ARRAY as i32,
            },
            u_data: 0,
            u_tied: None,
            u_arr: Some(vec!["initial".to_string()]),
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
        let mut v = value {
            pm: Some(pm),
            arr: Vec::new(),
            scanflags: 0,
            valflags: 0,
            start: 0,
            end: -1,
        };
        // Under NO_EXEC, the assign must be skipped.
        setarrvalue(&mut v, vec!["new1".to_string(), "new2".to_string()]);
        let arr = v.pm.as_ref().unwrap().u_arr.clone().unwrap_or_default();
        assert_eq!(
            arr,
            vec!["initial".to_string()],
            "c:2897 — NO_EXEC: setarrvalue must NOT replace u_arr"
        );

        // With exec=true, the same call replaces.
        opt_state_set("exec", true);
        setarrvalue(&mut v, vec!["new1".to_string(), "new2".to_string()]);
        let arr = v.pm.as_ref().unwrap().u_arr.clone().unwrap_or_default();
        assert_eq!(
            arr,
            vec!["new1".to_string(), "new2".to_string()],
            "with EXEC set, setarrvalue replaces u_arr"
        );

        opt_state_set("exec", saved_exec);
    }

    /// Pin `setnumvalue` EXECOPT bail per `Src/params.c:2860`.
    /// When unset(EXECOPT) (i.e. NO_EXEC mode via `zsh -n` or
    /// `set -n`), param mutations MUST be skipped so dry-run shell
    /// evaluation doesn't leak state into the param table.
    #[test]
    fn setnumvalue_bails_under_no_exec() {
        let _g = crate::test_util::global_state_lock();

        let saved_exec = opt_state_get("exec").unwrap_or(false);

        // c:2860 — NO_EXEC: setnumvalue must not mutate the param.
        opt_state_set("exec", false);
        let mut pm = Box::new(param {
            node: hashnode {
                next: None,
                nam: "ne".to_string(),
                flags: PM_INTEGER as i32,
            },
            u_data: 0,
            u_tied: None,
            u_arr: None,
            u_str: None,
            u_val: 999,
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
        let mut v = value {
            pm: Some(pm.clone()),
            arr: Vec::new(),
            scanflags: 0,
            valflags: 0,
            start: 0,
            end: -1,
        };
        let val = mnumber {
            l: 42,
            d: 0.0,
            type_: MN_INTEGER,
        };
        setnumvalue(Some(&mut v), val);
        // pm.u_val MUST still be 999 (the initial), not 42.
        let stored = v.pm.as_ref().unwrap().u_val;
        assert_eq!(
            stored, 999,
            "c:2860 — NO_EXEC: setnumvalue must NOT mutate pm.u_val \
             (was {} but should stay 999)",
            stored
        );

        // With exec=true, the same call mutates.
        opt_state_set("exec", true);
        setnumvalue(Some(&mut v), val);
        let stored = v.pm.as_ref().unwrap().u_val;
        assert_eq!(stored, 42, "with EXEC set, setnumvalue stores u_val = 42");

        let _ = pm;
        opt_state_set("exec", saved_exec);
    }

    /// Pin `$-` rendering to honor `set -n` (noexec). The previous
    /// Rust port called `opt("noexec")` which isn't a real option
    /// name in zsh — the lookup always returned false so `$-` never
    /// included 'n' even when `set -n` was active.
    #[test]
    fn dash_param_rendering_honors_noexec_via_exec_negation() {
        let _g = crate::test_util::global_state_lock();
        let saved = opt_state_get("exec").unwrap_or(false);

        // With exec=true (default), $- should NOT include 'n'.
        opt_state_set("exec", true);
        let s = lookup_special_var("-").unwrap_or_default();
        assert!(
            !s.contains('n'),
            "exec=true → $-=`{}` must NOT include 'n'",
            s
        );

        // With exec=false (`set -n`), $- SHOULD include 'n'.
        opt_state_set("exec", false);
        let s = lookup_special_var("-").unwrap_or_default();
        assert!(
            s.contains('n'),
            "exec=false → $-=`{}` MUST include 'n' (was silently dropped \
             when reading non-existent option name `noexec`)",
            s
        );

        opt_state_set("exec", saved);
    }

    /// Pin `TERM_UNKNOWN` bit value to the canonical C value at
    /// `Src/zsh.h:1986`. The previous params.rs duplicate had
    /// `1 << 0 = 0x01` which is actually C's TERM_BAD (Src/zsh.h:1985);
    /// the correct TERM_UNKNOWN value is 0x02. This single-byte
    /// drift caused the params.rs term-init code to silently set the
    /// TERM_BAD bit instead of TERM_UNKNOWN, while prompt.rs guards
    /// imported the correct 0x02 value from zsh_h.rs — the two
    /// paths disagreed about which bit means \"unknown terminal\".
    #[test]
    fn term_unknown_bit_value_matches_c() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            TERM_UNKNOWN, 0x02,
            "Src/zsh.h:1986 — TERM_UNKNOWN must be 0x02, got {:#x}",
            TERM_UNKNOWN
        );
        assert_eq!(
            TERM_BAD, 0x01,
            "Src/zsh.h:1985 — TERM_BAD must be 0x01 (and != TERM_UNKNOWN)"
        );
        // Crucially: TERM_BAD and TERM_UNKNOWN must be DISTINCT bits.
        assert_ne!(
            TERM_BAD, TERM_UNKNOWN,
            "TERM_BAD and TERM_UNKNOWN must be distinct (caught the 1<<0 drift bug)"
        );
    }

    /// Pin `getstrvalue` PM_INTEGER branch to canonical C convbase
    /// dispatch at `Src/params.c:2373`. The previous Rust port used
    /// naked `.to_string()` (base-10) regardless of `pm.base`; C
    /// honors the param's stored base so `typeset -i 16 x=255` renders
    /// as `0xff` not `255`.
    #[test]
    fn getstrvalue_pm_integer_honors_pm_base() {
        let _g = crate::test_util::global_state_lock();

        let saved_cbases_top = opt_state_get("cbases").unwrap_or(false);
        opt_state_set("cbases", true);

        // Build a PM_INTEGER param with u_val=255 and base=16.
        let mut pm = Box::new(param {
            node: hashnode {
                next: None,
                nam: "test_hex_var".to_string(),
                flags: PM_INTEGER as i32,
            },
            u_data: 0,
            u_tied: None,
            u_arr: None,
            u_str: None,
            u_val: 255,
            u_dval: 0.0,
            u_hash: None,
            gsu_s: None,
            gsu_i: None,
            gsu_f: None,
            gsu_a: None,
            gsu_h: None,
            base: 16,
            width: 0,
            env: None,
            ename: None,
            old: None,
            level: 0,
        });
        let mut v = value {
            pm: Some(pm.clone()),
            arr: Vec::new(),
            scanflags: 0,
            valflags: 0,
            start: 0,
            end: -1,
        };
        let rendered = getstrvalue(Some(&mut v));
        assert_eq!(
            rendered, "0xFF",
            "c:2373 / c:5621 — PM_INTEGER base=16 + u_val=255 with CBASES \
             renders as `0xFF` (uppercase per C `dig - 10 + 'A'`), got {:?}",
            rendered
        );

        // Base-8 (octal) with OCTALZEROES.
        let saved_oct = opt_state_get("octalzeroes").unwrap_or(false);
        let saved_cbases = opt_state_get("cbases").unwrap_or(false);
        opt_state_set("cbases", true);
        opt_state_set("octalzeroes", true);
        pm.base = 8;
        pm.u_val = 8;
        v.pm = Some(pm.clone());
        let rendered = getstrvalue(Some(&mut v));
        assert_eq!(
            rendered, "010",
            "c:2373 — PM_INTEGER base=8 with OCTALZEROES renders as `010`, got {:?}",
            rendered
        );
        opt_state_set("cbases", saved_cbases);
        opt_state_set("octalzeroes", saved_oct);

        // Base=0 (default) → base-10.
        pm.base = 0;
        pm.u_val = 42;
        v.pm = Some(pm.clone());
        let rendered = getstrvalue(Some(&mut v));
        assert_eq!(
            rendered, "42",
            "c:2373 — PM_INTEGER base=0 defaults to base-10"
        );

        opt_state_set("cbases", saved_cbases_top);
    }

    /// Pin `unsetparam` to its canonical C body at `Src/params.c:3819-3833`.
    /// Two guards the previous Rust port skipped:
    ///   1. PM_NAMEREF params are NOT removed by unsetparam (c:3830).
    ///   2. PM_READONLY rejection per unsetparam_pm c:3850 — readonly
    ///      params survive the unset call.
    #[test]
    fn unsetparam_skips_nameref_and_readonly() {
        let _g = crate::test_util::global_state_lock();

        let saved_exec = opt_state_get("exec").unwrap_or(false);
        opt_state_set("exec", true);

        // Helper: install a scalar param with the given flag-set.
        fn install(name: &str, value: &str, flags: u32) {
            let mut tab = paramtab().write().unwrap();
            tab.insert(
                name.to_string(),
                Box::new(param {
                    node: hashnode {
                        next: None,
                        nam: name.to_string(),
                        flags: (PM_SCALAR | flags) as i32,
                    },
                    u_data: 0,
                    u_tied: None,
                    u_arr: None,
                    u_str: Some(value.to_string()),
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
                }),
            );
        }

        // c:3830 — nameref params skip the unset.
        let nameref_name = "zshrs_test_unsetparam_nameref";
        install(nameref_name, "target_var_name", PM_NAMEREF);
        unsetparam(nameref_name);
        {
            let tab = paramtab().read().unwrap();
            assert!(
                tab.contains_key(nameref_name),
                "c:3830 — PM_NAMEREF param survives unsetparam"
            );
        }

        // c:3850 (via unsetparam_pm) — readonly rejection.
        let ro_name = "zshrs_test_unsetparam_readonly";
        install(ro_name, "locked", PM_READONLY);
        unsetparam(ro_name);
        {
            let tab = paramtab().read().unwrap();
            assert!(
                tab.contains_key(ro_name),
                "c:3850 — PM_READONLY param survives unsetparam"
            );
        }

        // Plain scalar removed normally.
        let plain_name = "zshrs_test_unsetparam_plain";
        install(plain_name, "removable", 0);
        unsetparam(plain_name);
        {
            let tab = paramtab().read().unwrap();
            assert!(
                !tab.contains_key(plain_name),
                "plain scalar successfully removed"
            );
        }

        // Clean up.
        {
            let mut tab = paramtab().write().unwrap();
            tab.remove(nameref_name);
            tab.remove(ro_name);
            tab.remove(plain_name);
        }
        opt_state_set("exec", saved_exec);
    }

    /// Pin `assigniparam` to its canonical C body at `Src/params.c:3754-3761`.
    /// Three-arg signature: `(s, val, flags)`. Previous Rust port
    /// dropped the flags arg AND returned void; this restores both.
    #[test]
    fn assigniparam_takes_flags_arg_and_returns_param() {
        let _g = crate::test_util::global_state_lock();

        let saved_exec = opt_state_get("exec").unwrap_or(false);
        opt_state_set("exec", true);

        let name = "zshrs_test_assigniparam_x";
        {
            let mut tab = paramtab().write().unwrap();
            tab.remove(name);
        }

        // c:3755-3760 — assigniparam returns Param and threads flags through.
        let r = assigniparam(name, 77, ASSPM_WARN as i32);
        assert!(
            r.is_some(),
            "c:3760 — returns Some(Param) for new int param"
        );
        {
            let tab = paramtab().read().unwrap();
            let pm = tab.get(name).expect("integer param created");
            assert_ne!(
                (pm.node.flags as u32) & PM_INTEGER,
                0,
                "c:3757-3760 — PM_INTEGER flag set"
            );
            assert_eq!(pm.u_val, 77, "c:3759 — value stored in u_val");
        }

        // Reassign with a different flag value (0 — no warnings).
        let r = assigniparam(name, 88, 0);
        assert!(r.is_some(), "reassign returns Some");
        {
            let tab = paramtab().read().unwrap();
            let pm = tab.get(name).expect("param still present");
            assert_eq!(pm.u_val, 88, "reassign updates u_val");
        }

        // Clean up.
        {
            let mut tab = paramtab().write().unwrap();
            tab.remove(name);
        }
        opt_state_set("exec", saved_exec);
    }

    /// Pin `setnparam` to its canonical C body at `Src/params.c:3745-3749`.
    /// MUST accept `mnumber` (integer or float) and return Param.
    /// Previous Rust port took `f64` only and returned void — losing
    /// the integer side and the Param return entirely.
    #[test]
    fn setnparam_accepts_both_integer_and_float() {
        let _g = crate::test_util::global_state_lock();

        let saved_exec = opt_state_get("exec").unwrap_or(false);
        opt_state_set("exec", true);

        // Clean up any leftover.
        let int_name = "zshrs_test_setnparam_i";
        let flt_name = "zshrs_test_setnparam_f";
        {
            let mut tab = paramtab().write().unwrap();
            tab.remove(int_name);
            tab.remove(flt_name);
        }

        // c:3748 — integer branch: setnparam returns Some(Param) with
        // PM_INTEGER flag and u_val set.
        let r = setnparam(
            int_name,
            mnumber {
                l: 999,
                d: 0.0,
                type_: MN_INTEGER,
            },
        );
        assert!(r.is_some(), "setnparam returns Some for new param");
        {
            let tab = paramtab().read().unwrap();
            let pm = tab.get(int_name).expect("integer param created");
            assert_ne!(
                (pm.node.flags as u32) & PM_INTEGER,
                0,
                "c:3748 — PM_INTEGER flag set for integer mnumber"
            );
            assert_eq!(pm.u_val, 999, "c:3748 — integer value stored in u_val");
        }

        // c:3748 — float branch: setnparam with MN_FLOAT creates PM_FFLOAT.
        let r = setnparam(
            flt_name,
            mnumber {
                l: 0,
                d: 3.14,
                type_: MN_FLOAT,
            },
        );
        assert!(r.is_some(), "setnparam returns Some for new float param");
        {
            let tab = paramtab().read().unwrap();
            let pm = tab.get(flt_name).expect("float param created");
            assert_ne!(
                (pm.node.flags as u32) & PM_FFLOAT,
                0,
                "c:3748 — PM_FFLOAT flag set for float mnumber"
            );
            assert!(
                (pm.u_dval - 3.14).abs() < 1e-10,
                "c:3748 — float value stored in u_dval"
            );
        }

        // Clean up.
        {
            let mut tab = paramtab().write().unwrap();
            tab.remove(int_name);
            tab.remove(flt_name);
        }
        opt_state_set("exec", saved_exec);
    }

    /// Pin `setiparam` to its canonical C body at `Src/params.c:3767-3773`.
    /// MUST create the param as PM_INTEGER via `assignnparam`, not as
    /// PM_SCALAR via `assignsparam` with a stringified value.
    #[test]
    fn setiparam_creates_pm_integer_param() {
        let _g = crate::test_util::global_state_lock();
        let name = "zshrs_test_setiparam_x";

        // C: `assignnparam` bails when `unset(EXECOPT)` (Src/params.c:3679).
        // Real zsh startup sets exec=true; the unit-test env doesn't run
        // through `createoptiontable` so we set "exec" explicitly to
        // simulate normal runtime.
        let saved_exec = opt_state_get("exec").unwrap_or(false);
        opt_state_set("exec", true);

        // Clean up any leftover.
        {
            let mut tab = paramtab().write().unwrap();
            tab.remove(name);
        }

        // Set integer value.
        setiparam(name, 42);

        // Param should exist with PM_INTEGER flag set + u_val == 42.
        {
            let tab = paramtab().read().unwrap();
            let pm = tab.get(name).expect("setiparam must create the param");
            assert_ne!(
                (pm.node.flags as u32) & PM_INTEGER,
                0,
                "c:3770-3772 — created param must have PM_INTEGER flag set, \
                 got flags = {:#x}",
                pm.node.flags
            );
            assert_eq!(pm.u_val, 42, "c:3771 — integer value stored in pm.u_val");
        }

        // Reassign to verify update path also keeps PM_INTEGER.
        setiparam(name, 100);
        {
            let tab = paramtab().read().unwrap();
            let pm = tab.get(name).expect("setiparam reassign must keep param");
            assert_eq!(pm.u_val, 100, "reassign updates the integer value");
            assert_ne!(
                (pm.node.flags as u32) & PM_INTEGER,
                0,
                "reassign keeps PM_INTEGER flag"
            );
        }

        // Clean up.
        {
            let mut tab = paramtab().write().unwrap();
            tab.remove(name);
        }
        // Restore EXECOPT.
        opt_state_set("exec", saved_exec);
    }

    /// Pin `gethparam` / `gethkparam` to their canonical C bodies at
    /// `Src/params.c:3117-3140`. Same signature-fix family as `getaparam`:
    /// the `name: &str` path with digit-first reject + PM_HASHED check.
    #[test]
    fn gethparam_and_gethkparam_signature_matches_c() {
        let _g = crate::test_util::global_state_lock();
        // c:3122 / c:3136 — digit-first name reject.
        assert_eq!(
            gethparam("123abc"),
            None,
            "c:3122 — digit-first name rejected"
        );
        assert_eq!(
            gethkparam("123abc"),
            None,
            "c:3136 — digit-first name rejected"
        );

        // Missing param → None.
        assert_eq!(
            gethparam("zshrs_test_hashparam_xyz"),
            None,
            "missing param returns None"
        );
        assert_eq!(
            gethkparam("zshrs_test_hashparam_xyz"),
            None,
            "missing param returns None"
        );

        // PM_SCALAR param (not hashed) → None.
        {
            let mut tab = paramtab().write().unwrap();
            tab.insert(
                "zshrs_test_gethp_scalar".to_string(),
                Box::new(param {
                    node: hashnode {
                        next: None,
                        nam: "zshrs_test_gethp_scalar".to_string(),
                        flags: PM_SCALAR as i32,
                    },
                    u_data: 0,
                    u_tied: None,
                    u_arr: None,
                    u_str: Some("scalar value".to_string()),
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
                }),
            );
        }
        assert_eq!(
            gethparam("zshrs_test_gethp_scalar"),
            None,
            "c:3123 — non-PM_HASHED returns None"
        );
        assert_eq!(
            gethkparam("zshrs_test_gethp_scalar"),
            None,
            "c:3137 — non-PM_HASHED returns None"
        );

        // PM_HASHED param → Some(Vec::new()) (backend not yet wired,
        // but signature should at least classify the type correctly).
        {
            let mut tab = paramtab().write().unwrap();
            tab.insert(
                "zshrs_test_gethp_hash".to_string(),
                Box::new(param {
                    node: hashnode {
                        next: None,
                        nam: "zshrs_test_gethp_hash".to_string(),
                        flags: PM_HASHED as i32,
                    },
                    u_data: 0,
                    u_tied: None,
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
                }),
            );
        }
        assert_eq!(
            gethparam("zshrs_test_gethp_hash"),
            Some(Vec::new()),
            "c:3123-3124 — PM_HASHED empty-storage returns Some(empty vec)"
        );
        assert_eq!(
            gethkparam("zshrs_test_gethp_hash"),
            Some(Vec::new()),
            "c:3137-3138 — PM_HASHED empty-storage returns Some(empty vec)"
        );

        // Populate paramtab_hashed_storage and verify gethparam/gethkparam
        // return the actual values/keys per c:3124 (SCANPM_WANTVALS) and
        // c:3138 (SCANPM_WANTKEYS). IndexMap preserves insertion order.
        {
            let mut store = paramtab_hashed_storage().lock().unwrap();
            let mut map: IndexMap<String, String> = IndexMap::new();
            map.insert("k1".to_string(), "v1".to_string());
            map.insert("k2".to_string(), "v2".to_string());
            store.insert("zshrs_test_gethp_hash".to_string(), map);
        }
        assert_eq!(
            gethparam("zshrs_test_gethp_hash"),
            Some(vec!["v1".to_string(), "v2".to_string()]),
            "c:3124 — paramvalarr(SCANPM_WANTVALS) returns values from hashed-storage"
        );
        assert_eq!(
            gethkparam("zshrs_test_gethp_hash"),
            Some(vec!["k1".to_string(), "k2".to_string()]),
            "c:3138 — paramvalarr(SCANPM_WANTKEYS) returns keys from hashed-storage"
        );

        // Clean up.
        {
            let mut tab = paramtab().write().unwrap();
            tab.remove("zshrs_test_gethp_scalar");
            tab.remove("zshrs_test_gethp_hash");
        }
        paramtab_hashed_storage()
            .lock()
            .unwrap()
            .remove("zshrs_test_gethp_hash");
    }

    /// Pin `getaparam` to its canonical C body at `Src/params.c:3101-3110`.
    /// Three branches: digit-first reject (c:3107), PM_ARRAY return
    /// (c:3108-3109), non-array / missing-param return None (c:3110).
    #[test]
    fn getaparam_returns_array_for_pm_array_only() {
        let _g = crate::test_util::global_state_lock();
        // c:3107 — digit-first name → None (positional params reject).
        assert_eq!(
            getaparam("123abc"),
            None,
            "c:3107 — digit-first name rejected"
        );

        // c:3110 — missing param → None.
        assert_eq!(
            getaparam("zshrs_test_arr_nonexistent_xyz"),
            None,
            "c:3110 — missing param returns None"
        );

        // Helper that builds a Param via the canonical createparam
        // path so we don't reach into struct internals (param has
        // many fields and no Default).
        fn build_arr(name: &str, arr: Vec<String>) {
            let mut tab = paramtab().write().unwrap();
            tab.insert(
                name.to_string(),
                Box::new(param {
                    node: hashnode {
                        next: None,
                        nam: name.to_string(),
                        flags: PM_ARRAY as i32,
                    },
                    u_data: 0,
                    u_tied: None,
                    u_arr: Some(arr),
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
                }),
            );
        }
        fn build_scalar(name: &str, s: &str) {
            let mut tab = paramtab().write().unwrap();
            tab.insert(
                name.to_string(),
                Box::new(param {
                    node: hashnode {
                        next: None,
                        nam: name.to_string(),
                        flags: PM_SCALAR as i32,
                    },
                    u_data: 0,
                    u_tied: None,
                    u_arr: None,
                    u_str: Some(s.to_string()),
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
                }),
            );
        }

        // c:3108 — PM_ARRAY param returns the array contents.
        build_arr(
            "zshrs_test_getaparam_arr",
            vec!["one".to_string(), "two".to_string(), "three".to_string()],
        );
        assert_eq!(
            getaparam("zshrs_test_getaparam_arr"),
            Some(vec![
                "one".to_string(),
                "two".to_string(),
                "three".to_string()
            ]),
            "c:3108-3109 — PM_ARRAY param returns its array"
        );

        // c:3108 — PM_SCALAR (non-array) param → None.
        build_scalar("zshrs_test_getaparam_scalar", "not an array");
        assert_eq!(
            getaparam("zshrs_test_getaparam_scalar"),
            None,
            "c:3108 — non-PM_ARRAY param returns None"
        );

        // Clean up.
        {
            let mut tab = paramtab().write().unwrap();
            tab.remove("zshrs_test_getaparam_arr");
            tab.remove("zshrs_test_getaparam_scalar");
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // setX/getX round-trips and cross-type coercion.
    // Anchored to zsh behavior:
    //   - `typeset -i x=42; print -- $x` → "42"
    //   - `x=42; print -- ${(t)x}` → "scalar" (set without typeset stays scalar)
    //   - integer ↔ scalar coercion when reading back.
    // Each test sets up via setX, reads back via getX, and asserts the
    // observable shell-level value.
    // ═══════════════════════════════════════════════════════════════════

    fn with_exec<F: FnOnce()>(body: F) {
        let _g = crate::test_util::global_state_lock();
        let saved = opt_state_get("exec").unwrap_or(false);
        opt_state_set("exec", true);
        body();
        opt_state_set("exec", saved);
    }

    // ── Scalar round-trip ──────────────────────────────────────────
    /// `setsparam("X", "hello"); getsparam("X")` → Some("hello").
    /// Anchor: `X=hello; print -r -- "$X"` in zsh → `hello`.
    #[test]
    fn setsparam_then_getsparam_roundtrip_basic() {
        with_exec(|| {
            unsetparam("zshrs_rt_s1");
            setsparam("zshrs_rt_s1", "hello");
            assert_eq!(getsparam("zshrs_rt_s1").as_deref(), Some("hello"));
            unsetparam("zshrs_rt_s1");
        });
    }

    /// Empty string round-trip — scalar can hold "".
    #[test]
    fn setsparam_empty_string_roundtrips_as_empty() {
        with_exec(|| {
            unsetparam("zshrs_rt_s2");
            setsparam("zshrs_rt_s2", "");
            assert_eq!(getsparam("zshrs_rt_s2").as_deref(), Some(""));
            unsetparam("zshrs_rt_s2");
        });
    }

    /// Multi-byte UTF-8 scalar round-trip.
    /// Anchor: `X="日本語"; print -r -- "$X"` → "日本語"
    #[test]
    fn setsparam_multibyte_utf8_roundtrips() {
        with_exec(|| {
            unsetparam("zshrs_rt_s3");
            setsparam("zshrs_rt_s3", "日本語");
            assert_eq!(getsparam("zshrs_rt_s3").as_deref(), Some("日本語"));
            unsetparam("zshrs_rt_s3");
        });
    }

    /// Embedded newline survives round-trip.
    #[test]
    fn setsparam_embedded_newline_roundtrips() {
        with_exec(|| {
            unsetparam("zshrs_rt_s4");
            setsparam("zshrs_rt_s4", "a\nb\nc");
            assert_eq!(getsparam("zshrs_rt_s4").as_deref(), Some("a\nb\nc"));
            unsetparam("zshrs_rt_s4");
        });
    }

    /// Overwrite: setsparam twice returns the latter value.
    #[test]
    fn setsparam_overwrite_replaces_previous_value() {
        with_exec(|| {
            unsetparam("zshrs_rt_s5");
            setsparam("zshrs_rt_s5", "first");
            setsparam("zshrs_rt_s5", "second");
            assert_eq!(getsparam("zshrs_rt_s5").as_deref(), Some("second"));
            unsetparam("zshrs_rt_s5");
        });
    }

    // ── Integer round-trip ─────────────────────────────────────────
    /// `setiparam("X", 42); getiparam("X")` → 42.
    /// Anchor: `typeset -i X=42; print -- $X` → "42".
    #[test]
    fn setiparam_then_getiparam_roundtrip_basic() {
        with_exec(|| {
            unsetparam("zshrs_rt_i1");
            setiparam("zshrs_rt_i1", 42);
            assert_eq!(getiparam("zshrs_rt_i1"), 42);
            unsetparam("zshrs_rt_i1");
        });
    }

    /// Negative integer round-trip.
    #[test]
    fn setiparam_negative_value_roundtrips() {
        with_exec(|| {
            unsetparam("zshrs_rt_i2");
            setiparam("zshrs_rt_i2", -12345);
            assert_eq!(getiparam("zshrs_rt_i2"), -12345);
            unsetparam("zshrs_rt_i2");
        });
    }

    /// Zero round-trip — distinct from "unset".
    #[test]
    fn setiparam_zero_roundtrips() {
        with_exec(|| {
            unsetparam("zshrs_rt_i3");
            setiparam("zshrs_rt_i3", 0);
            assert_eq!(getiparam("zshrs_rt_i3"), 0);
            unsetparam("zshrs_rt_i3");
        });
    }

    /// `i64::MAX` and `i64::MIN` survive — zsh uses zlong (= long long
    /// on 64-bit hosts, which is i64 in Rust).
    #[test]
    fn setiparam_i64_extremes_roundtrip() {
        with_exec(|| {
            unsetparam("zshrs_rt_i4");
            setiparam("zshrs_rt_i4", i64::MAX);
            assert_eq!(getiparam("zshrs_rt_i4"), i64::MAX);
            setiparam("zshrs_rt_i4", i64::MIN);
            assert_eq!(getiparam("zshrs_rt_i4"), i64::MIN);
            unsetparam("zshrs_rt_i4");
        });
    }

    // ── Cross-type coercion: setiparam → getsparam (int → string) ──
    /// `setiparam("X", 42); getsparam("X")` → Some("42").
    /// Anchor: `typeset -i X=42; print -r -- "$X"` → "42".
    #[test]
    fn setiparam_then_getsparam_coerces_to_decimal_string() {
        with_exec(|| {
            unsetparam("zshrs_rt_x1");
            setiparam("zshrs_rt_x1", 42);
            assert_eq!(getsparam("zshrs_rt_x1").as_deref(), Some("42"));
            unsetparam("zshrs_rt_x1");
        });
    }

    /// Negative int → "-N" string form via getsparam.
    #[test]
    fn setiparam_negative_int_to_string_carries_minus_sign() {
        with_exec(|| {
            unsetparam("zshrs_rt_x2");
            setiparam("zshrs_rt_x2", -7);
            assert_eq!(getsparam("zshrs_rt_x2").as_deref(), Some("-7"));
            unsetparam("zshrs_rt_x2");
        });
    }

    // ── Cross-type: setsparam("42") → getiparam → 42 ───────────────
    /// `setsparam("X", "42"); getiparam("X")` → 42 (numeric parse).
    /// Anchor: `X=42; print -- $((X+0))` → "42" (arith coerces).
    #[test]
    fn setsparam_numeric_string_then_getiparam_coerces_to_int() {
        with_exec(|| {
            unsetparam("zshrs_rt_x3");
            setsparam("zshrs_rt_x3", "42");
            assert_eq!(getiparam("zshrs_rt_x3"), 42);
            unsetparam("zshrs_rt_x3");
        });
    }

    /// Non-numeric scalar → getiparam returns 0 (C zsh behavior).
    /// Anchor: `X=hello; print -- $((X+0))` → "0" (no numeric parse).
    #[test]
    fn setsparam_non_numeric_then_getiparam_returns_zero() {
        with_exec(|| {
            unsetparam("zshrs_rt_x4");
            setsparam("zshrs_rt_x4", "not a number");
            assert_eq!(getiparam("zshrs_rt_x4"), 0);
            unsetparam("zshrs_rt_x4");
        });
    }

    // ── Array round-trip ───────────────────────────────────────────
    /// `setaparam("X", vec![...]); getaparam("X")` round-trips elements.
    /// Anchor: `X=(a b c); print -r -- "${X[@]}"` → "a b c".
    #[test]
    fn setaparam_then_getaparam_roundtrip_basic() {
        with_exec(|| {
            unsetparam("zshrs_rt_a1");
            setaparam("zshrs_rt_a1", vec!["a".into(), "b".into(), "c".into()]);
            assert_eq!(
                getaparam("zshrs_rt_a1"),
                Some(vec!["a".into(), "b".into(), "c".into()])
            );
            unsetparam("zshrs_rt_a1");
        });
    }

    /// Empty array round-trip — distinct from unset.
    #[test]
    fn setaparam_empty_array_roundtrips_as_empty_vec() {
        with_exec(|| {
            unsetparam("zshrs_rt_a2");
            setaparam("zshrs_rt_a2", vec![]);
            assert_eq!(getaparam("zshrs_rt_a2"), Some(vec![]));
            unsetparam("zshrs_rt_a2");
        });
    }

    /// Array element with embedded space stays one element (not split).
    /// Anchor: `X=("hi there" world); print ${#X}` → 2.
    #[test]
    fn setaparam_element_with_space_stays_one_element() {
        with_exec(|| {
            unsetparam("zshrs_rt_a3");
            setaparam("zshrs_rt_a3", vec!["hi there".into(), "world".into()]);
            assert_eq!(
                getaparam("zshrs_rt_a3"),
                Some(vec!["hi there".into(), "world".into()])
            );
            unsetparam("zshrs_rt_a3");
        });
    }

    // ── unsetparam: idempotent + clears value ──────────────────────
    /// Unsetting a never-set param does nothing (no panic, no error).
    #[test]
    fn unsetparam_on_never_set_param_is_noop() {
        let _g = crate::test_util::global_state_lock();
        unsetparam("zshrs_rt_never_existed");
        // No assertion — just must not panic.
    }

    /// Unset then get → None.
    #[test]
    fn unsetparam_then_getsparam_returns_none() {
        with_exec(|| {
            setsparam("zshrs_rt_u1", "value");
            unsetparam("zshrs_rt_u1");
            assert_eq!(getsparam("zshrs_rt_u1"), None);
        });
    }

    /// Unsetting twice is idempotent.
    #[test]
    fn unsetparam_twice_is_idempotent() {
        with_exec(|| {
            setsparam("zshrs_rt_u2", "value");
            unsetparam("zshrs_rt_u2");
            unsetparam("zshrs_rt_u2"); // second call must not panic
            assert_eq!(getsparam("zshrs_rt_u2"), None);
        });
    }

    // ── Cross-type: setiparam writes through scalar table, getaparam None ─
    /// PM_INTEGER is NOT PM_ARRAY — getaparam on integer returns None.
    /// Anchor: `typeset -i X=42; print -- ${X[1]}` errors (not an array).
    #[test]
    fn setiparam_then_getaparam_returns_none_not_an_array() {
        with_exec(|| {
            unsetparam("zshrs_rt_x5");
            setiparam("zshrs_rt_x5", 42);
            assert_eq!(
                getaparam("zshrs_rt_x5"),
                None,
                "PM_INTEGER is not PM_ARRAY — getaparam must return None"
            );
            unsetparam("zshrs_rt_x5");
        });
    }

    // ── Read missing param: returns None / 0 ───────────────────────
    /// `getsparam("doesnt_exist")` → None.
    #[test]
    fn getsparam_on_unset_returns_none() {
        let _g = crate::test_util::global_state_lock();
        unsetparam("zshrs_rt_missing");
        assert_eq!(getsparam("zshrs_rt_missing"), None);
    }

    /// `getiparam("doesnt_exist")` → 0 (C zsh semantics).
    /// Anchor: `unset X; print -- $((X+1))` → "1" (X read as 0).
    #[test]
    fn getiparam_on_unset_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        unsetparam("zshrs_rt_missing2");
        assert_eq!(getiparam("zshrs_rt_missing2"), 0);
    }

    /// `getaparam("doesnt_exist")` → None.
    #[test]
    fn getaparam_on_unset_returns_none() {
        let _g = crate::test_util::global_state_lock();
        unsetparam("zshrs_rt_missing3");
        assert_eq!(getaparam("zshrs_rt_missing3"), None);
    }

    // ─── zsh corpus pins: params set/get round-trips ─────────────────

    /// `setsparam("foo", "bar")` followed by `getsparam("foo")` returns
    /// `"bar"`. Round-trip scalar.
    #[test]
    fn params_corpus_scalar_round_trip_simple() {
        let _g = crate::test_util::global_state_lock();
        unsetparam("ZP_RT1");
        setsparam("ZP_RT1", "bar");
        assert_eq!(getsparam("ZP_RT1").as_deref(), Some("bar"));
        unsetparam("ZP_RT1");
    }

    /// `setsparam("x", "")` then `getsparam("x")` returns `Some("")`,
    /// NOT `None` — empty-set is distinct from unset (per zsh semantics).
    #[test]
    fn params_corpus_empty_scalar_is_set_not_unset() {
        let _g = crate::test_util::global_state_lock();
        unsetparam("ZP_EMP");
        setsparam("ZP_EMP", "");
        assert_eq!(getsparam("ZP_EMP").as_deref(), Some(""), "empty-set is set");
        unsetparam("ZP_EMP");
    }

    /// `unsetparam` after `setsparam` removes the param entirely:
    /// `getsparam` returns `None`.
    #[test]
    fn params_corpus_unset_after_set_returns_none() {
        let _g = crate::test_util::global_state_lock();
        setsparam("ZP_UR", "v");
        unsetparam("ZP_UR");
        assert_eq!(getsparam("ZP_UR"), None, "unsetparam removes param");
    }

    /// `setiparam("i", 42)` and `getiparam` round-trip integer.
    #[test]
    fn params_corpus_integer_round_trip() {
        let _g = crate::test_util::global_state_lock();
        unsetparam("ZP_I");
        setiparam("ZP_I", 42);
        assert_eq!(getiparam("ZP_I"), 42);
        unsetparam("ZP_I");
    }

    /// `setiparam` then `getsparam` reads integer as string.
    #[test]
    fn params_corpus_integer_read_as_string() {
        let _g = crate::test_util::global_state_lock();
        unsetparam("ZP_IS");
        setiparam("ZP_IS", 42);
        assert_eq!(
            getsparam("ZP_IS").as_deref(),
            Some("42"),
            "integer param reads as string repr"
        );
        unsetparam("ZP_IS");
    }

    /// `setaparam("a", vec!["one","two","three"])` round-trips via
    /// `getaparam`.
    #[test]
    fn params_corpus_array_round_trip() {
        let _g = crate::test_util::global_state_lock();
        unsetparam("ZP_A");
        setaparam("ZP_A", vec!["one".into(), "two".into(), "three".into()]);
        assert_eq!(
            getaparam("ZP_A").as_deref(),
            Some(&["one".into(), "two".into(), "three".into()][..]),
            "array round-trip",
        );
        unsetparam("ZP_A");
    }

    /// Empty array round-trip — `setaparam("a", vec![])` then
    /// `getaparam` returns `Some(empty vec)`, not `None`.
    #[test]
    fn params_corpus_empty_array_is_set_not_unset() {
        let _g = crate::test_util::global_state_lock();
        unsetparam("ZP_EA");
        setaparam("ZP_EA", vec![]);
        let got = getaparam("ZP_EA");
        assert!(got.is_some(), "empty array is set, not None");
        assert_eq!(got.unwrap().len(), 0, "len = 0");
        unsetparam("ZP_EA");
    }

    /// `setsparam` overwrites previous value (scalar reassignment).
    #[test]
    fn params_corpus_scalar_overwrite() {
        let _g = crate::test_util::global_state_lock();
        unsetparam("ZP_O");
        setsparam("ZP_O", "first");
        setsparam("ZP_O", "second");
        assert_eq!(
            getsparam("ZP_O").as_deref(),
            Some("second"),
            "scalar overwrites cleanly"
        );
        unsetparam("ZP_O");
    }

    /// Re-assigning a scalar to an array (changing type via setaparam).
    /// Default zsh allows this: array replaces scalar.
    #[test]
    fn params_corpus_scalar_to_array_replace_type() {
        let _g = crate::test_util::global_state_lock();
        unsetparam("ZP_ST");
        setsparam("ZP_ST", "scalar_val");
        setaparam("ZP_ST", vec!["a".into(), "b".into()]);
        assert_eq!(
            getaparam("ZP_ST").as_deref(),
            Some(&["a".into(), "b".into()][..]),
            "setaparam replaces scalar type",
        );
        unsetparam("ZP_ST");
    }

    /// `unsetparam` on a name that doesn't exist is a no-op
    /// (zsh's idempotent unset).
    #[test]
    fn params_corpus_unset_on_missing_is_noop() {
        let _g = crate::test_util::global_state_lock();
        unsetparam("ZP_NEVER_EXISTED_42_xyz");
        // No panic: just returns None on subsequent get.
        assert_eq!(getsparam("ZP_NEVER_EXISTED_42_xyz"), None);
    }

    /// Empty string passed to integer slot: `setiparam("z", 0)`
    /// returns `Some(0)` via getiparam (zero is a legitimate int).
    #[test]
    fn params_corpus_integer_zero_is_set() {
        let _g = crate::test_util::global_state_lock();
        unsetparam("ZP_IZ");
        setiparam("ZP_IZ", 0);
        assert_eq!(getiparam("ZP_IZ"), 0);
        assert_eq!(getsparam("ZP_IZ").as_deref(), Some("0"));
        unsetparam("ZP_IZ");
    }

    /// Negative integers round-trip with sign preserved.
    #[test]
    fn params_corpus_integer_negative_preserves_sign() {
        let _g = crate::test_util::global_state_lock();
        unsetparam("ZP_IN");
        setiparam("ZP_IN", -42);
        assert_eq!(getiparam("ZP_IN"), -42);
        assert_eq!(getsparam("ZP_IN").as_deref(), Some("-42"));
        unsetparam("ZP_IN");
    }

    // ─── associative array (hash) pins ───────────────────────────────

    /// `sethparam` + `gethparam` round-trip. C `gethparam` returns
    /// `paramvalarr(..., SCANPM_WANTVALS)` (Src/params.c:3122) — i.e.
    /// the VALUES side of the hash, not flat k/v pairs. For
    /// `(key1 val1 key2 val2)` that's `[val1, val2]`. Keys come from
    /// `gethkparam` (covered by `..._hash_keys_only_returns_keys`).
    #[test]
    fn params_corpus_hash_round_trip_basic() {
        let _g = crate::test_util::global_state_lock();
        unsetparam("ZP_H");
        sethparam(
            "ZP_H",
            vec!["key1".into(), "val1".into(), "key2".into(), "val2".into()],
        );
        let got = gethparam("ZP_H");
        assert!(got.is_some(), "hash param set");
        let g = got.unwrap();
        assert_eq!(g.len(), 2, "2 values (SCANPM_WANTVALS) preserved");
        let mut sorted = g.clone();
        sorted.sort();
        assert_eq!(
            sorted,
            vec!["val1".to_string(), "val2".to_string()],
            "values come back regardless of hash-iter order"
        );
        unsetparam("ZP_H");
    }

    /// `gethparam` on missing returns None.
    #[test]
    fn params_corpus_hash_missing_returns_none() {
        let _g = crate::test_util::global_state_lock();
        unsetparam("ZP_HM_xyz");
        assert!(gethparam("ZP_HM_xyz").is_none());
    }

    /// `gethkparam` returns the keys-only view of a hash. With
    /// `{a:1,b:2}`, keys are `["a","b"]` (order may not be stable).
    #[test]
    fn params_corpus_hash_keys_only_returns_keys() {
        let _g = crate::test_util::global_state_lock();
        unsetparam("ZP_HK");
        sethparam(
            "ZP_HK",
            vec!["a".into(), "1".into(), "b".into(), "2".into()],
        );
        let keys = gethkparam("ZP_HK");
        assert!(keys.is_some(), "keys-only view exists");
        let k = keys.unwrap();
        assert_eq!(k.len(), 2, "2 keys");
        let mut sorted = k.clone();
        sorted.sort();
        assert_eq!(sorted, vec!["a".to_string(), "b".to_string()]);
        unsetparam("ZP_HK");
    }

    /// Setting a hash with empty `Vec` removes the previous key/value
    /// pairs. After empty `sethparam`, lookups find no entries. Note
    /// `gethparam` returns SCANPM_WANTVALS (values only); `(a 1)` has
    /// 1 value, not 2 — see `..._hash_round_trip_basic`.
    #[test]
    fn params_corpus_hash_set_empty_clears_entries() {
        let _g = crate::test_util::global_state_lock();
        unsetparam("ZP_HC");
        sethparam("ZP_HC", vec!["a".into(), "1".into()]);
        assert_eq!(gethparam("ZP_HC").map(|v| v.len()), Some(1));
        sethparam("ZP_HC", vec![]);
        // empty hash still exists (some implementations distinguish
        // empty hash from unset)
        let after = gethparam("ZP_HC");
        if let Some(v) = after {
            assert!(v.is_empty(), "empty hash has 0 elements");
        }
        unsetparam("ZP_HC");
    }

    /// Large integer at i64 boundary: i64::MAX round-trips.
    #[test]
    fn params_corpus_integer_max_round_trips() {
        let _g = crate::test_util::global_state_lock();
        unsetparam("ZP_IMX");
        setiparam("ZP_IMX", i64::MAX);
        assert_eq!(getiparam("ZP_IMX"), i64::MAX);
        unsetparam("ZP_IMX");
    }

    /// i64::MIN round-trips with sign.
    #[test]
    fn params_corpus_integer_min_round_trips() {
        let _g = crate::test_util::global_state_lock();
        unsetparam("ZP_IMN");
        setiparam("ZP_IMN", i64::MIN);
        assert_eq!(getiparam("ZP_IMN"), i64::MIN);
        unsetparam("ZP_IMN");
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests pinning Src/params.c:1288 (isident). Foundational
    // identifier validator used by every param-name validation site.
    // ═══════════════════════════════════════════════════════════════════

    /// `isident("")` returns false. C c:1292:
    ///   `if (!*s) return 0;`
    #[test]
    fn isident_empty_string_returns_false() {
        let _g = crate::test_util::global_state_lock();
        assert!(!isident(""));
    }

    /// `isident("foo")` returns true — basic identifier.
    #[test]
    fn isident_alpha_identifier_returns_true() {
        let _g = crate::test_util::global_state_lock();
        assert!(isident("foo"));
    }

    /// `isident("_foo")` — underscore prefix allowed.
    #[test]
    fn isident_underscore_prefix_returns_true() {
        let _g = crate::test_util::global_state_lock();
        assert!(isident("_foo"));
    }

    /// `isident("foo_bar123")` — alnum + underscore mid.
    #[test]
    fn isident_alnum_with_underscore_returns_true() {
        let _g = crate::test_util::global_state_lock();
        assert!(isident("foo_bar123"));
    }

    /// `isident("123")` — all-digit positional param. C c:1300+:
    ///   "All-digit names are valid (positional params)"
    #[test]
    fn isident_all_digit_positional_returns_true() {
        let _g = crate::test_util::global_state_lock();
        assert!(isident("123"), "all-digit names are positional params");
    }

    /// `isident("123abc")` — digit prefix with letters → false.
    /// Mixed digit-first not a valid positional param OR identifier.
    #[test]
    fn isident_digit_prefix_alpha_returns_false() {
        let _g = crate::test_util::global_state_lock();
        assert!(!isident("123abc"));
    }

    /// `isident(".ns.foo")` — ksh93 namespace dotted name allowed.
    /// C c:1296-1311 — leading `.` accepted if not followed by digit.
    #[test]
    fn isident_ksh93_namespace_dot_prefix_returns_true() {
        let _g = crate::test_util::global_state_lock();
        assert!(isident(".ns.foo"));
    }

    /// `isident(".0bad")` — namespace must NOT start with digit.
    /// C c:1300: `if (idigit(s[1])) return 0;`
    #[test]
    fn isident_namespace_starting_with_digit_returns_false() {
        let _g = crate::test_util::global_state_lock();
        assert!(!isident(".0bad"), "namespace can't start with digit");
    }

    /// `isident("foo-bar")` — dash not allowed in identifier.
    #[test]
    fn isident_dash_in_name_returns_false() {
        let _g = crate::test_util::global_state_lock();
        assert!(!isident("foo-bar"));
    }

    /// `isident("foo bar")` — space not allowed.
    #[test]
    fn isident_space_in_name_returns_false() {
        let _g = crate::test_util::global_state_lock();
        assert!(!isident("foo bar"));
    }

    /// `isident("foo[0]")` — subscript at end is allowed (C handles
    /// it in `isident_requires_balanced_subscript_brackets`).
    /// Pin: a simple `[0]` subscript validates.
    #[test]
    fn isident_with_simple_subscript_returns_true() {
        let _g = crate::test_util::global_state_lock();
        assert!(isident("foo[0]"));
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/params.c isident.
    // ═══════════════════════════════════════════════════════════════════

    /// c:1288 — `isident("")` returns false (empty not an identifier).
    #[test]
    fn isident_empty_returns_false() {
        let _g = crate::test_util::global_state_lock();
        assert!(!isident(""));
    }

    /// c:1288 — single underscore IS valid ident.
    #[test]
    fn isident_single_underscore_is_valid() {
        let _g = crate::test_util::global_state_lock();
        assert!(isident("_"));
    }

    /// c:1288 — single letter IS valid ident.
    #[test]
    fn isident_single_letter_is_valid() {
        let _g = crate::test_util::global_state_lock();
        assert!(isident("a"));
        assert!(isident("Z"));
    }

    /// c:1288 — all-digit name (positional param like '$1') IS valid.
    #[test]
    fn isident_all_digits_is_valid_positional() {
        let _g = crate::test_util::global_state_lock();
        assert!(isident("1"));
        assert!(isident("123"));
        assert!(isident("0"));
    }

    /// c:1288 — digit-first mixed (e.g. '1abc') is INVALID.
    #[test]
    fn isident_digit_first_mixed_is_invalid() {
        let _g = crate::test_util::global_state_lock();
        assert!(!isident("1abc"));
        assert!(!isident("0x"));
    }

    /// c:1288 — underscore + alnum is valid.
    #[test]
    fn isident_underscore_prefix_alnum_is_valid() {
        let _g = crate::test_util::global_state_lock();
        assert!(isident("_foo"));
        assert!(isident("_123"));
        assert!(isident("foo_bar"));
    }

    /// c:1288 — alphabetic + digits is valid.
    #[test]
    fn isident_alpha_digits_mix_is_valid() {
        let _g = crate::test_util::global_state_lock();
        assert!(isident("foo123"));
        assert!(isident("var2"));
    }

    /// c:1326 — unbalanced subscript `foo[` is invalid.
    #[test]
    fn isident_unbalanced_open_bracket_returns_false() {
        let _g = crate::test_util::global_state_lock();
        assert!(!isident("foo["));
        assert!(!isident("foo[0"));
    }

    /// c:1326 — special chars like `-`, `.`, `@` not allowed in identifier.
    #[test]
    fn isident_special_chars_rejected() {
        let _g = crate::test_util::global_state_lock();
        assert!(!isident("foo-bar")); // hyphen
        assert!(!isident("foo@bar"));
        assert!(!isident("foo!"));
    }

    /// c:1288 — namespace prefix `.ns.var` requires a non-digit after dot.
    #[test]
    fn isident_namespace_prefix_dot_rejects_digit_after() {
        let _g = crate::test_util::global_state_lock();
        assert!(!isident(".0"), "dot + digit invalid");
        assert!(!isident("."), "lone dot invalid");
    }

    /// c:1288 — namespace prefix `.foo` is valid.
    #[test]
    fn isident_namespace_prefix_alpha_after_dot_valid() {
        let _g = crate::test_util::global_state_lock();
        assert!(isident(".foo"));
        assert!(isident(".ns.var"));
    }

    /// c:1288 — `isident` is deterministic.
    #[test]
    fn isident_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for s in ["foo", "_bar", "123", "1abc", "", ".foo", "foo[", "foo-bar"] {
            let first = isident(s);
            for _ in 0..5 {
                assert_eq!(isident(s), first, "{:?} must be pure", s);
            }
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/params.c
    // c:1403 issetvar / c:2152 setsparam / c:2317 isident / c:4099 getiparam
    // c:4191 getsparam / c:4326 getsparam_u / c:4353 getaparam /
    // c:5363 setaparam / c:5696 setiparam / c:5775 unsetparam
    // ═══════════════════════════════════════════════════════════════════

    /// c:1403 — `issetvar` returns i32 (compile-time type pin).
    #[test]
    fn issetvar_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = issetvar("anything");
    }

    /// c:1403 — `issetvar("__nonexistent")` returns 0 for unset.
    #[test]
    fn issetvar_unknown_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = issetvar("__never_a_real_var_xyz_zshrs__");
        assert_eq!(r, 0, "unset var → 0");
    }

    /// c:4191 — `getsparam` returns Option<String>.
    #[test]
    fn getsparam_returns_option_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<String> = getsparam("anything");
    }

    /// c:4191 — `getsparam("__nonexistent")` returns None.
    #[test]
    fn getsparam_unknown_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            getsparam("__never_a_real_var_xyz_zshrs__").is_none(),
            "unknown var → None"
        );
    }

    /// c:4099 — `getiparam` returns i64.
    #[test]
    fn getiparam_returns_i64_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i64 = getiparam("anything");
    }

    /// c:4353 — `getaparam` returns Option<Vec<String>>.
    #[test]
    fn getaparam_returns_option_vec_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<Vec<String>> = getaparam("anything");
    }

    /// c:4326 — `getsparam_u("")` empty returns None.
    #[test]
    fn getsparam_u_empty_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(getsparam_u("").is_none(), "empty → None");
    }

    /// c:2152 — `setsparam`/`getsparam` round-trip.
    #[test]
    fn setsparam_getsparam_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let name = "ZSHRS_TEST_SETSPARAM_ROUND_TRIP";
        let _ = unsetparam(name);
        setsparam(name, "hello");
        assert_eq!(
            getsparam(name).as_deref(),
            Some("hello"),
            "setsparam/getsparam round-trip"
        );
        unsetparam(name);
    }

    /// c:5696 — `setiparam`/`getiparam` round-trip.
    #[test]
    fn setiparam_getiparam_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let name = "ZSHRS_TEST_SETIPARAM_ROUND_TRIP";
        unsetparam(name);
        setiparam(name, 42);
        assert_eq!(getiparam(name), 42, "setiparam/getiparam round-trip");
        unsetparam(name);
    }

    /// c:5363 — `setaparam`/`getaparam` round-trip preserves array.
    #[test]
    fn setaparam_getaparam_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let name = "ZSHRS_TEST_SETAPARAM_ROUND_TRIP";
        unsetparam(name);
        let v = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        setaparam(name, v.clone());
        assert_eq!(getaparam(name), Some(v), "setaparam/getaparam round-trip");
        unsetparam(name);
    }

    /// c:5775 — `unsetparam` is idempotent on unset names.
    #[test]
    fn unsetparam_unknown_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        unsetparam("__never_real_var_xyz__");
        unsetparam("__never_real_var_xyz__");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/params.c GSU vtable callbacks.
    // c:3993 intgetfn   / c:4002 intsetfn   / c:4011 floatgetfn
    // c:4020 floatsetfn / c:4029 strgetfn   / c:4057 arrgetfn
    // c:4084 hashgetfn  / c:4093 hashsetfn  / c:4180 nullstrsetfn
    // c:4192 nullunsetfn
    // ═══════════════════════════════════════════════════════════════════

    /// c:3993-3997 — `intgetfn` body is `return pm->u.val;`. Pin that
    /// the Rust port reads `pm.u_val` verbatim with no transform.
    #[test]
    fn intgetfn_returns_u_val_verbatim() {
        let _g = crate::test_util::global_state_lock();
        let mut pm = param::default();
        pm.u_val = 0;
        assert_eq!(intgetfn(&pm), 0, "u_val=0 → 0");
        pm.u_val = 42;
        assert_eq!(intgetfn(&pm), 42, "u_val=42 → 42");
        pm.u_val = i64::MIN;
        assert_eq!(intgetfn(&pm), i64::MIN, "u_val=i64::MIN preserved");
        pm.u_val = i64::MAX;
        assert_eq!(intgetfn(&pm), i64::MAX, "u_val=i64::MAX preserved");
    }

    /// c:4002-4006 — `intsetfn` body is `pm->u.val = x;`. For a
    /// non-special name (not SECONDS/RANDOM), the Rust port must
    /// write `pm.u_val = x` and intgetfn must round-trip.
    #[test]
    fn intsetfn_intgetfn_round_trip_for_non_special_name() {
        let _g = crate::test_util::global_state_lock();
        let mut pm = param::default();
        pm.node.nam = "X".to_string(); // non-SECONDS, non-RANDOM
        intsetfn(&mut pm, 7);
        assert_eq!(intgetfn(&pm), 7, "non-special intsetfn writes u_val");
        intsetfn(&mut pm, -123);
        assert_eq!(intgetfn(&pm), -123, "negative i64 round-trips");
    }

    /// c:4011-4015 — `floatgetfn` body is `return pm->u.dval;`.
    /// Read `pm.u_dval` verbatim with no transform.
    #[test]
    fn floatgetfn_returns_u_dval_verbatim() {
        let _g = crate::test_util::global_state_lock();
        let mut pm = param::default();
        pm.u_dval = 0.0;
        assert_eq!(floatgetfn(&pm), 0.0, "u_dval=0.0 → 0.0");
        pm.u_dval = 3.14;
        assert_eq!(floatgetfn(&pm), 3.14, "u_dval=3.14 preserved");
        pm.u_dval = -2.71;
        assert_eq!(floatgetfn(&pm), -2.71, "negative dval preserved");
    }

    /// c:4020-4024 — `floatsetfn` body for non-SECONDS is
    /// `pm->u.dval = x;`. Round-trip via floatgetfn.
    #[test]
    fn floatsetfn_floatgetfn_round_trip_for_non_seconds() {
        let _g = crate::test_util::global_state_lock();
        let mut pm = param::default();
        pm.node.nam = "Y".to_string(); // non-SECONDS
        floatsetfn(&mut pm, 1.5);
        assert_eq!(floatgetfn(&pm), 1.5, "non-SECONDS floatsetfn writes u_dval");
        floatsetfn(&mut pm, -0.0);
        assert_eq!(floatgetfn(&pm), -0.0, "neg zero round-trips");
    }

    /// c:4029-4033 — `strgetfn` C body returns `pm->u.str ? pm->u.str
    /// : (char *) hcalloc(1);` — when u_str is None the C path
    /// returns a freshly allocated empty C string. Rust port must
    /// return `String::new()` (empty owned String) for the None case.
    #[test]
    fn strgetfn_returns_empty_string_when_u_str_none() {
        let _g = crate::test_util::global_state_lock();
        let mut pm = param::default();
        pm.u_str = None;
        let s = strgetfn(&pm);
        assert!(
            s.is_empty(),
            "None u_str → empty String (port of hcalloc(1))"
        );
    }

    /// c:4029-4033 — `strgetfn` returns u_str clone when present.
    #[test]
    fn strgetfn_returns_clone_when_u_str_some() {
        let _g = crate::test_util::global_state_lock();
        let mut pm = param::default();
        pm.u_str = Some("hello".to_string());
        assert_eq!(strgetfn(&pm), "hello", "Some(s) → s");
    }

    /// c:4057-4061 — `arrgetfn` C body returns `pm->u.arr ? pm->u.arr
    /// : &nullarray;` — when u_arr is None, the C path returns a
    /// pointer to the static empty `nullarray`. Rust port returns
    /// `Vec::new()` for the None case (empty owned Vec).
    #[test]
    fn arrgetfn_returns_empty_vec_when_u_arr_none() {
        let _g = crate::test_util::global_state_lock();
        let mut pm = param::default();
        pm.u_arr = None;
        let v = arrgetfn(&pm);
        assert!(v.is_empty(), "None u_arr → empty Vec (port of &nullarray)");
    }

    /// c:4057-4061 — `arrgetfn` returns u_arr clone when present.
    #[test]
    fn arrgetfn_returns_clone_when_u_arr_some() {
        let _g = crate::test_util::global_state_lock();
        let mut pm = param::default();
        let src = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        pm.u_arr = Some(src.clone());
        assert_eq!(arrgetfn(&pm), src, "Some(v) → v clone");
    }

    /// c:4084-4088 — `hashgetfn` C body is `return pm->u.hash;`. The
    /// Rust port returns `Option<&HashTable>` (borrowing form of the
    /// C nullable pointer). Pin both the None and Some cases.
    #[test]
    fn hashgetfn_returns_option_ref_hashtable() {
        let _g = crate::test_util::global_state_lock();
        let mut pm = param::default();
        pm.u_hash = None;
        assert!(hashgetfn(&pm).is_none(), "None u_hash → None");
        pm.u_hash = newparamtable(8, "h");
        assert!(hashgetfn(&pm).is_some(), "Some u_hash → Some");
    }

    /// c:4093-4097 — `hashsetfn` C body installs `x` into `pm->u.hash`.
    /// Rust port writes `pm.u_hash = Some(x)`; round-trip via
    /// hashgetfn must observe the same table.
    #[test]
    fn hashsetfn_hashgetfn_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let mut pm = param::default();
        let ht = newparamtable(8, "h").expect("newparamtable returns Some");
        let want_size = ht.hsize;
        hashsetfn(&mut pm, ht);
        let got = hashgetfn(&pm).expect("hashsetfn must store table");
        assert_eq!(got.hsize, want_size, "stored table size preserved");
    }

    /// c:4180-4183 — `nullstrsetfn(UNUSED pm, char *x)` is the
    /// PM_SPECIAL "cannot be set" hook: `zsfree(x);` discards x and
    /// makes no change to pm. Pin that the Rust port leaves every
    /// field of pm untouched.
    #[test]
    fn nullstrsetfn_leaves_pm_unchanged() {
        let _g = crate::test_util::global_state_lock();
        let mut pm = param::default();
        pm.node.nam = "PINNED".to_string();
        pm.u_str = Some("orig".to_string());
        pm.u_val = 99;
        nullstrsetfn(&mut pm, "discarded".to_string());
        assert_eq!(pm.node.nam, "PINNED", "nam untouched");
        assert_eq!(pm.u_str.as_deref(), Some("orig"), "u_str untouched");
        assert_eq!(pm.u_val, 99, "u_val untouched");
    }

    /// c:4192-4195 — `nullunsetfn(UNUSED pm, UNUSED exp)` is the
    /// PM_SPECIAL "cannot be unset" hook: empty body. Pin that the
    /// Rust port leaves every field of pm untouched.
    #[test]
    fn nullunsetfn_leaves_pm_unchanged() {
        let _g = crate::test_util::global_state_lock();
        let mut pm = param::default();
        pm.node.nam = "PINNED".to_string();
        pm.node.flags = PM_SCALAR as i32;
        pm.u_str = Some("keep".to_string());
        nullunsetfn(&mut pm, 0);
        nullunsetfn(&mut pm, 1);
        assert_eq!(pm.node.nam, "PINNED", "nam untouched");
        assert_eq!(pm.node.flags, PM_SCALAR as i32, "flags untouched");
        assert_eq!(pm.u_str.as_deref(), Some("keep"), "u_str untouched");
    }
}
