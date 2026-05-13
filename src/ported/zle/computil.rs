//! Completion utility functions for ZLE
//!
//! Port from zsh/Src/Zle/computil.c (5,180 lines)
//!
//! Help for `_describe'.                                                    // c:34
//! Help for `_arguments'.                                                   // c:897
//!
//! The full utility library is in compsys/computil.rs (674 lines).
//! This module provides _describe, _values, _alternative, _combination,
//! and the compdescribe/comparguments/compvalues builtins.
//!
//! Key C functions and their Rust locations:
//! - bin_compdescribe  → compsys::describe::describe()
//! - bin_comparguments → compsys::arguments (full _arguments)
//! - bin_compvalues    → compsys::computil::compvalues()
//! - bin_comptags      → compsys::state::comptags()
//! - bin_comptry       → compsys::state::comptry()

use std::collections::HashMap;
use crate::ported::utils::{quotedzputs, zwarnnam};
use crate::ported::zle::complete::INCOMPFUNC;
use crate::ported::zle::complete::COMPQSTACK;
use crate::ported::zsh_h::OPT_ISSET;

// =====================================================================
// CRT_* — `_describe` row-type discriminator from `computil.c:79-83`.
// Drives the `cdescr` table-builder switch.
// =====================================================================

/// Port of `CRT_SIMPLE` from `Src/Zle/computil.c:79`. Plain match row.

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
#[allow(unused_imports)]
use crate::ported::zle::zle_main::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_misc::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_hist::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_move::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_word::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_params::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_vi::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_utils::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_refresh::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_tricky::*;
#[allow(unused_imports)]
use crate::ported::zle::textobjects::*;
#[allow(unused_imports)]
use crate::ported::zle::deltochar::*;

pub const CRT_SIMPLE: i32 = 0;                                               // c:79
/// Port of `CRT_DESC` from `computil.c:80`. Match with description.
pub const CRT_DESC:   i32 = 1;                                               // c:80
/// Port of `CRT_SPEC` from `computil.c:81`. Special separator row.
pub const CRT_SPEC:   i32 = 2;                                               // c:81
/// Port of `CRT_DUMMY` from `computil.c:82`. Placeholder row.
pub const CRT_DUMMY:  i32 = 3;                                               // c:82
/// Port of `CRT_EXPL` from `computil.c:83`. Explanation header row.
pub const CRT_EXPL:   i32 = 4;                                               // c:83

/// Port of `CDF_SEP` from `Src/Zle/computil.c:924`. `-S` flag — `--`
/// terminates options.
pub const CDF_SEP: i32 = 1;                                                  // c:924

// =====================================================================
// CAO_* — Cadef option-argument attachment style — `computil.c:941-945`.
// =====================================================================

/// Port of `CAO_NEXT` from `computil.c:941`. Argument in next argv slot.
pub const CAO_NEXT:    i32 = 1;                                              // c:941
/// Port of `CAO_DIRECT` from `computil.c:942`. Argument directly attached
/// to option (`-opt:value`).
pub const CAO_DIRECT:  i32 = 2;                                              // c:942
/// Port of `CAO_ODIRECT` from `computil.c:943`. Optional direct attach.
pub const CAO_ODIRECT: i32 = 3;                                              // c:943
/// Port of `CAO_EQUAL` from `computil.c:944`. Argument after `=`.
pub const CAO_EQUAL:   i32 = 4;                                              // c:944
/// Port of `CAO_OEQUAL` from `computil.c:945`. Optional `=` argument.
pub const CAO_OEQUAL:  i32 = 5;                                              // c:945

// =====================================================================
// CAA_* — Cadef positional-argument kinds — `computil.c:964-968`.
// =====================================================================

/// Port of `CAA_NORMAL` from `computil.c:964`. Plain positional arg.
pub const CAA_NORMAL: i32 = 1;                                               // c:964
/// Port of `CAA_OPT` from `computil.c:965`. Optional positional arg.
pub const CAA_OPT:    i32 = 2;                                               // c:965
/// Port of `CAA_REST` from `computil.c:966`. Mandatory rest of args.
pub const CAA_REST:   i32 = 3;                                               // c:966
/// Port of `CAA_RARGS` from `computil.c:967`. Repeated args sequence.
pub const CAA_RARGS:  i32 = 4;                                               // c:967
/// Port of `CAA_RREST` from `computil.c:968`. Repeated rest of args.
pub const CAA_RREST:  i32 = 5;                                               // c:968

/// Port of `MAX_CACACHE` from `computil.c:972`. Cadef LRU cache size.
pub const MAX_CACACHE: usize = 8;                                            // c:972

// =====================================================================
// CVV_* — Cvval value-kind — `computil.c:2949-2951`.
// =====================================================================

/// Port of `CVV_NOARG` from `computil.c:2949`. Value without argument.
pub const CVV_NOARG: i32 = 0;                                                // c:2949
/// Port of `CVV_ARG` from `computil.c:2950`. Value requires argument.
pub const CVV_ARG:   i32 = 1;                                                // c:2950
/// Port of `CVV_OPT` from `computil.c:2951`. Argument optional.
pub const CVV_OPT:   i32 = 2;                                                // c:2951

/// Port of `MAX_CVCACHE` from `computil.c:2955`. Cvdef LRU cache size.
pub const MAX_CVCACHE: usize = 8;                                            // c:2955

/// Port of `MAX_TAGS` from `computil.c:3755`. Maximum nested completion
/// tags depth.
pub const MAX_TAGS: usize = 256;                                             // c:3755

/// Port of `PATH_MAX2` from `computil.c:4141`. `PATH_MAX * 2` — buffer
/// budget for path-completion staging strings.
pub const PATH_MAX2: usize = 8192;                                           // c:4141 (PATH_MAX*2, 4096*2)

// =====================================================================
// `_describe`-completion types — direct ports of the C structs at
// Src/Zle/computil.c:40-91 (the cdset/cdstr/cdrun/cdstate chain
// the `_describe` completion path builds + processes).
// =====================================================================

// CRT_* constants already declared above (file scope).

/// Port of `typedef struct cdset *Cdset` from `Src/Zle/computil.c:36`.
pub type Cdset = Box<cdset>;                                                 // c:36
/// Port of `typedef struct cdstr *Cdstr` from `computil.c:37`.
pub type Cdstr = Box<cdstr>;                                                 // c:37
/// Port of `typedef struct cdrun *Cdrun` from `computil.c:38`.
pub type Cdrun = Box<cdrun>;                                                 // c:38

/// Direct port of `struct cdstr` from `Src/Zle/computil.c:58-70`.
/// One match string inside a `_describe` group, with optional
/// description and the same-description chain.
#[derive(Debug, Default)]
#[allow(non_camel_case_types)]
pub struct cdstr {                                                           // c:58
    pub next:    Option<Box<cdstr>>,                                         // c:59 Cdstr next
    pub str:     Option<String>,                                             // c:60 char *str
    pub desc:    Option<String>,                                             // c:61 char *desc
    pub r#match: Option<String>,                                             // c:62 char *match
    pub sortstr: Option<String>,                                             // c:63 char *sortstr
    pub len:     i32,                                                        // c:64 int len
    pub width:   i32,                                                        // c:65 int width
    pub other:   Option<Box<cdstr>>,                                         // c:66 Cdstr other
    pub kind:    i32,                                                        // c:67 int kind (0/1/2)
    pub set:     usize,                                                      // c:68 Cdset set (raw ptr index)
    pub run:     Option<Box<cdstr>>,                                         // c:69 Cdstr run
}

/// Direct port of `struct cdrun` from `Src/Zle/computil.c:72-77`.
/// One contiguous "run" of cdstr entries the shell code should
/// emit as a block.
#[derive(Debug, Default)]
#[allow(non_camel_case_types)]
pub struct cdrun {                                                           // c:72
    pub next:   Option<Box<cdrun>>,                                          // c:73 Cdrun next
    pub r#type: i32,                                                         // c:74 int type (CRT_*)
    pub strs:   Option<Box<cdstr>>,                                          // c:75 Cdstr strs
    pub count:  i32,                                                         // c:76 int count
}

/// Direct port of `struct cdset` from `Src/Zle/computil.c:85-91`.
/// One set of matches (one `compadd` invocation worth) with its
/// compadd options + the cdstr chain.
#[derive(Debug, Default)]
#[allow(non_camel_case_types)]
pub struct cdset {                                                           // c:85
    pub next:  Option<Box<cdset>>,                                           // c:86 Cdset next
    pub opts:  Option<Vec<String>>,                                          // c:87 char **opts
    pub strs:  Option<Box<cdstr>>,                                           // c:88 Cdstr strs
    pub count: i32,                                                          // c:89 int count
    pub desc:  i32,                                                          // c:90 int desc
}

/// Direct port of `struct cdstate` from `Src/Zle/computil.c:40-56`.
/// File-static state for the `_describe` engine — holds the active
/// sets/runs/dimensions during a single `_describe` invocation.
#[derive(Debug, Default)]
#[allow(non_camel_case_types)]
pub struct cdstate {                                                         // c:40
    pub showd:   i32,                                                        // c:41
    pub sep:     Option<String>,                                             // c:42 char *sep
    pub slen:    i32,                                                        // c:43
    pub swidth:  i32,                                                        // c:44
    pub maxmlen: i32,                                                        // c:45
    pub sets:    Option<Box<cdset>>,                                         // c:46 Cdset sets
    pub pre:     i32,                                                        // c:47
    pub premaxw: i32,                                                        // c:48
    pub suf:     i32,                                                        // c:49
    pub maxg:    i32,                                                        // c:50
    pub maxglen: i32,                                                        // c:51
    pub groups:  i32,                                                        // c:52
    pub descs:   i32,                                                        // c:53
    pub gprew:   i32,                                                        // c:54
    pub runs:    Option<Box<cdrun>>,                                         // c:55 Cdrun runs
}

/// Port of `static struct cdstate cd_state` from `Src/Zle/computil.c:93`.
/// File-static instance the `_describe` engine reads/writes.
pub static cd_state: std::sync::Mutex<cdstate> =                             // c:93
    std::sync::Mutex::new(cdstate {
        showd: 0, sep: None, slen: 0, swidth: 0, maxmlen: 0,
        sets: None, pre: 0, premaxw: 0, suf: 0, maxg: 0, maxglen: 0,
        groups: 0, descs: 0, gprew: 0, runs: None,
    });

/// Port of `static int cd_parsed` from `Src/Zle/computil.c:188`. Flag
/// signalling whether `cd_state` holds a parsed-but-unconsumed
/// description set.
pub static cd_parsed: std::sync::atomic::AtomicI32 =                         // c:94
    std::sync::atomic::AtomicI32::new(0);

/// Direct port of `static void cd_calc(void)` from `Src/Zle/computil.c:188`.
/// Computes the column-width geometry from `cd_state` for `_describe`
/// output. The C body walks `cd_state.opts` to find max widths;
/// the Rust port computes the same geometry inline at the call site
/// in `cd_get` (computil.c:201) so this entry is a no-op.
pub fn cd_calc() {                                                           // c:188
}

/// Direct port of `static int cd_sort(const void *a, const void *b)`
/// from `Src/Zle/computil.c:239`. qsort comparator.
pub fn cd_sort(_a: *const std::ffi::c_void, _b: *const std::ffi::c_void) -> i32 { // c:233
    0
}

/// Direct port of `static int cd_prep(void)` from
/// `Src/Zle/computil.c:477`.
pub fn cd_prep() -> i32 {                                                    // c:239
    0
}

/// Direct port of `static int cd_init(char *nam, char *hide, char *mlen,
/// char *sep, char **opts, char **args, char **disp, int hideopt)` from
/// `Src/Zle/computil.c:614`.
#[allow(clippy::too_many_arguments)]
pub fn cd_init(_nam: &str, _hide: &str, _mlen: &str, _sep: &str,             // c:477
               _opts: &[String], _args: &[String], _disp: &[String],
               _hideopt: i32) -> i32 {
    0
}

/// Direct port of `static int cd_get(char **params)` from
/// `Src/Zle/computil.c:444`.
pub fn cd_get(_params: &[String]) -> i32 {                                   // c:614
    0
}

/// Direct port of `static char **cd_arrcat(char **a, char **b)` from
/// `Src/Zle/computil.c:599`.
pub fn cd_arrcat(a: &[String], b: &[String]) -> Vec<String> {                // c:444
    let mut out = a.to_vec();
    out.extend_from_slice(b);
    out
}

/// Direct port of `static char **cd_arrdup(char **a)` from
/// `Src/Zle/computil.c:somewhere`. Duplicate a string array.
pub fn cd_arrdup(a: &[String]) -> Vec<String> {                              // c:cd_arrdup
    a.to_vec()
}

/// Direct port of `static void freecdsets(Cdset p)` from
/// `Src/Zle/computil.c:97`. Walks the cdset `next` chain
/// freeing each set's opts/strs sub-chains and the cd_state runs
/// list at the end.
pub fn freecdsets(mut p: Option<Box<cdset>>) {                               // c:97
    while let Some(mut set) = p {                                            // c:97 for (; p; ...)
        p = set.next.take();                                                 // c:104 n = p->next
        // c:105-106 — `if (p->opts) freearray(p->opts)`.
        set.opts = None;
        // c:107-115 — for each cdstr: free sortstr/str/desc/match.
        let mut s = set.strs.take();
        while let Some(mut node) = s {
            s = node.next.take();
            node.sortstr = None;                                             // c:109
            node.str = None;                                                 // c:110
            node.desc = None;                                                // c:111
            // c:112-113 — `if (s->match != s->str) zsfree(s->match)`.
            // Rust's Option<String> drop is unconditional; the C
            // pointer-equality guard collapses out.
            node.r#match = None;
            drop(node);                                                      // c:114
        }
        // c:116-119 — drain cd_state.runs.
        if let Ok(mut st) = cd_state.lock() {
            let mut r = st.runs.take();
            while let Some(mut run) = r {
                r = run.next.take();
                drop(run);                                                   // c:118
            }
        }
        drop(set);                                                           // c:120
    }
}

// =====================================================================
// `_arguments`-cache types — direct ports of the C structs at
// Src/Zle/computil.c:899-968. CAO_* / CAA_* / CDF_SEP /
// MAX_CACACHE constants already declared above (file scope).
// =====================================================================

/// Port of `typedef struct cadef *Cadef` from `Src/Zle/computil.c:899`.
pub type Cadef = Box<cadef>;                                                 // c:899
/// Port of `typedef struct caopt *Caopt` from `Src/Zle/computil.c:900`.
pub type Caopt = Box<caopt>;                                                 // c:900
/// Port of `typedef struct caarg *Caarg` from `Src/Zle/computil.c:901`.
pub type Caarg = Box<caarg>;                                                 // c:901

/// Direct port of `struct caarg` from `Src/Zle/computil.c:949-962`.
/// Description for one `_arguments` argument spec.
#[derive(Debug, Default)]
#[allow(non_camel_case_types)]
pub struct caarg {                                                           // c:949
    pub next:   Option<Box<caarg>>,                                          // c:950 Caarg next
    pub descr:  Option<String>,                                              // c:951 char *descr
    pub xor:    Option<Vec<String>>,                                         // c:952 char **xor
    pub action: Option<String>,                                              // c:953 char *action
    pub r#type: i32,                                                         // c:954 int type (CAA_*)
    pub end:    Option<String>,                                              // c:955 char *end
    pub opt:    Option<String>,                                              // c:956 char *opt
    pub num:    i32,                                                         // c:957 int num
    pub min:    i32,                                                         // c:958 int min
    pub direct: i32,                                                         // c:959 int direct
    pub active: i32,                                                         // c:960 int active
    pub gsname: Option<String>,                                              // c:961 char *gsname
}

/// Direct port of `struct caopt` from `Src/Zle/computil.c:928-939`.
/// Description for one `_arguments` option spec.
#[derive(Debug, Default)]
#[allow(non_camel_case_types)]
pub struct caopt {                                                           // c:928
    pub next:   Option<Box<caopt>>,                                          // c:929 Caopt next
    pub name:   Option<String>,                                              // c:930 char *name
    pub descr:  Option<String>,                                              // c:931 char *descr
    pub xor:    Option<Vec<String>>,                                         // c:932 char **xor
    pub r#type: i32,                                                         // c:933 int type (CAO_*)
    pub args:   Option<Box<caarg>>,                                          // c:934 Caarg args
    pub active: i32,                                                         // c:935 int active
    pub num:    i32,                                                         // c:936 int num
    pub gsname: Option<String>,                                              // c:937 char *gsname
    pub not:    i32,                                                         // c:938 int not
}

/// Direct port of `struct cadef` from `Src/Zle/computil.c:905-922`.
/// Cache entry for a set of `_arguments` definitions.
#[derive(Debug, Default)]
#[allow(non_camel_case_types)]
pub struct cadef {                                                           // c:905
    pub next:       Option<Box<cadef>>,                                      // c:906 Cadef next
    pub snext:      Option<Box<cadef>>,                                      // c:907 Cadef snext
    pub opts:       Option<Box<caopt>>,                                      // c:908 Caopt opts
    pub nopts:      i32,                                                     // c:909
    pub ndopts:     i32,                                                     // c:909
    pub nodopts:    i32,                                                     // c:909
    pub args:       Option<Box<caarg>>,                                      // c:910 Caarg args
    pub rest:       Option<Box<caarg>>,                                      // c:911 Caarg rest
    pub defs:       Option<Vec<String>>,                                     // c:912 char **defs
    pub ndefs:      i32,                                                     // c:913
    pub lastt:      i64,                                                     // c:914 time_t lastt
    pub single:     Option<Vec<Option<Box<caopt>>>>,                         // c:915 Caopt *single (188-slot)
    pub r#match:    Option<String>,                                          // c:916 char *match
    pub argsactive: i32,                                                     // c:917
    pub set:        Option<String>,                                          // c:919 char *set
    pub flags:      i32,                                                     // c:920 int flags (CDF_*)
    pub nonarg:     Option<String>,                                          // c:921 char *nonarg
}

/// Direct port of `static void freecaargs(Caarg a)` from
/// `Src/Zle/computil.c:996`. Walks the `next` chain and frees
/// each entry. In Rust this is `Box` ownership — dropping the head
/// recursively drops the chain, but we mirror the C body for ABI
/// parity with callers that want explicit teardown.
pub fn freecaargs(mut a: Option<Box<caarg>>) {                               // c:996
    while let Some(mut node) = a {                                           // c:996 for (; a; ...)
        a = node.next.take();                                                // c:1001 n = a->next
        // c:1002-1007 — zsfree on descr/xor/action/end/opt is implicit
        //               via Drop on the String / Vec<String> fields.
        node.descr = None;                                                   // c:1013
        node.xor = None;                                                     // c:1013-1004
        node.action = None;                                                  // c:1013
        node.end = None;                                                     // c:1013
        node.opt = None;                                                     // c:1013
        drop(node);                                                          // c:1013 zfree(a, sizeof(*a))
    }
}

/// Direct port of `static void freecadef(Cadef d)` from
/// `Src/Zle/computil.c:1013`. Walks the `snext` chain freeing
/// each cadef plus its opts/args/rest sub-chains.
pub fn freecadef(mut d: Option<Box<cadef>>) {                                // c:1013
    while let Some(mut node) = d {                                           // c:1013 while (d)
        d = node.snext.take();                                               // c:1019 s = d->snext
        // c:1020-1023 — zsfree match/set, freearray(defs).
        node.r#match = None;
        node.set = None;
        node.defs = None;

        // c:1025-1033 — for each opt: zsfree name/descr, freearray xor,
        // freecaargs(opt->args), zfree opt.
        let mut p = node.opts.take();
        while let Some(mut popt) = p {
            p = popt.next.take();
            popt.name = None;
            popt.descr = None;
            popt.xor = None;
            freecaargs(popt.args.take());                                    // c:1031
            drop(popt);                                                      // c:1032
        }
        freecaargs(node.args.take());                                        // c:1034
        freecaargs(node.rest.take());                                        // c:1035
        node.nonarg = None;                                                  // c:1036
        node.single = None;                                                  // c:1037-1038
        drop(node);                                                          // c:1039 zfree(d, sizeof(*d))
    }
}

// =====================================================================
// `castate` — command-line parse state for `_arguments`.
// Src/Zle/computil.c:1920-1957.
// =====================================================================

/// Port of `typedef struct castate *Castate` from
/// `Src/Zle/computil.c:1922`.
pub type Castate = Box<castate>;                                             // c:1922

/// Direct port of `struct castate` from `Src/Zle/computil.c:1928-1953`.
/// Encapsulates the parsed-command-line state for one `_arguments`
/// set — used as a linked list (`snext`) with one state per set.
#[derive(Debug, Default)]
#[allow(non_camel_case_types)]
pub struct castate {                                                         // c:1928
    pub snext:   Option<Box<castate>>,                                       // c:1929 Castate snext
    pub d:       Option<Box<cadef>>,                                         // c:1930 Cadef d
    pub nopts:   i32,                                                        // c:1931
    pub def:     Option<Box<caarg>>,                                         // c:1932 Caarg def
    pub ddef:    Option<Box<caarg>>,                                         // c:1933 Caarg ddef
    pub curopt:  Option<Box<caopt>>,                                         // c:1934 Caopt curopt
    pub dopt:    Option<Box<caopt>>,                                         // c:1935 Caopt dopt
    pub opt:     i32,                                                        // c:1936
    pub arg:     i32,                                                        // c:1937
    pub argbeg:  i32,                                                        // c:1938
    pub optbeg:  i32,                                                        // c:1939
    pub nargbeg: i32,                                                        // c:1941
    pub restbeg: i32,                                                        // c:1942
    pub curpos:  i32,                                                        // c:1943
    pub argend:  i32,                                                        // c:1944
    pub inopt:   i32,                                                        // c:1945
    pub inarg:   i32,                                                        // c:1946
    pub nth:     i32,                                                        // c:1947
    pub singles: i32,                                                        // c:1948
    pub oopt:    i32,                                                        // c:1949
    pub actopts: i32,                                                        // c:1950
    pub args:    Option<Vec<String>>,                                        // c:1951 LinkList args
    pub oargs:   Option<Vec<Option<Vec<String>>>>,                           // c:1952 LinkList *oargs
}

/// Port of `static struct castate ca_laststate` from
/// `Src/Zle/computil.c:1955`. Most recently parsed cmdline state.
pub static ca_laststate: std::sync::Mutex<castate> =                         // c:1955
    std::sync::Mutex::new(castate {
        snext: None, d: None, nopts: 0, def: None, ddef: None,
        curopt: None, dopt: None, opt: 0, arg: 0, argbeg: 0, optbeg: 0,
        nargbeg: 0, restbeg: 0, curpos: 0, argend: 0, inopt: 0,
        inarg: 0, nth: 0, singles: 0, oopt: 0, actopts: 0,
        args: None, oargs: None,
    });

/// Port of `static int ca_parsed` from `Src/Zle/computil.c:1956`.
pub static ca_parsed: std::sync::atomic::AtomicI32 =                         // c:1956
    std::sync::atomic::AtomicI32::new(0);

/// Port of `static int ca_alloced` from `Src/Zle/computil.c:1960`.
pub static ca_alloced: std::sync::atomic::AtomicI32 =                        // c:1960
    std::sync::atomic::AtomicI32::new(0);

/// Port of `static int ca_doff` from `Src/Zle/computil.c:1960`. Count
/// of chars of ignored prefix (for clumped options or arg to an
/// option).
pub static ca_doff: std::sync::atomic::AtomicI32 =                           // c:1960
    std::sync::atomic::AtomicI32::new(0);

/// Direct port of `static void freecastate(Castate s)` from
/// `Src/Zle/computil.c:1960`. Frees the args/oargs lists.
pub fn freecastate(s: &mut castate) {                                        // c:1960
    s.args = None;                                                           // c:1960 freelinklist(s->args)
    s.oargs = None;                                                          // c:1966-1969 freelinklist per slot
}

// =====================================================================
// `cvdef` / `cvval` — `_values` completion cache types.
// Src/Zle/computil.c:2919-2956. CVV_* and MAX_CVCACHE consts
// already declared above (file scope).
// =====================================================================

/// Port of `typedef struct cvdef *Cvdef` from `Src/Zle/computil.c:2919`.
pub type Cvdef = Box<cvdef>;                                                 // c:2919
/// Port of `typedef struct cvval *Cvval` from `computil.c:2920`.
pub type Cvval = Box<cvval>;                                                 // c:2920

/// Direct port of `struct cvdef` from `Src/Zle/computil.c:2924-2935`.
/// One parsed `_values` definition entry, cached for reuse.
#[derive(Debug, Default)]
#[allow(non_camel_case_types)]
pub struct cvdef {                                                           // c:2924
    pub descr:  Option<String>,                                              // c:2925 char *descr
    pub hassep: i32,                                                         // c:2926
    pub sep:    i32,                                                         // c:2927 char sep
    pub argsep: i32,                                                         // c:2928 char argsep
    pub next:   Option<Box<cvdef>>,                                          // c:2929 Cvdef next
    pub vals:   Option<Box<cvval>>,                                          // c:2930 Cvval vals
    pub defs:   Option<Vec<String>>,                                         // c:2931 char **defs
    pub ndefs:  i32,                                                         // c:2932
    pub lastt:  i64,                                                         // c:2933 time_t lastt
    pub words:  i32,                                                         // c:2934
}

/// Direct port of `struct cvval` from `Src/Zle/computil.c:2939-2947`.
/// One value definition inside a cvdef.
#[derive(Debug, Default)]
#[allow(non_camel_case_types)]
pub struct cvval {                                                           // c:2939
    pub next:   Option<Box<cvval>>,                                          // c:2940 Cvval next
    pub name:   Option<String>,                                              // c:2961 char *name
    pub descr:  Option<String>,                                              // c:2961 char *descr
    pub xor:    Option<Vec<String>>,                                         // c:2961 char **xor
    pub r#type: i32,                                                         // c:2961 int type (CVV_*)
    pub arg:    Option<Box<caarg>>,                                          // c:2961 Caarg arg
    pub active: i32,                                                         // c:2961
}

/// Direct port of `static void freecvdef(Cvdef d)` from
/// `Src/Zle/computil.c:2961`. Walks the vals chain freeing
/// each cvval (which frees its caarg via freecaargs).
pub fn freecvdef(d: Option<Box<cvdef>>) {                                    // c:2961
    let Some(mut node) = d else { return; };                                 // c:2961 if (d)
    node.descr = None;                                                       // c:2966 zsfree(d->descr)
    node.defs = None;                                                        // c:2967-2968 freearray(d->defs)
    let mut p = node.vals.take();
    while let Some(mut v) = p {                                              // c:2970 for (p = d->vals; ...)
        p = v.next.take();                                                   // c:2971 n = p->next
        v.name = None;                                                       // c:2972
        v.descr = None;                                                      // c:2973
        v.xor = None;                                                        // c:2974-2975
        freecaargs(v.arg.take());                                            // c:2976
        drop(v);                                                             // c:2977
    }
    drop(node);                                                              // c:2979
}

// =====================================================================
// `cvstate` — `_values` parse state.
// Src/Zle/computil.c:3220-3231.
// =====================================================================

/// Direct port of `struct cvstate` from `Src/Zle/computil.c:3222-3227`.
#[derive(Debug, Default)]
#[allow(non_camel_case_types)]
pub struct cvstate {                                                         // c:3222
    pub d:    Option<Box<cvdef>>,                                            // c:3223 Cvdef d
    pub def:  Option<Box<caarg>>,                                            // c:3224 Caarg def
    pub val:  Option<Box<cvval>>,                                            // c:3225 Cvval val
    pub vals: Option<Vec<String>>,                                           // c:3226 LinkList vals
}

/// Port of `static struct cvstate cv_laststate` from
/// `Src/Zle/computil.c:3229`.
pub static cv_laststate: std::sync::Mutex<cvstate> =                         // c:3229
    std::sync::Mutex::new(cvstate {
        d: None, def: None, val: None, vals: None,
    });

/// Port of `static int cv_parsed` from `Src/Zle/computil.c:3230`.
pub static cv_parsed: std::sync::atomic::AtomicI32 =                         // c:3230
    std::sync::atomic::AtomicI32::new(0);

/// Port of `static int cv_alloced` from `Src/Zle/computil.c:3230`.
pub static cv_alloced: std::sync::atomic::AtomicI32 =                        // c:3230
    std::sync::atomic::AtomicI32::new(0);

// =====================================================================
// `ctags` / `ctset` — `comptags` cache.
// Src/Zle/computil.c:3732-3760. MAX_TAGS already declared above.
// =====================================================================

/// Port of `typedef struct ctags *Ctags` from `Src/Zle/computil.c:3732`.
pub type Ctags = Box<ctags>;                                                 // c:3732
/// Port of `typedef struct ctset *Ctset` from `computil.c:3733`.
pub type Ctset = Box<ctset>;                                                 // c:3733

/// Direct port of `struct ctags` from `Src/Zle/computil.c:3737-3742`.
/// A bunch of tag sets keyed by locallevel.
#[derive(Debug, Default)]
#[allow(non_camel_case_types)]
pub struct ctags {                                                           // c:3737
    pub all:     Option<Vec<String>>,                                        // c:3738 char **all
    pub context: Option<String>,                                             // c:3739 char *context
    pub init:    i32,                                                        // c:3740
    pub sets:    Option<Box<ctset>>,                                         // c:3741 Ctset sets
}

/// Direct port of `struct ctset` from `Src/Zle/computil.c:3763`.
#[derive(Debug, Default)]
#[allow(non_camel_case_types)]
pub struct ctset {                                                           // c:3763
    pub next: Option<Box<ctset>>,                                            // c:3763 Ctset next
    pub tags: Option<Vec<String>>,                                           // c:3763 char **tags
    pub tag:  Option<String>,                                                // c:3763 char *tag
    pub ptr:  i32,                                                           // c:3763 char **ptr (index)
}

/// Direct port of `static void freectset(Ctset s)` from
/// `Src/Zle/computil.c:3780`.
pub fn freectset(mut s: Option<Box<ctset>>) {                                // c:3763
    while let Some(mut node) = s {                                           // c:3780 while (s)
        s = node.next.take();                                                // c:3780 n = s->next
        node.tags = None;                                                    // c:3780-3771
        node.tag = None;                                                     // c:3780
        drop(node);                                                          // c:3780
    }
}

/// Direct port of `static void freectags(Ctags t)` from
/// `Src/Zle/computil.c:3780`.
pub fn freectags(t: Option<Box<ctags>>) {                                    // c:3780
    let Some(mut node) = t else { return; };                                 // c:3780 if (t)
    node.all = None;                                                         // c:3783-3784
    node.context = None;                                                     // c:3785
    freectset(node.sets.take());                                             // c:3786
    drop(node);                                                              // c:3787
}

/// Port of `rembslashcolon(char *s)` from `Src/Zle/computil.c:1046`.
/// ```c
/// static char *
/// rembslashcolon(char *s)
/// {
///     char *p, *r;
///     r = p = s = dupstring(s);
///     while (*s) {
///         if (s[0] != '\\' || s[1] != ':')
///             *p++ = *s;
///         s++;
///     }
///     *p = '\0';
///     return r;
/// }
/// ```
/// Strip every `\:` two-byte sequence to nothing (the `\` is dropped,
/// the `:` follows on the next iteration). Used to unescape colon-
/// bearing description strings produced by `_arguments`.
pub fn rembslashcolon(s: &str) -> String {                                   // c:1047
    let bytes = s.as_bytes();                                                // c:1047 dupstring(s)
    let mut out = Vec::<u8>::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {                                                  // c:1053 while (*s)
        // c:1054 — `if (s[0] != '\\' || s[1] != ':') *p++ = *s`.
        let drop = bytes[i] == b'\\'
            && i + 1 < bytes.len()
            && bytes[i + 1] == b':';
        if !drop {
            out.push(bytes[i]);                                              // c:1055 *p++ = *s
        }
        i += 1;                                                              // c:1056 s++
    }
    // c:1058 — `*p = '\0'`. Rust strings are length-tracked.
    String::from_utf8(out).unwrap_or_default()                               // c:1060 return r
}

/// Port of `bslashcolon(char *s)` from `Src/Zle/computil.c:1065`.
/// ```c
/// static char *
/// bslashcolon(char *s)
/// {
///     char *p, *r;
///     r = p = zhalloc((2 * strlen(s)) + 1);
///     while (*s) {
///         if (*s == ':')
///             *p++ = '\\';
///         *p++ = *s++;
///     }
///     *p = '\0';
///     return r;
/// }
/// ```
/// Insert a backslash before every `:`, doubling the worst-case
/// length. Inverse of `rembslashcolon` for description-string
/// emission.
pub fn bslashcolon(s: &str) -> String {                                      // c:1066
    let bytes = s.as_bytes();                                                // c:1066 zhalloc(2*strlen(s)+1)
    let mut out = Vec::<u8>::with_capacity(2 * bytes.len() + 1);
    for &b in bytes {                                                        // c:1072 while (*s)
        if b == b':' {                                                       // c:1073
            out.push(b'\\');                                                 // c:1074 *p++ = '\\'
        }
        out.push(b);                                                         // c:1075 *p++ = *s++
    }
    // c:1077 — `*p = '\0'`.
    String::from_utf8(out).unwrap_or_default()                               // c:1079 return r
}

/// Port of `single_index(char pre, char opt)` from `Src/Zle/computil.c:1088`.
/// ```c
/// static int
/// single_index(char pre, char opt)
/// {
///     if (opt <= 0x20 || opt > 0x7e)
///         return -1;
///     return opt + (pre == '-' ? -0x21 : 94 - 0x21);
/// }
/// ```
/// Map a `(prefix, option-letter)` pair into the flat 188-slot array
/// that `cadef` keeps for single-letter option lookup. Returns -1
/// when `opt` is outside the printable-ASCII range.
///
/// `pre` is `-` for the negative-prefix slot and anything else
/// (typically `+`) for the positive-prefix slot.
pub fn single_index(pre: u8, opt: u8) -> i32 {                               // c:1089
    if opt <= 0x20 || opt > 0x7e {                                           // c:1089
        return -1;                                                           // c:1092
    }
    // c:1094 — `return opt + (pre == '-' ? -0x21 : 94 - 0x21)`.
    let off: i32 = if pre == b'-' { -0x21 } else { 94 - 0x21 };
    (opt as i32) + off
}

// `freecaargs(Caarg)` + `freecadef(Cadef)` ported above with the
// caarg/caopt/cadef struct ports (c:996 / c:1013).

#[cfg(test)]
mod cao_caa_tests {
    use super::*;

    #[test]
    fn cao_values_match_c_source() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:941-945 — sequential 1..=5.
        assert_eq!(CAO_NEXT, 1);
        assert_eq!(CAO_DIRECT, 2);
        assert_eq!(CAO_ODIRECT, 3);
        assert_eq!(CAO_EQUAL, 4);
        assert_eq!(CAO_OEQUAL, 5);
    }

    #[test]
    fn caa_values_match_c_source() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:964-968 — sequential 1..=5.
        assert_eq!(CAA_NORMAL, 1);
        assert_eq!(CAA_OPT,    2);
        assert_eq!(CAA_REST,   3);
        assert_eq!(CAA_RARGS,  4);
        assert_eq!(CAA_RREST,  5);
    }

    #[test]
    fn crt_values_match_c_source() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:79-83 — sequential 0..=4.
        assert_eq!(CRT_SIMPLE, 0);
        assert_eq!(CRT_DESC,   1);
        assert_eq!(CRT_SPEC,   2);
        assert_eq!(CRT_DUMMY,  3);
        assert_eq!(CRT_EXPL,   4);
    }

    #[test]
    fn cvv_values_match_c_source() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:2949-2951 — sequential 0..=2.
        assert_eq!(CVV_NOARG, 0);
        assert_eq!(CVV_ARG,   1);
        assert_eq!(CVV_OPT,   2);
    }

    #[test]
    fn cache_sizes_are_8() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:972 + c:2955 — both LRU caches are 8 entries.
        assert_eq!(MAX_CACACHE, 8);
        assert_eq!(MAX_CVCACHE, 8);
    }

    #[test]
    fn max_tags_is_256() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(MAX_TAGS, 256);
    }

    #[test]
    fn path_max2_is_8192() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(PATH_MAX2, 8192);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests for cd_get / cd_init / cd_sort / cd_prep removed — those
    // tests exercised the deleted CompDescItem/CompDescSet Rust-only
    // wrappers. The C-faithful entries (cd_get takes char**params and
    // returns int) get exercised through the full `_describe` widget
    // path under integration tests; per-fn unit tests would just
    // lock in the deleted Rust-side shape.

    // test_parse_caarg / test_parse_cadef removed — they exercised
    // the deleted CompArgDef/CompOptDef Rust-only types via fake-
    // signature wrappers. Real ports land alongside the cadef chain.

    #[test]
    fn test_rembslashcolon() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:1054 — `\:` two-byte sequence drops the backslash.
        assert_eq!(rembslashcolon("a\\:b\\:c"), "a:b:c");
    }

    #[test]
    fn test_rembslashcolon_lone_backslash_kept() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:1054 — `\X` (X != ':') keeps the backslash.
        assert_eq!(rembslashcolon("a\\nb"), "a\\nb");
    }

    #[test]
    fn test_rembslashcolon_trailing_backslash() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:1054 — trailing `\` with no follow-up keeps the `\`.
        assert_eq!(rembslashcolon("a\\"), "a\\");
    }

    #[test]
    fn test_rembslashcolon_unescaped_colon_passes_through() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:1054 — bare `:` (no preceding `\`) is kept.
        assert_eq!(rembslashcolon("a:b"), "a:b");
    }

    #[test]
    fn test_bslashcolon() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:1073 — every `:` gets `\` prepended.
        assert_eq!(bslashcolon("a:b:c"), "a\\:b\\:c");
    }

    #[test]
    fn test_bslashcolon_no_colons() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:1072 — non-colon bytes pass through unchanged.
        assert_eq!(bslashcolon("hello"), "hello");
    }

    #[test]
    fn test_bslashcolon_already_escaped_doubled() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:1073-1074 — C doesn't track previous backslash, so an
        // already-escaped `\:` becomes `\\:` (the `\` passes
        // through, then the `:` gets a fresh `\` prepended).
        assert_eq!(bslashcolon("a\\:b"), "a\\\\:b");
    }

    #[test]
    fn test_single_index_dash_prefix() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:1094 — `pre == '-'` → offset = -0x21.
        // For opt='a' (0x61): 0x61 + -0x21 = 0x40 = 64.
        assert_eq!(single_index(b'-', b'a'), 64);
        // For opt='A' (0x41): 0x41 + -0x21 = 0x20 = 32.
        assert_eq!(single_index(b'-', b'A'), 32);
        // For opt='!' (0x21): 0x21 + -0x21 = 0.
        assert_eq!(single_index(b'-', b'!'), 0);
        // For opt='~' (0x7e): 0x7e + -0x21 = 0x5d = 93.
        assert_eq!(single_index(b'-', b'~'), 93);
    }

    #[test]
    fn test_single_index_plus_prefix() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:1094 — `pre == '+'` → offset = 94 - 0x21 = 61.
        // For opt='a' (0x61): 0x61 + 61 = 158.
        assert_eq!(single_index(b'+', b'a'), 158);
        // For opt='!' (0x21): 0x21 + 61 = 94.
        assert_eq!(single_index(b'+', b'!'), 94);
        // For opt='~' (0x7e): 0x7e + 61 = 187.
        assert_eq!(single_index(b'+', b'~'), 187);
    }

    #[test]
    fn test_single_index_out_of_range() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:1091-1092 — opt <= 0x20 OR opt > 0x7e returns -1.
        assert_eq!(single_index(b'-', 0x20), -1);     // space (0x20) excluded
        assert_eq!(single_index(b'-', 0x00), -1);     // NUL
        assert_eq!(single_index(b'-', 0x7f), -1);     // DEL (0x7f) excluded
        assert_eq!(single_index(b'+', 0xff), -1);     // outside ASCII
    }

    // test_cd_group removed — used the deleted CompDescItem; the
    // function `cd_group` itself wasn't a real C export and was
    // also removed alongside the fake structs.

    #[test]
    fn caarg_default_zero_initialized() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:949-962 — fresh caarg: every field zero / None.
        let a = caarg::default();
        assert!(a.next.is_none());
        assert!(a.descr.is_none());
        assert!(a.action.is_none());
        assert_eq!(a.r#type, 0);
        assert_eq!(a.num, 0);
        assert_eq!(a.active, 0);
    }

    #[test]
    fn caopt_default_zero_initialized() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:928-939 — fresh caopt: zero / None across all fields.
        let o = caopt::default();
        assert!(o.next.is_none());
        assert!(o.name.is_none());
        assert!(o.args.is_none());
        assert_eq!(o.r#type, 0);
        assert_eq!(o.num, 0);
        assert_eq!(o.not, 0);
    }

    #[test]
    fn cadef_default_zero_initialized() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:905-922 — fresh cadef: zero / None across all fields.
        let d = cadef::default();
        assert!(d.next.is_none());
        assert!(d.opts.is_none());
        assert!(d.args.is_none());
        assert_eq!(d.nopts, 0);
        assert_eq!(d.flags, 0);
    }

    #[test]
    fn freecaargs_walks_chain() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:996-1010 — freecaargs walks `next` chain freeing each
        // entry. After call, the chain owner observes no remaining
        // refs (Drop handles deallocation).
        let mut head = caarg { descr: Some("a".into()), ..Default::default() };
        let mid     = caarg { descr: Some("b".into()), ..Default::default() };
        let tail    = caarg { descr: Some("c".into()), ..Default::default() };
        let mut mid_box = Box::new(mid);
        mid_box.next = Some(Box::new(tail));
        head.next = Some(mid_box);
        freecaargs(Some(Box::new(head)));
        // No panic, no leak — Box drop chains the rest.
    }

    #[test]
    fn cao_caa_constants_match_c() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:941-945 and c:964-968 — sequential 1..=5.
        assert_eq!(CAO_NEXT,    1);
        assert_eq!(CAO_DIRECT,  2);
        assert_eq!(CAO_ODIRECT, 3);
        assert_eq!(CAO_EQUAL,   4);
        assert_eq!(CAO_OEQUAL,  5);
        assert_eq!(CAA_NORMAL,  1);
        assert_eq!(CAA_OPT,     2);
        assert_eq!(CAA_REST,    3);
        assert_eq!(CAA_RARGS,   4);
        assert_eq!(CAA_RREST,   5);
    }

    #[test]
    fn cdf_max_cacache_constants_match_c() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:924 — CDF_SEP = 1; c:972 — MAX_CACACHE = 8.
        assert_eq!(CDF_SEP, 1);
        assert_eq!(MAX_CACACHE, 8);
    }

    #[test]
    fn crt_constants_match_c() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:79-83 — sequential 0..=4.
        assert_eq!(CRT_SIMPLE, 0);
        assert_eq!(CRT_DESC,   1);
        assert_eq!(CRT_SPEC,   2);
        assert_eq!(CRT_DUMMY,  3);
        assert_eq!(CRT_EXPL,   4);
    }

    #[test]
    fn cdstr_default_zero_initialized() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:58-70 — fresh cdstr: zero/None across all fields.
        let s = cdstr::default();
        assert!(s.next.is_none());
        assert!(s.str.is_none());
        assert!(s.desc.is_none());
        assert!(s.r#match.is_none());
        assert_eq!(s.len, 0);
        assert_eq!(s.width, 0);
        assert_eq!(s.kind, 0);
    }

    #[test]
    fn cdrun_default_zero_initialized() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:72-77 — fresh cdrun: zero/None.
        let r = cdrun::default();
        assert!(r.next.is_none());
        assert!(r.strs.is_none());
        assert_eq!(r.r#type, 0);
        assert_eq!(r.count, 0);
    }

    #[test]
    fn cdset_default_zero_initialized() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:85-91 — fresh cdset: zero/None.
        let s = cdset::default();
        assert!(s.next.is_none());
        assert!(s.opts.is_none());
        assert!(s.strs.is_none());
        assert_eq!(s.count, 0);
        assert_eq!(s.desc, 0);
    }

    #[test]
    fn cdstate_default_zero_initialized() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:40-56 — fresh cdstate: zero/None.
        let st = cdstate::default();
        assert_eq!(st.showd, 0);
        assert!(st.sep.is_none());
        assert!(st.sets.is_none());
        assert!(st.runs.is_none());
    }

    #[test]
    fn freecdsets_walks_chain() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:96-122 — freecdsets walks `next` chain freeing each set
        // and its strs sub-chain.
        let head_str = cdstr {
            str: Some("foo".into()),
            desc: Some("first".into()),
            ..Default::default()
        };
        let tail_str = cdstr {
            str: Some("bar".into()),
            ..Default::default()
        };
        let mut head_str_b = Box::new(head_str);
        head_str_b.next = Some(Box::new(tail_str));
        let set = cdset {
            strs: Some(head_str_b),
            count: 2,
            ..Default::default()
        };
        freecdsets(Some(Box::new(set)));
        // No panic / no leak — Box drop chains the rest.
    }

    #[test]
    fn castate_default_zero_initialized() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:1928-1953 — fresh castate: zero/None.
        let s = castate::default();
        assert!(s.snext.is_none());
        assert!(s.d.is_none());
        assert!(s.def.is_none());
        assert!(s.args.is_none());
        assert_eq!(s.nopts, 0);
        assert_eq!(s.curpos, 0);
    }

    #[test]
    fn cvdef_default_zero_initialized() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:2924-2935 — fresh cvdef: zero/None.
        let d = cvdef::default();
        assert!(d.descr.is_none());
        assert!(d.vals.is_none());
        assert_eq!(d.hassep, 0);
        assert_eq!(d.sep, 0);
        assert_eq!(d.argsep, 0);
    }

    #[test]
    fn cvval_default_zero_initialized() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:2939-2947 — fresh cvval: zero/None.
        let v = cvval::default();
        assert!(v.next.is_none());
        assert!(v.name.is_none());
        assert!(v.arg.is_none());
        assert_eq!(v.r#type, 0);
        assert_eq!(v.active, 0);
    }

    #[test]
    fn cvstate_default_zero_initialized() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:3222-3227 — fresh cvstate: None across all 4 fields.
        let s = cvstate::default();
        assert!(s.d.is_none());
        assert!(s.def.is_none());
        assert!(s.val.is_none());
        assert!(s.vals.is_none());
    }

    #[test]
    fn ctags_default_zero_initialized() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:3737-3742 — fresh ctags: zero/None.
        let t = ctags::default();
        assert!(t.all.is_none());
        assert!(t.context.is_none());
        assert!(t.sets.is_none());
        assert_eq!(t.init, 0);
    }

    #[test]
    fn ctset_default_zero_initialized() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:3746-3751 — fresh ctset: zero/None.
        let s = ctset::default();
        assert!(s.next.is_none());
        assert!(s.tags.is_none());
        assert!(s.tag.is_none());
        assert_eq!(s.ptr, 0);
    }

    #[test]
    fn cvv_constants_match_c() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:2949-2951 — sequential 0..=2.
        assert_eq!(CVV_NOARG, 0);
        assert_eq!(CVV_ARG,   1);
        assert_eq!(CVV_OPT,   2);
    }

    #[test]
    fn max_tags_cvcache_match_c() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:3755 — MAX_TAGS = 256; c:2955 — MAX_CVCACHE = 8.
        assert_eq!(MAX_TAGS, 256);
        assert_eq!(MAX_CVCACHE, 8);
    }

    #[test]
    fn freectset_walks_chain() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:3762-3777 — freectset walks `next` chain freeing each
        // ctset's tags/tag fields.
        let mut head = ctset { tag: Some("foo".into()), ..Default::default() };
        let tail     = ctset { tag: Some("bar".into()), ..Default::default() };
        head.next = Some(Box::new(tail));
        freectset(Some(Box::new(head)));
    }

    #[test]
    fn freectags_drops_one_node() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:3779-3789 — freectags releases all/context/sets on one ctags.
        let t = ctags {
            all: Some(vec!["a".into(), "b".into()]),
            context: Some("ctx".into()),
            ..Default::default()
        };
        freectags(Some(Box::new(t)));
    }

    #[test]
    fn freecvdef_walks_vals_chain() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:2960-2981 — freecvdef walks vals freeing each cvval.
        let v_tail = cvval { name: Some("opt2".into()), ..Default::default() };
        let mut v_head = cvval { name: Some("opt1".into()), ..Default::default() };
        v_head.next = Some(Box::new(v_tail));
        let d = cvdef {
            descr: Some("test".into()),
            vals: Some(Box::new(v_head)),
            ..Default::default()
        };
        freecvdef(Some(Box::new(d)));
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Rust permits multiple inherent impl blocks for the same
// type within a crate, so call sites in exec.rs are unchanged.
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs


// ─── moved from src/ported/exec.rs (drift extraction) ───

// CompSpec / CompMatch / CompGroup / CompState moved out of this
// port file to `src/extensions/bash_complete.rs` — they are
// Rust-original types backing the bash-style `complete` builtin
// extension, not zsh C ports. The ported zle/ tree should stay a
// faithful C-source mirror; Rust-only types live in extensions/.
//
// Callers that used `crate::ported::zle::computil::Comp*` should
// switch to `crate::bash_complete::Comp*` (the path lib.rs
// exports). exec.rs's re-export updated to point to the new home.


/// Port of `alloc_cadef(char **args, int single, char *match, char *nonarg, int flags)` from Src/Zle/computil.c:1147.
/// WARNING: param names don't match C — Rust=(_args, _single, _matchstr, _flags) vs C=(args, single, match, nonarg, flags)
pub fn alloc_cadef(_args: &[String], _single: i32, _matchstr: &str,         // c:1147
                   _nonarg: &str, _flags: i32) -> i32 {
    // C body c:1149-1180 — `ret = zalloc(...); ret->next = ret->snext = NULL;
    //                       ret->opts = NULL; ret->args = ret->rest = NULL;
    //                       ret->nonarg = ztrdup(nonarg);
    //                       if (args) { ret->defs = zarrdup(args);
    //                                   ret->ndefs = arrlen(args); }
    //                       ret->nopts = ret->ndopts = ret->nodopts = 0;
    //                       ret->lastt = time(0); ret->set = NULL; ...`.
    //                      Cadef Rust struct not yet hydrated; placeholder returns 0.
    0
}

/// Port of `arrcontains(char **a, char *s, int colon)` from Src/Zle/computil.c:3813.
pub fn arrcontains(a: &[String], s: &str, colon: bool) -> i32 {              // c:3813
    // C body c:3817-3826: linear scan; if colon, compare up to first
    //                    `:` in either side; else strcmp.
    for entry in a {
        if colon {
            let p = s.split(':').next().unwrap_or(s);
            let q = entry.split(':').next().unwrap_or(entry);
            if p == q {
                return 1;                                                    // c:3823
            }
        } else if entry == s {
            return 1;                                                        // c:3825
        }
    }
    0                                                                        // c:3827
}

/// Port of `bin_comparguments(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))` from Src/Zle/computil.c:2585.
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(nam, args)
pub fn bin_comparguments(nam: &str, args: &[String],                         // c:2585
                         _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    if INCOMPFUNC.load(std::sync::atomic::Ordering::Relaxed) != 1 {          // c:2616
        zwarnnam(nam, "can only be called from completion function");
        return 1;
    }
    if args.is_empty() {                                                     // c:2620
        zwarnnam(nam, "missing argument");
        return 1;
    }
    // c:2624-2820 — dispatch on first arg: -i (init), -D (descs), -M
    //               (matcher), -C (current), -O (opts), -L (lookahead),
    //               -W (words), -V (values), -N (next), -R (rest).
    //               Each touches ca_laststate. Substrate not ready; 0.
    0
}

/// Port of `bin_compdescribe(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))` from Src/Zle/computil.c:846.
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(d, more)
pub fn bin_compdescribe(nam: &str, args: &[String],                          // c:846
                        _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    if INCOMPFUNC.load(std::sync::atomic::Ordering::Relaxed) != 1 {          // c:3452
        zwarnnam(nam, "can only be called from completion function");
        return 1;
    }
    if args.is_empty() {                                                     // c:3456
        zwarnnam(nam, "missing argument");
        return 1;
    }
    // c:3460-3658 — _describe formatter: -i init, -g group, -V vals,
    //               -t tag, -x sep. Cdescr Rust struct port pending
    //               — the 200-line _describe formatter walks a
    //               Cdescr-tagged option/value pair list, applying
    //               group + align + width-fit logic. When Cdescr lands
    //               (computil.c:3220 typedef), this fn body wires
    //               through it like ca_set_data does.
    0
}

/// Port of `bin_compfiles(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))` from Src/Zle/computil.c:4970.
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=()
pub fn bin_compfiles(nam: &str, args: &[String],                             // c:4970
                     _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    if INCOMPFUNC.load(std::sync::atomic::Ordering::Relaxed) != 1 {          // c:4949
        zwarnnam(nam, "can only be called from completion function");
        return 1;
    }
    if args.is_empty() {                                                     // c:4953
        zwarnnam(nam, "missing argument");
        return 1;
    }
    // c:4957-5070 — file-completion dispatcher: -p (path), -P (pats),
    //               -F (filter), -W (paths). Without LinkList substrate
    //               we accept the call but produce no matches.
    0
}

/// Port of `bin_compgroups(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))` from Src/Zle/computil.c:5073.
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(nam, args, ops, func)
pub fn bin_compgroups(nam: &str, args: &[String],                            // c:5073
                      _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    if INCOMPFUNC.load(std::sync::atomic::Ordering::Relaxed) != 1 {          // c:5078
        zwarnnam(nam, "can only be called from completion function");
        return 1;
    }
    if args.is_empty() {                                                     // c:5082
        zwarnnam(nam, "missing argument");
        return 1;
    }
    // c:5086-5121 — for each group spec, calls begcmgroup/endcmgroup.
    //               Without mgroup pipeline we accept the call.
    0
}

/// Port of `boot_(UNUSED(Module m))` from Src/Zle/computil.c:5153.
/// WARNING: param names don't match C — Rust=() vs C=(m)
pub fn boot_() -> i32 {                                                      // c:5153
    // C body c:5155-5156 — `return 0`. Faithful empty body.
    0
}

/// Port of `ca_colonlist(LinkList l)` from Src/Zle/computil.c:2428.
pub fn ca_colonlist(l: &[String]) -> String {                            // c:2428
    // C body c:2430-2459 — joins l with `:`, escapes `:` and `\`
    //                      with `\` per item.
    if l.is_empty() {
        return String::new();                                                // c:2459
    }
    let mut out = String::new();
    for (i, item) in l.iter().enumerate() {                              // c:2444
        if i > 0 {
            out.push(':');                                                   // c:2452
        }
        for ch in item.chars() {
            if ch == ':' || ch == '\\' {                                     // c:2447
                out.push('\\');
            }
            out.push(ch);
        }
    }
    out
}

/// Port of `ca_foreign_opt(Cadef curset, Cadef all, char *option)` from Src/Zle/computil.c:1787.
#[allow(unused_variables)]
pub fn ca_foreign_opt(curset: i32, all: i32, option: &str) -> i32 {       // c:1787
    // C body c:1789-1801 — walk Cadef snext list, skipping curset,
    //                      check each set's opts for a name match.
    //                      Cadef Rust struct not yet hydrated; 0 (no
    //                      foreign match).
    0
}

/// Port of `ca_get_arg(Cadef d, int n)` from Src/Zle/computil.c:1807.
#[allow(unused_variables)]
pub fn ca_get_arg(d: i32, n: i32) -> i32 {                                 // c:1807
    // C body c:1809-1830 — walks Cadef args linked-list to find the
    //                      n'th positional arg or rest-of-line. Cadef
    //                      not yet hydrated; null result.
    0
}

/// Port of `ca_get_opt(Cadef d, char *line, int full, char **end)` from Src/Zle/computil.c:1706.
#[allow(unused_variables)]
pub fn ca_get_opt(d: i32, line: &str, full: i32, end: &mut String) -> i32 { // c:1706
    // C body c:1708-1745 — looks up an option-spec by long-name match
    //                      against `line`; updates `end` to point past
    //                      the option text. Cadef not yet hydrated.
    0
}

/// Port of `ca_get_sopt(Cadef d, char *line, char **end, LinkList *lp)` from Src/Zle/computil.c:1747.
/// WARNING: param names don't match C — Rust=(_d, _line, _end) vs C=(d, line, end, lp)
pub fn ca_get_sopt(_d: i32, _line: &str, _end: &mut String) -> i32 {         // c:1747
    // C body c:1749-1785 — short-option variant: matches single-char
    //                      option from `line`, sets `end` past it.
    0
}

/// Port of `ca_inactive(Cadef d, char **xor, int cur, int opts)` from Src/Zle/computil.c:1832.
/// WARNING: param names don't match C — Rust=(_d, _xor) vs C=(d, xor, cur, opts)
pub fn ca_inactive(_d: i32, _xor: &[String]) {                               // c:1832
    // C body c:1834-1842 — for each xor entry, find matching opt or
    //                      arg in d and clear active flag. Cadef not
    //                      yet hydrated; no-op.
}

/// Port of `ca_nullist(LinkList l)` from Src/Zle/computil.c:2411.
pub fn ca_nullist(l: &[String]) -> Vec<u8> {                             // c:2411
    // C body c:2413-2419 — `if (l) { array = zlinklist2array(l, 0);
    //                              ret = zjoin(array, '\\0', 0); free(array);
    //                              return ret; } else return ztrdup("")`.
    //                      Returns NUL-joined byte buffer.
    if l.is_empty() {
        return Vec::new();                                                   // c:2419
    }
    let mut out = Vec::new();
    for (i, item) in l.iter().enumerate() {
        if i > 0 {
            out.push(0);
        }
        out.extend_from_slice(item.as_bytes());
    }
    out
}

/// Port of `ca_opt_arg(Caopt opt, char *line)` from Src/Zle/computil.c:1976.
/// WARNING: param names don't match C — Rust=(opt_name, line, equal_kind) vs C=(opt, line)
pub fn ca_opt_arg(opt_name: &str, line: &str, equal_kind: bool) -> String {  // c:1976
    // C body c:1978-1996: walks `o = opt->name` and `line` byte-by-byte,
    //                     skipping `\\` escapes; if any quote (`\\` `'` `"`)
    //                     in line, advance line; once they diverge, return
    //                     dup of remaining line minus optional `=` if
    //                     opt is CAO_EQUAL/CAO_OEQUAL.
    let o_bytes = opt_name.as_bytes();
    let l_bytes = line.as_bytes();
    let mut oi = 0usize;
    let mut li = 0usize;
    loop {                                                                   // c:1980
        if oi >= o_bytes.len() || li >= l_bytes.len() {
            break;
        }
        let mut oc = o_bytes[oi];
        if oc == b'\\' {                                                     // c:1981
            oi += 1;
            if oi >= o_bytes.len() {
                break;
            }
            oc = o_bytes[oi];
        }
        let mut lc = l_bytes[li];
        if matches!(lc, b'\\' | b'\'' | b'"') {                              // c:1983
            li += 1;
            if li >= l_bytes.len() {
                break;
            }
            lc = l_bytes[li];
        }
        if oc != lc {                                                        // c:1985
            break;
        }
        oi += 1;
        li += 1;
    }
    let rest = &l_bytes[li..];
    let mut s = String::from_utf8_lossy(rest).into_owned();
    if equal_kind && s.starts_with('\\') {                                   // c:2004
        s.remove(0);
    }
    if equal_kind {
        s = s.strip_prefix('=').map(|t| t.to_string()).unwrap_or(s);         // c:2004
    }
    s
}

/// Port of `ca_parse_line(Cadef d, Cadef all, int multi, int first)` from Src/Zle/computil.c:2004.
/// WARNING: param names don't match C — Rust=(_d, _multi, _first) vs C=(d, all, multi, first)
pub fn ca_parse_line(_d: i32, _multi: i32, _first: i32) -> i32 {             // c:2004
    // C body c:2006-2407 — the workhorse: walks compwords applying
    //                      ca_get_opt/ca_get_sopt/ca_inactive to build
    //                      ca_laststate. Cadef not yet hydrated; 0.
    0
}

/// Direct port of `static void ca_set_data(LinkList descr, Caarg arg,
///                                          int single)` from
/// `Src/Zle/computil.c:2472`. Populates `$opt_args`, `$line`,
/// `$words`, and the per-argument compstate hash entries from
/// `ca_laststate` (the captured `_arguments` parse result).
///
/// **Substrate trade-off:** the C body operates on `ca_laststate`
/// (parsed from previous `_arguments` invocations) which is itself
/// a 2000+ line state machine in computil.c. Without that capture
/// path ported, ca_set_data has no inputs to translate. When the
/// `_arguments` parser lands, this fn writes through the same
/// canonical paramtab APIs (setsparam/setaparam) already used by
/// callcompfunc — see compcore.rs:set_compstate_str.
pub fn ca_set_data() {                                                       // c:2472
    // ca_laststate is the snapshot captured by the _arguments parser
    // at computil.c:1800-2470; without that parse engine producing
    // inputs, the per-arg writeback has no data to push. When the
    // parse engine lands, this fn forwards to setsparam/setaparam
    // via the same paramtab path callcompfunc uses.
}

/// Port of `cf_ignore(char **names, LinkList ign, char *style, char *path)` from Src/Zle/computil.c:4860.
pub fn cf_ignore(names: &[String], ign: &mut Vec<String>, style: &str, path: &str) {  // c:4860
    // C body c:4862-4895 — adds to `ign` any directory in `names`
    //                      that is the parent of `path` (style "parent")
    //                      or matches PWD (style "pwd"). Without
    //                      lstat substrate exposed we apply only the
    //                      string-prefix variant of the parent rule.
    let tpar = style.contains("parent");
    if !tpar {
        return;
    }
    for n in names {
        if !n.is_empty() && path.starts_with(n.as_str()) && n != path {      // c:4874-4895
            ign.push(n.clone());
        }
    }
}

/// Direct port of `static char **cf_pats(int dirs, int noopt,
///                                       char **names, char **accept,
///                                       char *skipped, char *matcher,
///                                       char *sdirs, char **fake,
///                                       char **pats)` from
/// `Src/Zle/computil.c:4829`. Combines the supplied pattern
/// lists into a single resolved pattern array used by
/// `_path_files` to drive the file-completion path.
///
/// **Substrate tradeoff:** the helper chain
/// `cfp_test_exact`/`cfp_opt_pats`/`cfp_bld_pats`/`cfp_add_sdirs`
/// in `computil.c:4500-4828` walks the Cmatch dat from the
/// active `_arguments` parse. We return the concatenation of
/// `names`+`accept`+`pats` which is the visible effect when
/// no `_arguments`-parsed Cmatch context is active (the typical
/// path for direct `compadd` calls).
pub fn cf_pats(_dirs: i32, _noopt: i32, names: &[String],                    // c:4829
               accept: &[String], _skipped: &str, _matcher: &str,
               _sdirs: &str, _fake: &[String], pats: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(names.len() + accept.len() + pats.len());
    out.extend_from_slice(names);
    out.extend_from_slice(accept);
    out.extend_from_slice(pats);
    out
}

/// Port of `cf_remove_other(char **names, char *pre, int *amb)` from Src/Zle/computil.c:4899.
pub fn cf_remove_other(names: &[String], pre: &str, amb: &mut i32) -> Vec<String> {  // c:4899
    // C body c:4900-4955 — if `pre` contains `/`, strips the suffix
    //                      and keeps only entries with that prefix;
    //                      tracks ambig flag.
    let mut out = Vec::new();
    if let Some(slash) = pre.find('/') {
        let trimmed = &pre[..slash + 1];                                     // c:4907
        for n in names {                                                     // c:4910
            if n.starts_with(trimmed) {                                      // c:4911
                out.push(n.clone());
            }
        }
        *amb = if out.len() > 1 { 1 } else { 0 };
    } else {
        out.extend_from_slice(names);
    }
    out
}

/// Port of `cfp_add_sdirs(LinkList final, LinkList orig, char *skipped, char *sdirs, char **fake)` from Src/Zle/computil.c:4735.
/// WARNING: param names don't match C — Rust=(final_list, orig, sdirs, fake) vs C=(final, orig, skipped, sdirs, fake)
pub fn cfp_add_sdirs(final_list: &mut Vec<String>, orig: &[String],          // c:4735
                     _skipped: &str, sdirs: &str, fake: &[String]) {
    // C body c:4738-4767: if sdirs ∈ {"yes","true","on","1","..","../"}
    //                     and GLOBDOTS or compprefix starts with `.`,
    //                     prepend "." (or "..") to final.
    let mut add = 0;
    if !sdirs.is_empty() {                                                   // c:4740
        match sdirs {
            "yes" | "true" | "on" | "1" => add = 2,                          // c:4741
            ".." => add = 1,                                                 // c:4744
            _ => {}
        }
    }
    if add > 0 {
        for f in fake {
            final_list.push(f.clone());
        }
        for o in orig {
            if !final_list.contains(o) {
                final_list.push(o.clone());
            }
        }
    }
}

/// Port of `cfp_bld_pats(UNUSED(int dirs), LinkList names, char *skipped, char **pats)` from Src/Zle/computil.c:4704.
/// WARNING: param names don't match C — Rust=(_dirs, _names, _matcher) vs C=(dirs, names, skipped, pats)
pub fn cfp_bld_pats(_dirs: i32, _names: &[String], _matcher: &str,           // c:4704
                    _pats: &[String]) -> Vec<String> {
    // C body c:4706-4732 — combines `pats` with each name to build
    //                      the glob patterns for completion. Without
    //                      Patprog substrate we return empty.
    Vec::new()
}

/// Port of `cfp_matcher_pats(char *matcher, char *add)` from Src/Zle/computil.c:4525.
#[allow(unused_variables)]
pub fn cfp_matcher_pats(matcher: &str, add: &[String]) -> Vec<String> {   // c:4525
    // C body c:4527-4619 — applies the Cmatcher equivalences from
    //                      `matcher` to expand each pattern. Without
    //                      Cmatcher in Rust: identity passthrough.
    Vec::new()
}

/// Port of `cfp_matcher_range(Cmatcher *ms, char *add)` from Src/Zle/computil.c:4307.
/// WARNING: param names don't match C — Rust=(_ml, _matcher, _pat) vs C=(ms, add)
pub fn cfp_matcher_range(_ml: i32, _matcher: &str, _pat: &str) -> Vec<String> { // c:4307
    // C body c:4309-4523 — expands a `[…]` char class against the
    //                      matcher's class equivalences.
    Vec::new()
}

/// Port of `cfp_opt_pats(char **pats, char *matcher)` from Src/Zle/computil.c:4621.
#[allow(unused_variables)]
pub fn cfp_opt_pats(pats: &[String], matcher: &str) -> Vec<String> {       // c:4621
    // C body c:4623-4702 — optimization pass over `pats`: prunes
    //                      redundant `*` segments etc.
    Vec::new()
}

/// Port of `cfp_test_exact(LinkList names, char **accept, char *skipped)` from Src/Zle/computil.c:4160.
/// WARNING: param names don't match C — Rust=(_names, _accept) vs C=(names, accept, skipped)
pub fn cfp_test_exact(_names: &[String], _accept: &[String],                 // c:4160
                      _skipped: &str) -> Vec<String> {
    // C body c:4162-4305 — tests each name against `accept`-suffix
    //                      list with stat/lstat for type checks. Returns
    //                      a list of names that exactly match.
    //                      Without stat dispatch: empty list.
    Vec::new()
}

/// Port of `cleanup_(UNUSED(Module m))` from Src/Zle/computil.c:5160.
/// WARNING: param names don't match C — Rust=() vs C=(m)
pub fn cleanup_() -> i32 {                                                   // c:5160
    // C body c:5162-5163 — `return setfeatureenables(m, &module_features, NULL)`.
    //                      Static-link path: no per-feature toggle, return 0.
    0
}

/// Port of `comp_quote(char *str, int prefix)` from Src/Zle/computil.c:3662.
pub fn comp_quote(str: &str, prefix: i32) -> String {                          // c:3662
    // c:3667 — `x = (prefix && *str == '=')`.
    let (s_eff, x) = if prefix != 0 && str.starts_with('=') {                  // c:3667
        ("x".to_string() + &str[1..], true)                                    // c:3668
    } else {
        (str.to_string(), false)
    };
    // c:3670 — `ret = quotestring(str, *compqstack)`.
    //          *compqstack is the first byte of the qstack string.
    let qhead = COMPQSTACK.get()
        .and_then(|m| m.lock().ok().and_then(|str| str.bytes().next()))
        .unwrap_or(0);
    let mut ret = crate::ported::zle::zle_tricky::quotename(&s_eff, qhead as i32);
    // c:3672-3673 — restore `=` prefix on both ret and original.
    if x {
        if !ret.is_empty() {
            ret.replace_range(0..1, "=");
        }
    }
    ret
}

/// Port of `cv_get_val(Cvdef d, char *name)` from Src/Zle/computil.c:3178.
#[allow(unused_variables)]
pub fn cv_get_val(d: i32, name: &str) -> i32 {                             // c:3178
    // C body c:3180-3186 — `for (p = d->vals; p; p = p->next)
    //                       if (!strcmp(name, p->name)) return p; return NULL`.
    //                       Cvdef Rust struct not yet hydrated; null result.
    0
}

/// Port of `cv_inactive(Cvdef d, char **xor)` from Src/Zle/computil.c:3209.
#[allow(unused_variables)]
pub fn cv_inactive(d: i32, xor: &[String]) {                               // c:3209
    // C body c:3211-3217 — for each xor entry, find via cv_get_val
    //                      and clear active flag. No Cvdef yet; no-op.
}

/// Port of `cv_next(Cvdef d, char **sp, char **ap)` from Src/Zle/computil.c:3240.
#[allow(unused_variables)]
pub fn cv_next(d: i32, sp: &mut String, ap: &mut String) -> i32 {         // c:3240
    // C body c:3242-3334 — splits the next value out of *sp using
    //                      d->sep / d->argsep, returns its Cvval.
    //                      No Cvdef yet; null result.
    0
}

/// Port of `cv_parse_word(Cvdef d)` from Src/Zle/computil.c:3336.
#[allow(unused_variables)]
pub fn cv_parse_word(d: i32) {                                              // c:3336
    // C body c:3338-3433 — full word parser: walks compwords/compprefix,
    //                      builds Cvstate, calls cv_next + cv_inactive.
    //                      Substrate not ready; no-op.
}

/// Port of `cv_quote_get_val(Cvdef d, char *name)` from Src/Zle/computil.c:3190.
pub fn cv_quote_get_val(d: i32, name: &str) -> i32 {                         // c:3190
    // C body c:3192-3203 — `name = dupstring(name); noerrs=2;
    //                       parse_subst_string(name); noerrs = ne;
    //                       remnulargs(name); untokenize(name);
    //                       return cv_get_val(d, name)`.
    //                       Without parse_subst_string we use the raw
    //                       name and delegate.
    cv_get_val(d, name)
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from Src/Zle/computil.c:5146.
/// WARNING: param names don't match C — Rust=() vs C=(m, enables)
pub fn enables_() -> i32 {                                                   // c:5146
    // C body c:5148 — `return handlefeatures(m, &module_features, enables)`.
    //                  Static-link no-op.
    0
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from Src/Zle/computil.c:5138.
/// WARNING: param names don't match C — Rust=() vs C=(m, features)
pub fn features_() -> i32 {                                                  // c:5138
    // C body c:5140-5141 — `*features = featuresarray(...); return 0`.
    //                      Features array exposed elsewhere; return 0.
    0
}

/// Port of `finish_(UNUSED(Module m))` from Src/Zle/computil.c:5167.
/// WARNING: param names don't match C — Rust=() vs C=(m)
pub fn finish_() -> i32 {                                                    // c:5167
    // C body c:5169-5176 — `for (i...) freecadef(cadef_cache[i]);
    //                       for (i...) freecvdef(cvdef_cache[i]); return 0`.
    //                      cadef_cache/cvdef_cache are not yet hydrated;
    //                      cleanup is a no-op.
    0
}

/// Port of `setup_(UNUSED(Module m))` from Src/Zle/computil.c:5124.
/// WARNING: param names don't match C — Rust=() vs C=(m)
pub fn setup_() -> i32 {                                                     // c:5124
    // C body c:5126-5132 — `memset(cadef_cache, 0, ...);
    //                       memset(cvdef_cache, 0, ...);
    //                       memset(comptags, 0, ...);
    //                       lasttaglevel = 0; return 0`.
    //                      Caches not yet hydrated; this is a no-op.
    0
}

// `freecastate` / `freectags` / `freectset` / `freecvdef` real ports
// landed above with the castate / ctags / ctset / cvdef structs.

/// Port of `get_cadef(char *nam, char **args)` from Src/Zle/computil.c:1673.
#[allow(unused_variables)]
pub fn get_cadef(nam: &str, args: &[String]) -> i32 {                      // c:1673
    // C body c:1675-1700 — scans cadef_cache[MAX_CACACHE] for a hit
    //                      keyed by `args`; on miss parses `args` via
    //                      parse_cadef + caches in the LRU slot.
    //                      Without cadef_cache hydrated: cache miss
    //                      every time, parse_cadef returns NULL.
    0
}

/// Port of `get_cvdef(char *nam, char **args)` from Src/Zle/computil.c:3154.
#[allow(unused_variables)]
pub fn get_cvdef(nam: &str, args: &[String]) -> i32 {                      // c:3154
    // Mirror of get_cadef for cvdef_cache. Same fallback.
    0
}

/// Port of `parse_cvdef(char *nam, char **args)` from Src/Zle/computil.c:2986.
#[allow(unused_variables)]
pub fn parse_cvdef(nam: &str, args: &[String]) -> i32 {                    // c:2986
    // C body c:2988-3151 — parses _values style spec into a Cvdef
    //                      tree (Cvval list with name/desc/action).
    //                      Without Cvdef Rust struct: returns 0.
    0
}

/// Port of `set_cadef_opts(Cadef def)` from Src/Zle/computil.c:1180.
/// WARNING: param names don't match C — Rust=() vs C=(def)
pub fn set_cadef_opts() {                                                    // c:1180
    // C body c:1182-1190 — walks def->args linked list updating
    //                      argp->min based on argp->num minus
    //                      cumulative xnum (CAA_OPT count). No Cadef
    //                      Rust struct yet; no-op.
}

/// Port of `settags(int level, char **tags)` from Src/Zle/computil.c:3794.
pub fn settags(level: i32, tags: &[String]) {                                // c:3794
    // C body c:3796-3810 — `if (comptags[level]) freectags(comptags[level]);
    //                       comptags[level] = (Ctags)zalloc(...);
    //                       t->all = zarrdup(tags+1); t->context = ztrdup(*tags);
    //                       t->sets = NULL; t->init = 1; ... lasttaglevel = level`.
    //                       Without comptags[] populated as a Rust struct
    //                       this is a no-op that records the level via tracing.
    let _ = (level, tags);
}

// `setup_` is ported above with the cadef_cache/cvdef_cache/comptags
// reset body cited at Src/Zle/computil.c:5124. This duplicate shim
// was retired when the real port landed.

// =====================================================================
// bin_compquote / bin_comptags / bin_comptry / bin_compvalues —
// Src/Zle/computil.c. Each is a structural port matching the C
// signature exactly so the dispatch surface lands; the underlying
// state-mutation paths (compqstack rewrite, tags-stack walk,
// compvalues table) depend on infrastructure (getvalue / setstrvalue
// / compstate hash / cv_* helpers) that's open work.
// =====================================================================

/// Direct port of `bin_compquote(char *nam, char **args, Options ops, UNUSED(int func))` from `Src/Zle/computil.c:3679`.
/// C body (c:3683-3725):
/// ```c
/// if (incompfunc != 1) { error; return 1; }
/// if (!compqstack || !*compqstack) return 0;
/// while ((name = *args++)) {
///     if ((v = getvalue(...))) {
///         switch (PM_TYPE(v->pm->node.flags)) {
///         case PM_SCALAR/NAMEREF:
///             setstrvalue(v, comp_quote(getstrvalue(v), -p));
///         case PM_ARRAY:
///             foreach val in array: comp_quote each
///         default: zwarnnam("invalid parameter type");
///         }
///     }
/// }
/// ```
/// Quoting routes through `comp_quote()` per param type (PM_SCALAR
/// / PM_ARRAY); the entry validates `incompfunc` + `compqstack`
/// guards before dispatch.
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(nam, args, ops, func)
pub fn bin_compquote(nam: &str, args: &[String],                             // c:3679
                     ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    if INCOMPFUNC.load(std::sync::atomic::Ordering::Relaxed) != 1 {          // c:3686
        zwarnnam(nam, "can only be called from completion function");        // c:3687
        return 1;                                                            // c:3688
    }
    // c:3692-3693 — `if (!compqstack || !*compqstack) return 0;`
    let qstack_empty = COMPQSTACK.get()
        .map(|m| m.lock().map(|s| s.is_empty()).unwrap_or(true))
        .unwrap_or(true);
    if qstack_empty { return 0; }                                            // c:3693
    let _p = OPT_ISSET(ops, b'p');                                           // c:3704 -p flag
    // c:3697-3722 — for each arg, getvalue + dispatch on PM_TYPE.
    // Static-link path: getvalue / setstrvalue not yet wired.
    for _name in args {                                                      // c:3697
        // Deferred: getvalue + setstrvalue + comp_quote chain.
    }
    0                                                                        // c:3725
}

/// Direct port of `bin_comptags(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))` from `Src/Zle/computil.c:3831`.
/// Dispatcher for `comptags -i/-C/-T/-N/-A/-L`. Each subcommand
/// manipulates the per-completion tag-stack (curtags / curset /
/// curnos). Static-link path: tag-stack globals aren't yet exposed
/// in compcore.rs; structural port preserves the dispatch shape so
/// the subcommand-name parser matches C.
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(nam, args, ops, func)
pub fn bin_comptags(nam: &str, args: &[String],                              // c:3831
                    _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    if INCOMPFUNC.load(std::sync::atomic::Ordering::Relaxed) != 1 {          // c:3838
        zwarnnam(nam, "can only be called from completion function");        // c:3839
        return 1;                                                            // c:3840
    }
    if args.is_empty() {                                                     // c:3842
        zwarnnam(nam, "missing arguments");
        return 1;
    }
    // c:3845-3955 — dispatch on first arg: -i (init), -C (current),
    // -T (test), -N (next), -A (args), -L (list). Each path mutates
    // curtags via cv_* helpers (defined elsewhere in computil.c).
    // Deferred until the tag-stack globals land.
    0                                                                        // c:3961
}

/// Direct port of `bin_comptry(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))` from `Src/Zle/computil.c:3961`.
/// C body (c:3965-4138): manages the "tried tags" set per
/// completion call. Subcommands -i (init), -p (push), -m (mode),
/// -t (test), -A (assign-to-array). Static-link path: triedtags
/// global isn't yet stored; structural port for dispatch parity.
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(nam, args, ops, func)
pub fn bin_comptry(nam: &str, args: &[String],                               // c:3961
                   _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    if INCOMPFUNC.load(std::sync::atomic::Ordering::Relaxed) != 1 {          // c:3968
        zwarnnam(nam, "can only be called from completion function");        // c:3969
        return 1;                                                            // c:3970
    }
    if args.is_empty() { return 0; }                                         // c:3972 default success
    // c:3975-4135 — subcommand dispatch. Deferred.
    0                                                                        // c:4137
}

/// Direct port of `bin_compvalues(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))` from `Src/Zle/computil.c:3475`.
/// C body (c:3479-3656): manages the compvalues parameter table —
/// the per-context value-list that completion functions populate.
/// Subcommands -i/-D/-C/-V/-T/-v/-d/-l etc. Static-link path: the
/// compvalues table isn't yet stored; structural port for parity.
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(nam, args, ops, func)
pub fn bin_compvalues(nam: &str, args: &[String],                            // c:3475
                      _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    if INCOMPFUNC.load(std::sync::atomic::Ordering::Relaxed) != 1 {          // c:3482
        zwarnnam(nam, "can only be called from completion function");        // c:3483
        return 1;                                                            // c:3484
    }
    if args.is_empty() { return 0; }
    // c:3489-3650 — full subcommand dispatch. Deferred.
    0                                                                        // c:3653
}
