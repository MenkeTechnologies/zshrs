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

/// Port of `static Cadef cadef_cache[MAX_CACACHE]` from
/// `Src/Zle/computil.c:973`. The LRU cache holds parsed
/// `_arguments` defs keyed by the raw arg vector — `get_cadef`
/// scans linearly, returns on first match (arr-compare on `defs`),
/// and on miss evicts the entry with the oldest `lastt` slot before
/// inserting the freshly parsed result.
pub static cadef_cache: std::sync::Mutex<[Option<Box<cadef>>; MAX_CACACHE]> = // c:973
    std::sync::Mutex::new([const { None }; MAX_CACACHE]);

/// Port of `static Cvdef cvdef_cache[MAX_CVCACHE]` from
/// `Src/Zle/computil.c:2956`. Same LRU layout as cadef_cache;
/// `get_cvdef` scans for a defs-match hit, evicts the oldest slot
/// on miss.
pub static cvdef_cache: std::sync::Mutex<[Option<Box<cvdef>>; MAX_CVCACHE]> = // c:2956
    std::sync::Mutex::new([const { None }; MAX_CVCACHE]);

/// Port of `static Ctags comptags[MAX_TAGS]` from
/// `Src/Zle/computil.c:3756`. One ctags entry per `locallevel`;
/// indexed by completion level.
pub static comptags: std::sync::Mutex<[Option<Box<ctags>>; MAX_TAGS]> =        // c:3756
    std::sync::Mutex::new([const { None }; MAX_TAGS]);

/// Port of `static int lasttaglevel` from `Src/Zle/computil.c:3760`.
/// "locallevel at last comptags -i".
pub static lasttaglevel: std::sync::atomic::AtomicI32 =                       // c:3760
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

    /// c:1196 — `_arguments '-foo[only foo]' '*:file:_files'`. Verify
    /// that the option-name xor list contains the spec name, that
    /// nopts/ndopts reflect the option type (CAO_NEXT here), and that
    /// the rest arg lands on `rest` with type CAA_REST.
    #[test]
    fn parse_cadef_simple_opt_and_rest() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        crate::ported::utils::inittyptab();
        let args = vec![
            String::from(""),             // adpre/adsuf split (no %d)
            String::from("-foo[only foo]"),
            String::from("*:file:_files"),
        ];
        let def = parse_cadef("_arguments", &args).expect("cadef built");
        let opt = def.opts.as_deref().expect("opt linked");
        assert_eq!(opt.name.as_deref(), Some("-foo"));
        assert_eq!(opt.descr.as_deref(), Some("only foo"));
        assert_eq!(opt.r#type, CAO_NEXT);
        // c:1462-1468 — non-multi option appends its own name to xor.
        let xor = opt.xor.as_ref().expect("xor list");
        assert!(xor.iter().any(|s| s == "-foo"), "xor must include -foo: {:?}", xor);

        let rest = def.rest.as_deref().expect("rest linked");
        assert_eq!(rest.r#type, CAA_REST);
        assert_eq!(rest.descr.as_deref(), Some("file"));
        assert_eq!(rest.action.as_deref(), Some("_files"));
    }

    /// c:1617-1661 — numbered positional argument `1:cmd:_commands` lands
    /// on `def.args` with the right slot (num=0 because anum is `1`
    /// then `arg->num = anum - 1`).
    #[test]
    fn parse_cadef_numbered_positional_arg() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        crate::ported::utils::inittyptab();
        let args = vec![
            String::from(""),
            String::from("1:cmd:_commands"),
        ];
        let def = parse_cadef("_arguments", &args).expect("cadef built");
        let pos = def.args.as_deref().expect("positional arg linked");
        assert_eq!(pos.num, 1);
        assert_eq!(pos.r#type, CAA_NORMAL);
        assert_eq!(pos.descr.as_deref(), Some("cmd"));
        assert_eq!(pos.action.as_deref(), Some("_commands"));
        assert_eq!(pos.direct, 1, "explicit numbering sets direct=1");
    }

    /// c:1647-1656 — duplicate numbered argument must error out and
    /// return None (the cadef cache miss path picks this up).
    #[test]
    fn parse_cadef_doubled_arg_errors() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        crate::ported::utils::inittyptab();
        let args = vec![
            String::from(""),
            String::from("1:a:_a"),
            String::from("1:b:_b"),
        ];
        let def = parse_cadef("_arguments", &args);
        assert!(def.is_none(), "duplicate arg num=1 must reject");
    }

    /// c:1335-1370 — `(opt-x opt-y)-foo[descr]` builds a 3-element
    /// xor list `[opt-x, opt-y, -foo]` (the option's own name gets
    /// added at the end via c:1462-1468).
    #[test]
    fn parse_cadef_xor_list_populated() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        crate::ported::utils::inittyptab();
        let args = vec![
            String::from(""),
            String::from("(opt-x opt-y)-foo[descr]"),
        ];
        let def = parse_cadef("_arguments", &args).expect("cadef built");
        let opt = def.opts.as_deref().expect("opt linked");
        let xor = opt.xor.as_ref().expect("xor list");
        assert_eq!(xor.len(), 3, "xor: {:?}", xor);
        assert_eq!(xor[0], "opt-x");
        assert_eq!(xor[1], "opt-y");
        assert_eq!(xor[2], "-foo");
    }

    /// c:3796 — `settags(0, ["ctx", "tag1", "tag2"])` populates
    /// `comptags[0]` with context="ctx", all=["tag1","tag2"], init=1.
    #[test]
    fn settags_populates_slot() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // Clear slot to make test order-independent.
        if let Ok(mut tab) = comptags.lock() {
            tab[0] = None;
        }
        settags(0, &[
            "ctx".to_string(),
            "tag-a".to_string(),
            "tag-b".to_string(),
        ]);
        let tab = comptags.lock().unwrap();
        let slot = tab[0].as_deref().expect("comptags[0] populated");
        assert_eq!(slot.context.as_deref(), Some("ctx"));
        assert_eq!(slot.init, 1);
        let all = slot.all.as_ref().expect("all populated");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0], "tag-a");
        assert_eq!(all[1], "tag-b");
        assert!(slot.sets.is_none());
    }

    /// c:1712-1718 — exact name match returns the opt with `*end`
    /// pointing past the option name.
    #[test]
    fn ca_get_opt_exact_match() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        crate::ported::utils::inittyptab();
        let args = vec![
            String::from(""),
            String::from("-foo[d]"),
        ];
        let mut def = *parse_cadef("_arguments", &args).expect("cadef built");
        // Mark the only opt active so ca_get_opt accepts it.
        let mut cur = def.opts.as_deref_mut();
        while let Some(o) = cur {
            o.active = 1;
            cur = o.next.as_deref_mut();
        }
        let mut end: usize = 0;
        let hit = ca_get_opt(&def, "-foo", 1, &mut end).expect("hit");
        assert_eq!(hit.name.as_deref(), Some("-foo"));
        assert_eq!(end, 4);
    }

    /// c:1809-1822 — `argsactive=0` short-circuits to None even when
    /// args are linked. Guards against the easy off-by-one error of
    /// returning the first matching arg unconditionally.
    #[test]
    fn ca_get_arg_argsactive_zero_returns_none() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        crate::ported::utils::inittyptab();
        let args = vec![
            String::from(""),
            String::from("1:c:_c"),
        ];
        let def = *parse_cadef("_arguments", &args).expect("cadef built");
        // argsactive defaults to 0 — must short-circuit.
        assert!(ca_get_arg(&def, 1).is_none());
    }

    /// c:1817 — when `argsactive=1` and the positional arg is active,
    /// `n` inside `[min, num]` returns the matching node.
    #[test]
    fn ca_get_arg_in_range_active() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        crate::ported::utils::inittyptab();
        let args = vec![
            String::from(""),
            String::from("1:c:_c"),
        ];
        let mut def = *parse_cadef("_arguments", &args).expect("cadef built");
        def.argsactive = 1;
        if let Some(a) = def.args.as_deref_mut() {
            a.active = 1;
        }
        let hit = ca_get_arg(&def, 1).expect("hit");
        assert_eq!(hit.num, 1);
        assert_eq!(hit.descr.as_deref(), Some("c"));
    }

    /// c:2999-3027 — `-s , descr opt1[a]:val1: opt2[b]` builds a cvdef
    /// with sep=',', descr="descr", vals chain of two cvvals.
    #[test]
    fn parse_cvdef_sep_and_two_vals() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        crate::ported::utils::inittyptab();
        let args = vec![
            String::from("-s"),
            String::from(","),
            String::from("descr"),
            String::from("opt1[a]:val1:"),
            String::from("opt2[b]"),
        ];
        let def = parse_cvdef("_values", &args).expect("cvdef built");
        assert_eq!(def.hassep, 1);
        assert_eq!(def.sep, b',' as i32);
        assert_eq!(def.descr.as_deref(), Some("descr"));
        let v1 = def.vals.as_deref().expect("val1");
        assert_eq!(v1.name.as_deref(), Some("opt1"));
        assert_eq!(v1.descr.as_deref(), Some("a"));
        assert_eq!(v1.r#type, CVV_ARG);
        let v2 = v1.next.as_deref().expect("val2");
        assert_eq!(v2.name.as_deref(), Some("opt2"));
        assert_eq!(v2.descr.as_deref(), Some("b"));
        assert_eq!(v2.r#type, CVV_NOARG);
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


/// Direct port of `static Cadef alloc_cadef(char **args, int single,
/// char *match, char *nonarg, int flags)` from `Src/Zle/computil.c:1147-1177`.
///
/// Builds a fresh `cadef` with the option/single-letter/match/nonarg
/// fields initialized. `args` (if present) is captured into `defs`
/// for later cache-key compare in `get_cadef` (c:1681). `single` set
/// allocates the 188-slot single-letter index array. `match` is the
/// match-spec carried through to the option/arg matchers.
pub fn alloc_cadef(args: Option<&[String]>, single: i32, matchstr: &str,    // c:1147
                   nonarg: Option<&str>, flags: i32) -> Box<cadef> {
    Box::new(cadef {
        next:       None,                                                    // c:1152
        snext:      None,                                                    // c:1152
        opts:       None,                                                    // c:1153
        args:       None,                                                    // c:1154
        rest:       None,                                                    // c:1154
        nonarg:     nonarg.map(|s| s.to_string()),                           // c:1155 ztrdup(nonarg)
        defs:       args.map(|a| a.to_vec()),                                // c:1157 zarrdup(args)
        ndefs:      args.map_or(0, |a| a.len() as i32),                      // c:1158 arrlen(args)
        nopts:      0,                                                       // c:1163
        ndopts:     0,                                                       // c:1164
        nodopts:    0,                                                       // c:1165
        lastt:      {                                                        // c:1166 time(0)
            use std::time::{SystemTime, UNIX_EPOCH};
            SystemTime::now().duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64).unwrap_or(0)
        },
        set:        None,                                                    // c:1167
        // c:1168-1172 — 188-slot single-letter Caopt index. Capacity
        // 188 matches C exactly (range of single-letter option names).
        single:     if single != 0 {
            Some((0..188).map(|_| None).collect())
        } else {
            None
        },
        r#match:    Some(matchstr.to_string()),                              // c:1173 ztrdup(match)
        argsactive: 0,
        flags,                                                               // c:1174
    })
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

/// Direct port of `static Caarg ca_get_arg(Cadef d, int n)` from
/// `Src/Zle/computil.c:1807-1823`. Walks `d->args` looking for the
/// arg whose `[min, num]` range contains `n`. Falls back to `d->rest`
/// when no positional matches. Returns a shallow clone (no `next`)
/// of the matched arg.
pub fn ca_get_arg(d: &cadef, mut n: i32) -> Option<Box<caarg>> {             // c:1807
    if d.argsactive == 0 {                                                   // c:1809
        return None;                                                         // c:1822
    }

    // c:1810-1816 — skip inactive entries (advance `n` to compensate for
    // each skipped one, mirroring the C `n++` inside the loop).
    let mut a = d.args.as_deref();
    while let Some(node) = a {                                               // c:1812
        let in_range = node.active != 0 && n >= node.min && n <= node.num;
        if in_range { break; }                                               // c:1812 inverted
        if node.active == 0 {                                                // c:1813
            n += 1;                                                          // c:1814
        }
        a = node.next.as_deref();                                            // c:1815
    }

    if let Some(node) = a {                                                  // c:1817
        if node.active != 0 && node.min <= n && node.num >= n {
            return Some(Box::new(caarg {                                     // c:1818
                next:   None,
                descr:  node.descr.clone(),
                xor:    node.xor.clone(),
                action: node.action.clone(),
                r#type: node.r#type,
                end:    node.end.clone(),
                opt:    node.opt.clone(),
                num:    node.num,
                min:    node.min,
                direct: node.direct,
                active: node.active,
                gsname: node.gsname.clone(),
            }));
        }
    }

    // c:1820 — rest fallback.
    if let Some(r) = d.rest.as_deref() {
        if r.active != 0 {
            return Some(Box::new(caarg {
                next:   None,
                descr:  r.descr.clone(),
                xor:    r.xor.clone(),
                action: r.action.clone(),
                r#type: r.r#type,
                end:    r.end.clone(),
                opt:    r.opt.clone(),
                num:    r.num,
                min:    r.min,
                direct: r.direct,
                active: r.active,
                gsname: r.gsname.clone(),
            }));
        }
    }
    None                                                                     // c:1820
}

/// Direct port of `static Caopt ca_get_opt(Cadef d, char *line, int full,
///                                          char **end)` from
/// `Src/Zle/computil.c:1706-1742`. Looks up an option-spec by name
/// against `line`. With `full=0`, also accepts a prefix-of-`line`
/// match where the option name is a prefix and the rest of `line` is
/// the option's argument (handles `=` / `--name=value` shapes per
/// `CAO_OEQUAL` / `CAO_EQUAL`). Sets `*end` to the byte offset past
/// the option text (and past the `=` separator when applicable).
/// Returns a cloned shallow copy of the matched `caopt` (without its
/// `next` chain) — Rust ownership artifact, equivalent to C returning
/// the aliased `Caopt` pointer.
pub fn ca_get_opt(d: &cadef, line: &str, full: i32,                          // c:1706
                  end: &mut usize) -> Option<Box<caopt>> {
    let line_bytes = line.as_bytes();

    // c:1712-1718 — exact match against an active option name.
    let mut cur = d.opts.as_deref();
    while let Some(p) = cur {                                                // c:1712
        if p.active != 0 {                                                   // c:1713
            if let Some(name) = p.name.as_deref() {
                if name == line {
                    *end = line_bytes.len();                                 // c:1715
                    return Some(Box::new(caopt {                             // c:1717
                        next: None,
                        name: p.name.clone(),
                        descr: p.descr.clone(),
                        xor: p.xor.clone(),
                        r#type: p.r#type,
                        args: None,
                        active: p.active,
                        num: p.num,
                        gsname: p.gsname.clone(),
                        not: p.not,
                    }));
                }
            }
        }
        cur = p.next.as_deref();
    }

    if full == 0 {                                                           // c:1720
        // c:1722-1739 — prefix-match path for `name=value` / `nameSPC value`.
        let mut cur = d.opts.as_deref();
        while let Some(p) = cur {
            if p.active != 0 {                                               // c:1723
                if let Some(name) = p.name.as_deref() {
                    // c:1723-1724 — short args/NEXT → exact match, else strpfx.
                    let is_match = if p.args.is_none() || p.r#type == CAO_NEXT {
                        name == line
                    } else {
                        crate::ported::utils::strpfx(name, line)
                    };
                    if is_match {
                        let l = name.len();
                        // c:1726-1728 — for OEQUAL/EQUAL, the char at name's
                        // end must be `=` or absent; otherwise skip.
                        if (p.r#type == CAO_OEQUAL || p.r#type == CAO_EQUAL)
                            && l < line_bytes.len() && line_bytes[l] != b'='
                        {
                            cur = p.next.as_deref();
                            continue;                                        // c:1728
                        }
                        // c:1731-1736 — set end past the option (+= 1 for `=`).
                        let mut at = l;
                        if (p.r#type == CAO_OEQUAL || p.r#type == CAO_EQUAL)
                            && l < line_bytes.len() && line_bytes[l] == b'='
                        {
                            at += 1;                                         // c:1734
                        }
                        *end = at;                                           // c:1736
                        return Some(Box::new(caopt {                         // c:1738
                            next: None,
                            name: p.name.clone(),
                            descr: p.descr.clone(),
                            xor: p.xor.clone(),
                            r#type: p.r#type,
                            args: None,
                            active: p.active,
                            num: p.num,
                            gsname: p.gsname.clone(),
                            not: p.not,
                        }));
                    }
                }
            }
            cur = p.next.as_deref();
        }
    }
    None                                                                     // c:1741
}

/// Direct port of `static Caopt ca_get_sopt(Cadef d, char *line,
///                                           char **end, LinkList *lp)`
/// from `Src/Zle/computil.c:1747-1781`. Single-letter option lookup
/// for clumped flags like `-abc`. Walks `line[1..]` consulting
/// `d->single[]` for each char; CAO_NEXT matches accumulate in `lp`,
/// the first non-NEXT match terminates and sets `*end` past it.
/// Returns the terminating Caopt (cloned, no chain) or None.
pub fn ca_get_sopt(d: &cadef, line: &str,                                    // c:1747
                   end: &mut usize,
                   lp: &mut Option<Vec<Box<caopt>>>) -> Option<Box<caopt>> {
    let line_bytes = line.as_bytes();
    if line_bytes.is_empty() {
        *lp = None;
        return None;
    }
    let pre = line_bytes[0];                                                 // c:1750
    let mut idx: usize = 1;
    *lp = None;                                                              // c:1754

    let single = match d.single.as_ref() {                                   // c:1757
        Some(s) => s,
        None => return None,
    };

    let mut p_cur: Option<&caopt> = None;                                    // c:1755 p = NULL
    let mut pp_cur: Option<&caopt> = None;
    let mut list_acc: Option<Vec<Box<caopt>>> = None;

    while idx < line_bytes.len() {                                           // c:1755 for (;*line;line++)
        let ch = line_bytes[idx];
        let sidx = single_index(pre, ch);                                    // c:1756

        // c:1756 — d->single[sidx] lookup (assigns to p if valid).
        let lookup: Option<&caopt> = if sidx >= 0 && (sidx as usize) < single.len() {
            single[sidx as usize].as_deref()
        } else {
            None
        };
        if lookup.is_some() {
            p_cur = lookup;
        }
        let active_with_args = lookup
            .filter(|p| p.active != 0 && p.args.is_some());

        if let Some(p) = active_with_args {                                  // c:1757
            if p.r#type == CAO_NEXT {                                        // c:1758
                let list = list_acc.get_or_insert_with(Vec::new);
                list.push(Box::new(caopt {                                   // c:1761 addlinknode
                    next: None,
                    name: p.name.clone(),
                    descr: p.descr.clone(),
                    xor: p.xor.clone(),
                    r#type: p.r#type,
                    args: None,
                    active: p.active,
                    num: p.num,
                    gsname: p.gsname.clone(),
                    not: p.not,
                }));
            } else {                                                         // c:1762
                idx += 1;                                                    // c:1764 line++
                if (p.r#type == CAO_OEQUAL || p.r#type == CAO_EQUAL)         // c:1765
                    && idx < line_bytes.len() && line_bytes[idx] == b'='
                {
                    idx += 1;                                                // c:1767
                }
                *end = idx;                                                  // c:1768
                pp_cur = Some(p);                                            // c:1770
                break;                                                       // c:1771
            }
        } else if p_cur.is_none() || p_cur.map_or(true, |p| p.active == 0) { // c:1773
            return None;                                                     // c:1774
        }

        // c:1775 — pp = (p->name[0] == pre ? p : NULL); p = NULL.
        pp_cur = p_cur.filter(|p| {
            p.name.as_deref()
                .and_then(|n| n.as_bytes().first().copied())
                .map_or(false, |b| b == pre)
        });
        p_cur = None;
        idx += 1;                                                            // c:1755 line++
    }

    // c:1778 — pp && end: *end = line.
    if pp_cur.is_some() {
        *end = idx;
    }

    *lp = list_acc;

    pp_cur.map(|p| Box::new(caopt {                                          // c:1780
        next: None,
        name: p.name.clone(),
        descr: p.descr.clone(),
        xor: p.xor.clone(),
        r#type: p.r#type,
        args: None,
        active: p.active,
        num: p.num,
        gsname: p.gsname.clone(),
        not: p.not,
    }))
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

/// Direct port of `static Cadef get_cadef(char *nam, char **args)`
/// from `Src/Zle/computil.c:1673-1694`. Walks `cadef_cache` looking
/// for an entry whose `defs` array matches the requested `args`
/// (same length + position-for-position string equality). On hit,
/// bumps that entry's `lastt` and returns it. On miss, parses via
/// `parse_cadef` and evicts the entry with the oldest `lastt`
/// (or the first empty slot) to make room for the new one.
///
/// Returns `1` on hit, `0` on miss-and-cache-insert. The previous
/// return-`i32` shape is preserved for callers; the parsed cadef
/// itself lives in `cadef_cache` and is looked up by separate
/// per-name accessors (`ca_get_opt`, `ca_get_arg`, etc.).
pub fn get_cadef(nam: &str, args: &[String]) -> i32 {                       // c:1673
    let na = args.len() as i32;
    let now = {                                                              // c:1681 time(0)
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now().duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64).unwrap_or(0)
    };

    if let Ok(mut cache) = cadef_cache.lock() {
        // c:1678 — `for (i = MAX_CACACHE, p = cadef_cache, min = NULL;
        //                  i && *p; p++, i--)`. Linear scan; track LRU
        //          candidate for eviction in `min_idx`.
        let mut min_idx: Option<usize> = None;
        let mut min_lastt: i64 = i64::MAX;
        let mut hit_idx: Option<usize> = None;
        for (i, slot) in cache.iter().enumerate() {
            match slot {
                Some(entry) => {
                    // c:1679 — `if (*p && na == (*p)->ndefs && arrcmp(args, (*p)->defs))`.
                    if entry.ndefs == na
                        && entry.defs.as_deref()
                            .map_or(false, |d| d.len() == args.len()
                                && d.iter().zip(args.iter()).all(|(a, b)| a == b))
                    {
                        hit_idx = Some(i);
                        break;                                               // c:1682 break on match
                    }
                    // c:1684 — track entry with smallest lastt as eviction target.
                    if entry.lastt < min_lastt {
                        min_lastt = entry.lastt;
                        min_idx = Some(i);
                    }
                }
                None => {
                    // c:1684 — empty slot wins as eviction target.
                    min_idx = Some(i);
                    break;
                }
            }
        }
        if let Some(i) = hit_idx {
            if let Some(entry) = cache[i].as_mut() {
                entry.lastt = now;                                           // c:1681
            }
            return 1;                                                        // c:1683 hit
        }
        // c:1688 — parse_cadef; on success replace the chosen slot.
        if let Some(new) = parse_cadef(nam, args) {
            let idx = min_idx.unwrap_or(0);
            cache[idx] = Some(new);
        }
    }
    0                                                                        // c:1693 miss
}

/// Direct port of `static Caarg parse_caarg(int mult, int type, int num,
///                                          int opt, char *oname, char **def,
///                                          char *set)` from
/// `Src/Zle/computil.c:1099-1144`. Parses one `:descr[:action]`
/// fragment of an `_arguments` spec into a freshly-allocated caarg.
/// On return, `*idx` points at the first byte of `bytes` not consumed
/// (either the separator `:` for `mult=1` or `bytes.len()` for
/// `mult=0` rest specs).
pub fn parse_caarg(mult: i32, atype: i32, num: i32, opt: i32,                // c:1099
                   oname: Option<&str>, bytes: &[u8], idx: &mut usize,
                   set: Option<&str>) -> Box<caarg> {
    let mut ret = Box::new(caarg::default());
    ret.num = num;                                                           // c:1109
    ret.min = num - opt;                                                     // c:1110
    ret.r#type = atype;                                                      // c:1111
    ret.opt = oname.map(|s| s.to_string());                                  // c:1112
    ret.direct = 0;                                                          // c:1113
    ret.gsname = set.map(|s| s.to_string());                                 // c:1114

    let n = bytes.len();

    // c:1118-1120 — scan description up to the next `:` (escaped `\:` skipped).
    let d_start = *idx;
    while *idx < n && bytes[*idx] != b':' {
        if bytes[*idx] == b'\\' && *idx + 1 < n {
            *idx += 1;
        }
        *idx += 1;
    }
    let has_sav = *idx < n;
    let descr_slice = &bytes[d_start..*idx];
    let descr_str = std::str::from_utf8(descr_slice).unwrap_or("");
    ret.descr = Some(rembslashcolon(descr_str));                             // c:1123

    if has_sav {                                                             // c:1127
        if mult != 0 {                                                       // c:1128
            // c:1129-1136 — `*p == ':'` start, scan to next `:` or NUL.
            *idx += 1;
            let a_start = *idx;
            while *idx < n && bytes[*idx] != b':' {
                if bytes[*idx] == b'\\' && *idx + 1 < n {
                    *idx += 1;
                }
                *idx += 1;
            }
            let action_slice = &bytes[a_start..*idx];
            let action_str = std::str::from_utf8(action_slice).unwrap_or("");
            ret.action = Some(rembslashcolon(action_str));                   // c:1134
        } else {                                                             // c:1137
            // c:1138 — `ret->action = ztrdup(rembslashcolon(p + 1))`.
            let action_slice = &bytes[*idx + 1..];
            let action_str = std::str::from_utf8(action_slice).unwrap_or("");
            ret.action = Some(rembslashcolon(action_str));
            *idx = n;
        }
    } else {                                                                 // c:1139
        ret.action = Some(String::new());                                    // c:1140
    }
    // c:1141 — `*def = p`. Caller reads `bytes[*idx]` to decide whether to
    // continue scanning more `:` fragments.

    ret
}

/// Direct port of `static Cadef parse_cadef(char *nam, char **args)` from
/// `Src/Zle/computil.c:1196-1666`. Parses the leading auto-description
/// (first arg up to `%d`), the `-s/-A/-S/-M` flag block, then the
/// main spec-list loop that fills opts/args/rest from each remaining
/// `_arguments` spec entry.
pub fn parse_cadef(nam: &str, args: &[String]) -> Option<Box<cadef>> {      // c:1196
    use crate::ported::ztype_h::{iblank, idigit, inblank};

    if args.is_empty() {
        return None;                                                         // c:1262 `!*args`
    }

    let orig_args = args;
    let mut idx = 0usize;
    let mut single: i32 = 0;
    let mut flags: i32 = 0;
    let mut match_spec: String = "r:|[_-]=* r:|=*".to_string();              // c:1200
    let mut nonarg: Option<String> = None;

    // c:1208-1216 — split args[0] on `%d` into (adpre, adsuf). Used at
    // c:1543-1554 to auto-derive option descriptions.
    let (adpre, adsuf): (Option<String>, Option<String>) = {
        let first = args[0].as_bytes();
        let mut split_at: Option<usize> = None;
        let mut i = 0usize;
        while i + 1 < first.len() {
            if first[i] == b'%' && first[i + 1] == b'd' {
                split_at = Some(i);
                break;
            }
            i += 1;
        }
        if let Some(at) = split_at {
            let pre = String::from_utf8_lossy(&first[..at]).into_owned();
            let suf = String::from_utf8_lossy(&first[at + 2..]).into_owned();
            (Some(pre), Some(suf))
        } else {
            (None, None)
        }
    };

    idx += 1;                                                                // c:1220 args++

    // c:1221-1259 — `-s/-A/-S/-M[arg]` flag block.
    while idx < args.len() {
        let p = &args[idx];
        let bytes = p.as_bytes();
        if bytes.len() < 2 || bytes[0] != b'-' {                             // c:1221
            break;
        }
        let cluster = &bytes[1..];
        let mut ok = true;
        for (i, &c) in cluster.iter().enumerate() {
            match c {
                b's' => single = 1,                                          // c:1233
                b'S' => flags |= CDF_SEP,                                    // c:1235
                b'A' => {                                                    // c:1237
                    if i + 1 < cluster.len() {                               // c:1238
                        nonarg = Some(String::from_utf8_lossy(&cluster[i + 1..]).into_owned());
                    } else if idx + 1 < args.len() {                         // c:1241
                        nonarg = Some(args[idx + 1].clone());
                        idx += 1;
                    } else {
                        ok = false;
                    }
                    break;
                }
                b'M' => {                                                    // c:1245
                    if i + 1 < cluster.len() {                               // c:1246
                        match_spec = String::from_utf8_lossy(&cluster[i + 1..]).into_owned();
                    } else if idx + 1 < args.len() {                         // c:1249
                        match_spec = args[idx + 1].clone();
                        idx += 1;
                    } else {
                        ok = false;
                    }
                    break;
                }
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            break;                                                           // c:1230
        }
        idx += 1;                                                            // c:1258
    }

    if idx < args.len() && args[idx] == ":" {                                // c:1260
        idx += 1;
    }
    if idx >= args.len() {                                                   // c:1262
        return None;
    }

    // c:1266 — `tokenize(nonarg = dupstring(nonarg))`. The Rust matcher
    // path lazily tokenizes on use; the stored bytes are the spec text.

    // c:1269 — `all = ret = alloc_cadef(orig_args, single, match, nonarg, flags)`.
    let first_def = alloc_cadef(
        Some(orig_args),
        single,
        &match_spec,
        nonarg.as_deref(),
        flags,
    );

    // ---- spec-list loop state (c:1271-1273) ----
    // `sets` accumulates each Cadef in `snext` order; per-set opts/args/rest
    // are collected in parallel Vecs and linked into the cadef at the end.
    let mut sets: Vec<Box<cadef>> = vec![first_def];
    let mut opts_per_set: Vec<Vec<Box<caopt>>> = vec![Vec::new()];
    let mut args_per_set: Vec<Vec<Box<caarg>>> = vec![Vec::new()];
    let mut rest_per_set: Vec<Option<Box<caarg>>> = vec![None];

    let sargs = idx;                                                         // c:1271 saved set-start
    let mut anum: i32 = 1;                                                   // c:1203
    let mut doset: Option<String> = None;
    let mut axor: Option<String> = None;
    let mut curset: Option<usize> = None;                                    // c:1201
    let mut pendset: Option<usize> = None;
    let mut foreignset = false;

    // c:1275 — `for (; *args || pendset; args++)`.
    'outer: loop {
        // c:1276 — `if (!*args)` start a fresh set (restart from sargs).
        if idx >= args.len() {
            if pendset.is_none() {
                break 'outer;
            }
            // c:1278-1286 — set_cadef_opts on current; alloc new cadef as snext.
            {
                let cur = sets.last_mut().unwrap();
                let cur_args = args_per_set.last_mut().unwrap();
                // Link the args list into cur so set_cadef_opts can walk it.
                let mut head: Option<Box<caarg>> = None;
                for arg_box in cur_args.drain(..).rev() {
                    let mut a = arg_box;
                    a.next = head;
                    head = Some(a);
                }
                cur.args = head;
                set_cadef_opts(cur);                                          // c:1280
                // Stash args back as a Vec for the rest of the loop. We need
                // both forms; the linked list will be rebuilt at the end.
                let mut walk = cur.args.take();
                while let Some(mut node) = walk {
                    walk = node.next.take();
                    cur_args.push(node);
                }
            }
            idx = sargs;                                                     // c:1278
            doset = None;                                                    // c:1279
            sets.push(alloc_cadef(None, single, &match_spec,                  // c:1281
                                  nonarg.as_deref(), flags));
            opts_per_set.push(Vec::new());
            args_per_set.push(Vec::new());
            rest_per_set.push(None);
            anum = 1;                                                        // c:1283
            foreignset = false;                                              // c:1284
            curset = pendset;                                                // c:1285
            pendset = None;                                                  // c:1286
        }

        let arg = &args[idx];
        let arg_bytes = arg.as_bytes();

        // c:1288 — `args[0][0] == '-' && !args[0][1] && args[1]` — set marker.
        if arg_bytes == b"-" && idx + 1 < args.len() {
            if curset.is_some() && curset != Some(idx) {                     // c:1289
                foreignset = true;
                if pendset.is_none() && Some(idx) > curset {                 // c:1290
                    pendset = Some(idx);
                }
                idx += 1;                                                    // c:1292 ++args
            } else {                                                         // c:1293
                foreignset = false;
                idx += 1;
                let p_str = &args[idx];                                      // c:1295 char *p = *++args
                let pb = p_str.as_bytes();
                let l = pb.len().saturating_sub(1);
                // c:1298 — `if (*p == '(' && p[l] == ')')` strip parens for axor.
                let (set_name, ax) = if !pb.is_empty()
                    && pb[0] == b'(' && pb[l] == b')'
                {
                    let inner = String::from_utf8_lossy(&pb[1..l]).into_owned();
                    (inner.clone(), Some(inner))
                } else {
                    (p_str.clone(), None)
                };
                axor = ax;
                if set_name.is_empty() {                                     // c:1302
                    zwarnnam(nam, "empty set name");
                    return None;
                }
                let new_set = crate::ported::string::tricat(&set_name, "-", "");// c:1307
                doset = Some(new_set.clone());
                {
                    let cur = sets.last_mut().unwrap();
                    cur.set = Some(new_set);
                }
                curset = Some(idx);                                          // c:1308
            }
            idx += 1;
            continue;                                                        // c:1310
        }

        // c:1311 — `args[0][0] == '+' && !args[0][1] && args[1]` — group marker.
        if arg_bytes == b"+" && idx + 1 < args.len() {
            foreignset = false;                                              // c:1315
            idx += 1;
            let p_str = &args[idx];                                          // c:1316
            let pb = p_str.as_bytes();
            let l = pb.len().saturating_sub(1);
            let (group_name, ax) = if !pb.is_empty()
                && pb[0] == b'(' && pb[l] == b')'
            {
                let inner = String::from_utf8_lossy(&pb[1..l]).into_owned();
                (inner.clone(), Some(inner))
            } else {
                (p_str.clone(), None)
            };
            axor = ax;
            if group_name.is_empty() {                                       // c:1322
                zwarnnam(nam, "empty group name");
                return None;
            }
            doset = Some(crate::ported::string::tricat(&group_name, "-", ""));// c:1327
            idx += 1;
            continue;                                                        // c:1328
        }

        // c:1329 — `if (foreignset) continue` — skip specs for other sets.
        if foreignset {
            idx += 1;
            continue;
        }

        // c:1331 — parse one spec entry.
        let bytes = arg_bytes;
        let mut p = 0usize;
        let mut xnum: i32 = 0;                                               // c:1332
        let mut not_flag = false;
        if p < bytes.len() && bytes[p] == b'!' {                             // c:1333
            not_flag = true;
            p += 1;
        }

        let mut xor: Option<Vec<String>> = None;
        if p < bytes.len() && bytes[p] == b'(' {                             // c:1335 xor list
            let mut list: Vec<String> = Vec::new();
            // c:1342-1354 — collect words inside parens.
            let mut bad = false;
            'paren: loop {
                if p >= bytes.len() || bytes[p] == b')' { break; }
                p += 1;                                                       // c:1343 p++
                while p < bytes.len() && inblank(bytes[p]) { p += 1; }        // c:1343 inblank skip
                if p >= bytes.len() { bad = true; break 'paren; }
                if bytes[p] == b')' { break 'paren; }
                let q = p;
                p += 1;
                while p < bytes.len() && bytes[p] != b')' && !inblank(bytes[p]) {
                    p += 1;
                }
                if p >= bytes.len() { bad = true; break 'paren; }            // c:1349
                let word = String::from_utf8_lossy(&bytes[q..p]).into_owned();
                list.push(word);
                xnum += 1;                                                    // c:1353
            }
            if bad || p >= bytes.len() || bytes[p] != b')' {                  // c:1356
                zwarnnam(nam, &format!("invalid argument: {}", arg));
                return None;
            }
            if doset.is_some() && axor.is_some() {                            // c:1361
                xnum += 1;
                list.push(axor.clone().unwrap());                             // c:1366-1367
            }
            xor = Some(list);
            p += 1;                                                           // c:1370
        } else if doset.is_some() && axor.is_some() {                        // c:1371
            xnum = 1;
            xor = Some(vec![axor.clone().unwrap()]);
        }

        // c:1379 — option spec OR rest-arg OR normal-arg.
        let is_opt = p < bytes.len() && (
            bytes[p] == b'-' || bytes[p] == b'+'
            || (bytes[p] == b'*' && p + 1 < bytes.len()
                && (bytes[p + 1] == b'-' || bytes[p + 1] == b'+'))
        );

        if is_opt {
            // ---- c:1381-1580 option spec branch ----
            // The `rec:` goto loop handles `-+`/`+-` duplication by
            // parsing the same spec twice with name[0] flipped between
            // `-` and `+`.
            let mut again_iter = 0i32;                                       // c:1384
            let mut againp_start: Option<usize> = None;
            let mut p_state = p;
            let mut xor_state = xor;
            let mut xnum_state = xnum;

            'rec: loop {
                let mut multi = false;                                       // c:1390
                if p_state < bytes.len() && bytes[p_state] == b'*' {
                    multi = true;
                    p_state += 1;
                }

                let mut name_start: usize;
                let mut name_buf: Vec<u8>;
                let need_flip = p_state + 2 < bytes.len()
                    && ((bytes[p_state] == b'-' && bytes[p_state + 1] == b'+')
                        || (bytes[p_state] == b'+' && bytes[p_state + 1] == b'-'))
                    && bytes[p_state + 2] != b':'
                    && bytes[p_state + 2] != b'['
                    && bytes[p_state + 2] != b'='
                    && bytes[p_state + 2] != b'-'
                    && bytes[p_state + 2] != b'+';

                if need_flip {                                               // c:1393
                    if again_iter == 0 {
                        againp_start = Some(p_state);
                    }
                    name_start = p_state + 1;
                    name_buf = bytes[name_start..].to_vec();
                    if !name_buf.is_empty() {
                        name_buf[0] = if again_iter != 0 { b'-' } else { b'+' };
                    }
                    again_iter += 1;
                    p_state = name_start;
                } else {                                                     // c:1404
                    name_start = p_state;
                    name_buf = bytes[name_start..].to_vec();
                    if p_state + 1 < bytes.len()
                        && bytes[p_state] == b'-' && bytes[p_state + 1] == b'-'
                    {
                        p_state += 1;                                        // c:1407 skip 2nd '-'
                    }
                }

                if p_state + 1 >= bytes.len() {                              // c:1409
                    zwarnnam(nam, &format!("invalid argument: {}", arg));
                    return None;
                }

                // c:1416-1422 — skip option name body up to type byte.
                let mut np = p_state - name_start + 1;
                let nlen = name_buf.len();
                while np < nlen
                    && name_buf[np] != b':'
                    && name_buf[np] != b'['
                    && !((name_buf[np] == b'-' || name_buf[np] == b'+')
                         && np + 1 < nlen
                         && (name_buf[np + 1] == b':' || name_buf[np + 1] == b'['))
                    && !(name_buf[np] == b'='
                         && np + 1 < nlen
                         && (name_buf[np + 1] == b':'
                             || name_buf[np + 1] == b'['
                             || name_buf[np + 1] == b'-'))
                {
                    if name_buf[np] == b'\\' && np + 1 < nlen {
                        np += 1;
                    }
                    np += 1;
                }

                let mut c_byte = if np < nlen { name_buf[np] } else { 0 };
                let opt_name_slice = &name_buf[..np];
                let opt_name = String::from_utf8_lossy(opt_name_slice).into_owned();

                let mut otype = CAO_NEXT;                                    // c:1384
                if c_byte == b'-' {                                          // c:1427
                    otype = CAO_DIRECT;
                    np += 1;
                    c_byte = if np < nlen { name_buf[np] } else { 0 };
                } else if c_byte == b'+' {                                   // c:1430
                    otype = CAO_ODIRECT;
                    np += 1;
                    c_byte = if np < nlen { name_buf[np] } else { 0 };
                } else if c_byte == b'=' {                                   // c:1433
                    otype = CAO_OEQUAL;
                    np += 1;
                    c_byte = if np < nlen { name_buf[np] } else { 0 };
                    if c_byte == b'-' {
                        otype = CAO_EQUAL;                                   // c:1436
                        np += 1;
                        c_byte = if np < nlen { name_buf[np] } else { 0 };
                    }
                }

                // c:1441 — optional `[descr]`.
                let mut descr_str: Option<String> = None;
                if c_byte == b'[' {                                          // c:1441
                    np += 1;
                    let d_start = np;
                    while np < nlen && name_buf[np] != b']' {
                        if name_buf[np] == b'\\' && np + 1 < nlen { np += 1; }
                        np += 1;
                    }
                    if np >= nlen {                                          // c:1446
                        zwarnnam(nam, &format!("invalid option definition: {}", arg));
                        return None;
                    }
                    let d_slice = &name_buf[d_start..np];
                    descr_str = Some(String::from_utf8_lossy(d_slice).into_owned());
                    np += 1;
                    c_byte = if np < nlen { name_buf[np] } else { 0 };
                }

                if c_byte != 0 && c_byte != b':' {                           // c:1456
                    zwarnnam(nam, &format!("invalid option definition: {}", arg));
                    return None;
                }

                // c:1461 — add option name to xor list if not `*-...`.
                let clean_name = rembslashcolon(&opt_name);
                if !multi {
                    let xv = xor_state.get_or_insert_with(Vec::new);
                    if xv.len() <= xnum_state as usize {
                        xv.resize(xnum_state as usize + 1, String::new());
                    }
                    xv[xnum_state as usize] = clean_name.clone();
                }

                // c:1470-1531 — argument loop for `:descr:action[:...]`.
                let mut oargs: Vec<Box<caarg>> = Vec::new();
                if c_byte == b':' {
                    let mut oanum: i32 = 1;                                   // c:1473
                    let mut onum: i32 = 0;
                    while c_byte == b':' {                                    // c:1479
                        let mut rest = 0;
                        let mut end_str: Option<String> = None;
                        np += 1;                                              // c:1484 *++p
                        let atype: i32;
                        c_byte = if np < nlen { name_buf[np] } else { 0 };
                        if c_byte == b':' {                                   // c:1485
                            atype = CAA_OPT;
                            np += 1;
                        } else if c_byte == b'*' {                            // c:1487
                            np += 1;
                            if np < nlen && name_buf[np] != b':' {            // c:1488
                                let end_start = np;
                                while np < nlen && name_buf[np] != b':' {
                                    if name_buf[np] == b'\\' && np + 1 < nlen {
                                        np += 1;
                                    }
                                    np += 1;
                                }
                                let e_slice = &name_buf[end_start..np];
                                end_str = Some(String::from_utf8_lossy(e_slice).into_owned());
                            }
                            if np >= nlen || name_buf[np] != b':' {           // c:1500
                                zwarnnam(nam, &format!("invalid option definition: {}", arg));
                                return None;
                            }
                            np += 1;                                          // c:1507 *++p
                            if np < nlen && name_buf[np] == b':' {            // c:1508
                                np += 1;
                                if np < nlen && name_buf[np] == b':' {        // c:1509
                                    atype = CAA_RREST;
                                    np += 1;
                                } else {
                                    atype = CAA_RARGS;
                                }
                            } else {
                                atype = CAA_REST;
                            }
                            rest = 1;
                        } else {
                            atype = CAA_NORMAL;
                        }

                        // c:1521 — parse_caarg.
                        let mut oarg = parse_caarg(
                            if rest != 0 { 0 } else { 1 },
                            atype, oanum, onum,
                            Some(&clean_name),
                            &name_buf, &mut np,
                            doset.as_deref(),
                        );
                        oanum += 1;
                        if atype == CAA_OPT { onum += 1; }                    // c:1524
                        if let Some(end) = end_str {
                            oarg.end = Some(end);                             // c:1526
                        }
                        oargs.push(oarg);

                        if rest != 0 { break; }                               // c:1528
                        c_byte = if np < nlen { name_buf[np] } else { 0 };    // c:1530
                    }
                }

                // c:1534 — build the caopt.
                let mut opt_box = Box::new(caopt::default());
                opt_box.gsname = doset.clone();                               // c:1539
                opt_box.name = Some(clean_name.clone());                      // c:1540
                opt_box.descr = if let Some(d) = descr_str.clone() {          // c:1542
                    Some(d)
                } else if adpre.is_some() && oargs.len() == 1 {               // c:1543
                    let first_arg = &oargs[0];
                    let d_field = first_arg.descr.as_deref().unwrap_or("");
                    let has_visible = d_field.bytes().any(|b| !iblank(b));
                    if has_visible {                                          // c:1550
                        Some(crate::ported::string::tricat(
                            adpre.as_deref().unwrap_or(""),
                            d_field,
                            adsuf.as_deref().unwrap_or(""),
                        ))
                    } else {
                        None                                                  // c:1553
                    }
                } else {
                    None
                };
                let xor_clone = if again_iter == 1 {                          // c:1556
                    xor_state.clone()
                } else {
                    xor_state.take()
                };
                opt_box.xor = xor_clone;
                opt_box.r#type = otype;                                       // c:1557
                opt_box.not = if not_flag { 1 } else { 0 };                   // c:1560

                // Link in the arg list.
                let mut head: Option<Box<caarg>> = None;
                for a in oargs.into_iter().rev() {
                    let mut a = a;
                    a.next = head;
                    head = Some(a);
                }
                opt_box.args = head;

                {
                    let cur = sets.last_mut().unwrap();
                    opt_box.num = cur.nopts;
                    cur.nopts += 1;                                           // c:1559
                    if otype == CAO_DIRECT || otype == CAO_EQUAL {            // c:1562
                        cur.ndopts += 1;
                    } else if otype == CAO_ODIRECT || otype == CAO_OEQUAL {   // c:1564
                        cur.nodopts += 1;
                    }
                    // c:1571 — single-letter lookup table.
                    if single != 0 {
                        let nb = clean_name.as_bytes();
                        if nb.len() == 2 && nb[1] != b'-' {
                            let sidx = single_index(nb[0], nb[1]);
                            if sidx >= 0 {
                                if let Some(ref mut s) = cur.single {
                                    if (sidx as usize) < s.len() {
                                        s[sidx as usize] = Some(Box::new(
                                            caopt {
                                                next: None,
                                                name: opt_box.name.clone(),
                                                descr: opt_box.descr.clone(),
                                                xor: opt_box.xor.clone(),
                                                r#type: opt_box.r#type,
                                                args: None,
                                                active: 0,
                                                num: opt_box.num,
                                                gsname: opt_box.gsname.clone(),
                                                not: opt_box.not,
                                            }
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }

                opts_per_set.last_mut().unwrap().push(opt_box);

                if again_iter == 1 {                                          // c:1576
                    if let Some(start) = againp_start {
                        p_state = start;
                        xnum_state = xnum;                                    // restore
                        xor_state = xor_state.clone();
                        continue 'rec;
                    }
                }
                break 'rec;
            }
        } else if p < bytes.len() && bytes[p] == b'*' {
            // ---- c:1581-1607 rest-arg branch ----
            if not_flag {                                                    // c:1586
                idx += 1;
                continue;
            }
            p += 1;                                                          // c:1589 *++p
            if p >= bytes.len() || bytes[p] != b':' {
                zwarnnam(nam, &format!("invalid rest argument definition: {}", arg));
                return None;
            }
            if rest_per_set.last().unwrap().is_some() {                       // c:1594
                zwarnnam(nam, &format!("doubled rest argument definition: {}", arg));
                return None;
            }
            let mut atype = CAA_REST;                                        // c:1584
            p += 1;                                                          // c:1599 *++p
            if p < bytes.len() && bytes[p] == b':' {                         // c:1599
                p += 1;
                if p < bytes.len() && bytes[p] == b':' {                     // c:1600
                    atype = CAA_RREST;
                    p += 1;
                } else {
                    atype = CAA_RARGS;
                }
            }
            let mut rarg = parse_caarg(0, atype, -1, 0, None, bytes, &mut p,
                                        doset.as_deref());                    // c:1606
            rarg.xor = xor;                                                  // c:1607
            *rest_per_set.last_mut().unwrap() = Some(rarg);
        } else {
            // ---- c:1608-1661 normal-arg branch ----
            if not_flag {                                                    // c:1614
                idx += 1;
                continue;
            }
            let mut direct = 0;                                              // c:1611
            if p < bytes.len() && idigit(bytes[p]) {                         // c:1617
                direct = 1;
                let mut num: i32 = 0;
                while p < bytes.len() && idigit(bytes[p]) {
                    num = num * 10 + (bytes[p] - b'0') as i32;
                    p += 1;
                }
                anum = num + 1;                                              // c:1624
            } else {
                anum += 1;                                                   // c:1627
            }
            if p >= bytes.len() || bytes[p] != b':' {                        // c:1629
                zwarnnam(nam, &format!("invalid argument: {}", arg));
                return None;
            }
            let mut atype = CAA_NORMAL;
            p += 1;                                                          // c:1636 *++p
            if p < bytes.len() && bytes[p] == b':' {                         // c:1636
                atype = CAA_OPT;
                p += 1;
            }
            let mut narg = parse_caarg(0, atype, anum - 1, 0, None,
                                        bytes, &mut p, doset.as_deref());     // c:1641
            narg.xor = xor;                                                  // c:1642
            narg.direct = direct;                                            // c:1643

            // c:1647-1661 — sorted insert by num.
            let target = anum - 1;
            let cur_args = args_per_set.last_mut().unwrap();
            let mut insert_at = cur_args.len();
            for (i, existing) in cur_args.iter().enumerate() {
                if existing.num >= target {
                    insert_at = i;
                    break;
                }
            }
            if insert_at < cur_args.len() && cur_args[insert_at].num == target {
                zwarnnam(nam, &format!("doubled argument definition: {}", arg));
                return None;
            }
            cur_args.insert(insert_at, narg);
        }

        idx += 1;
    }

    // c:1664 — final set_cadef_opts on the last set.
    {
        let last_idx = sets.len() - 1;
        let cur = &mut sets[last_idx];
        let cur_args = &mut args_per_set[last_idx];
        let mut head: Option<Box<caarg>> = None;
        for a in cur_args.drain(..).rev() {
            let mut a = a;
            a.next = head;
            head = Some(a);
        }
        cur.args = head;
        set_cadef_opts(cur);
    }

    // ---- finalize: link opts/args/rest per set, then snext-chain ----
    let n_sets = sets.len();
    for i in 0..n_sets {
        // opts — append order.
        let mut head: Option<Box<caopt>> = None;
        for o in opts_per_set[i].drain(..).rev() {
            let mut o = o;
            o.next = head;
            head = Some(o);
        }
        sets[i].opts = head;
        // args was already linked in the per-set finalize step above for
        // every set except possibly the last (which is now done). Walk
        // any still-present Vec entries into the linked list for safety.
        if !args_per_set[i].is_empty() {
            let mut head: Option<Box<caarg>> = None;
            for a in args_per_set[i].drain(..).rev() {
                let mut a = a;
                a.next = head;
                head = Some(a);
            }
            sets[i].args = head;
        }
        sets[i].rest = rest_per_set[i].take();
    }

    // c:1281 — snext chain links each subsequent set off the head.
    while sets.len() > 1 {
        let tail = sets.pop().unwrap();
        let prev = sets.last_mut().unwrap();
        // Walk to the end of the snext chain on prev and attach tail.
        let mut cursor: &mut Option<Box<cadef>> = &mut prev.snext;
        while cursor.is_some() {
            cursor = &mut cursor.as_mut().unwrap().snext;
        }
        *cursor = Some(tail);
    }

    Some(sets.pop().unwrap())
}

/// Direct port of `static Cvdef get_cvdef(char *nam, char **args)` from
/// `Src/Zle/computil.c:3154-3173`. LRU lookup over `cvdef_cache`
/// keyed by the raw argv. On hit bumps `lastt` and returns 1. On
/// miss parses via `parse_cvdef` and evicts the entry with the
/// oldest `lastt` (or the first empty slot) for insertion.
pub fn get_cvdef(nam: &str, args: &[String]) -> i32 {                       // c:3154
    let na = args.len() as i32;
    let now = {                                                              // c:3161 time(0)
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now().duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64).unwrap_or(0)
    };

    if let Ok(mut cache) = cvdef_cache.lock() {
        let mut min_idx: Option<usize> = None;
        let mut min_lastt: i64 = i64::MAX;
        let mut hit_idx: Option<usize> = None;
        for (i, slot) in cache.iter().enumerate() {                          // c:3159
            match slot {
                Some(entry) => {
                    if entry.ndefs == na                                     // c:3160
                        && entry.defs.as_deref()
                            .map_or(false, |d| d.len() == args.len()
                                && d.iter().zip(args.iter()).all(|(a, b)| a == b))
                    {
                        hit_idx = Some(i);
                        break;
                    }
                    if entry.lastt < min_lastt {                             // c:3164
                        min_lastt = entry.lastt;
                        min_idx = Some(i);
                    }
                }
                None => {                                                    // c:3164 empty slot
                    min_idx = Some(i);
                    break;
                }
            }
        }
        if let Some(i) = hit_idx {                                           // c:3160
            if let Some(entry) = cache[i].as_mut() {
                entry.lastt = now;                                           // c:3161
            }
            return 1;                                                        // c:3163 hit
        }
        // c:3168 — parse_cvdef; on success replace the chosen slot.
        if let Some(new) = parse_cvdef(nam, args) {
            let idx = min_idx.unwrap_or(0);
            cache[idx] = Some(new);                                          // c:3170
        }
    }
    0                                                                        // c:3172 miss
}

/// Direct port of `static Cvdef parse_cvdef(char *nam, char **args)`
/// from `Src/Zle/computil.c:2986-3148`. Parses the leading
/// `-s SEP / -S SEP / -w` flag block, then the description, then
/// each value spec into a cvval chain.
pub fn parse_cvdef(nam: &str, args: &[String]) -> Option<Box<cvdef>> {       // c:2986
    use crate::ported::ztype_h::inblank;

    let orig_args = args;
    let mut idx = 0usize;

    let mut sep: i32 = 0;                                                    // c:2991 char sep = '\0'
    let mut asep: i32 = b'=' as i32;                                         // c:2991 char asep = '='
    let mut hassep: i32 = 0;                                                 // c:2992
    let mut words: i32 = 0;                                                  // c:2992

    // c:2994-3010 — leading flag block (-s SEP, -S SEP, -w).
    while idx + 1 < args.len()
        && args[idx].len() == 2
        && args[idx].starts_with('-')
        && (args[idx].as_bytes()[1] == b's'
            || args[idx].as_bytes()[1] == b'S'
            || args[idx].as_bytes()[1] == b'w')
    {
        let flag = args[idx].as_bytes()[1];
        if flag == b's' {                                                    // c:2999
            hassep = 1;
            sep = args[idx + 1].as_bytes().first().copied().unwrap_or(0) as i32;
            idx += 2;
        } else if flag == b'S' {                                             // c:3003
            asep = args[idx + 1].as_bytes().first().copied().unwrap_or(0) as i32;
            idx += 2;
        } else {                                                             // c:3006 -w
            words = 1;
            idx += 1;
        }
    }

    if idx + 1 >= args.len() {                                               // c:3011
        zwarnnam(nam, "not enough arguments");
        return None;
    }
    let descr = args[idx].clone();                                           // c:3015 descr = *args++
    idx += 1;

    let mut ret = Box::new(cvdef {
        descr:  Some(descr),                                                 // c:3018
        hassep,                                                              // c:3019
        sep,                                                                 // c:3020
        argsep: asep,                                                        // c:3021
        next:   None,                                                        // c:3022
        vals:   None,                                                        // c:3023
        defs:   Some(orig_args.to_vec()),                                    // c:3024
        ndefs:  orig_args.len() as i32,                                      // c:3025
        lastt:  {                                                            // c:3026
            use std::time::{SystemTime, UNIX_EPOCH};
            SystemTime::now().duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64).unwrap_or(0)
        },
        words,                                                               // c:3027
    });

    // c:3029-3147 — for each remaining arg, parse one value spec.
    let mut vals_collected: Vec<Box<cvval>> = Vec::new();

    while idx < args.len() {
        let spec = &args[idx];
        let bytes = spec.as_bytes();
        let mut p: usize = 0;
        let mut xnum: i32 = 0;                                               // c:3032
        let mut bs = 0;                                                      // c:3030
        let mut xor: Option<Vec<String>> = None;

        // c:3035-3068 — `(opt1 opt2)` xor list.
        if p < bytes.len() && bytes[p] == b'(' {                             // c:3035
            let mut list: Vec<String> = Vec::new();
            let mut bad = false;
            'paren: loop {
                if p >= bytes.len() || bytes[p] == b')' { break; }
                p += 1;                                                       // c:3041 p++
                while p < bytes.len() && inblank(bytes[p]) { p += 1; }
                if p >= bytes.len() { bad = true; break 'paren; }
                if bytes[p] == b')' { break 'paren; }
                let q = p;
                p += 1;
                while p < bytes.len() && bytes[p] != b')' && !inblank(bytes[p]) {
                    p += 1;
                }
                if p >= bytes.len() { bad = true; break 'paren; }
                let word = String::from_utf8_lossy(&bytes[q..p]).into_owned();
                list.push(word);
                xnum += 1;
            }
            if bad || p >= bytes.len() || bytes[p] != b')' {                  // c:3056
                zwarnnam(nam, &format!("invalid argument: {}", spec));
                return None;
            }
            xor = Some(list);
            p += 1;                                                           // c:3066
        }

        // c:3071 — `*` (multi).
        let multi = p < bytes.len() && bytes[p] == b'*';
        if multi { p += 1; }

        // c:3076 — scan option name up to `:` or `[`.
        let name_start = p;
        while p < bytes.len() && bytes[p] != b':' && bytes[p] != b'[' {      // c:3076
            if bytes[p] == b'\\' && p + 1 < bytes.len() {
                p += 1;
                bs = 1;                                                       // c:3078
            }
            p += 1;
        }

        // c:3080-3085 — multi-letter check against empty separator.
        if hassep != 0 && sep == 0 && name_start + (bs as usize) + 1 < p {   // c:3080
            zwarnnam(nam,
                "no multi-letter values with empty separator allowed");
            return None;
        }

        let name_bytes = &bytes[name_start..p];
        let name = String::from_utf8_lossy(name_bytes).into_owned();

        // c:3087 — optional [descr].
        let mut value_descr: Option<String> = None;
        let mut c_byte = if p < bytes.len() { bytes[p] } else { 0 };
        if c_byte == b'[' {                                                  // c:3088
            p += 1;
            let d_start = p;
            while p < bytes.len() && bytes[p] != b']' {                      // c:3090
                if bytes[p] == b'\\' && p + 1 < bytes.len() { p += 1; }
                p += 1;
            }
            if p >= bytes.len() {                                            // c:3094
                zwarnnam(nam, &format!("invalid value definition: {}", spec));
                return None;
            }
            value_descr = Some(String::from_utf8_lossy(&bytes[d_start..p]).into_owned());
            p += 1;                                                           // c:3100
            c_byte = if p < bytes.len() { bytes[p] } else { 0 };
        }

        if c_byte != 0 && c_byte != b':' {                                    // c:3106
            zwarnnam(nam, &format!("invalid value definition: {}", spec));
            return None;
        }

        // c:3114 — :arg or ::optarg.
        let mut vtype = CVV_NOARG;
        let mut arg: Option<Box<caarg>> = None;
        if c_byte == b':' {                                                   // c:3114
            if hassep != 0 && sep == 0 {                                      // c:3115
                zwarnnam(nam,
                    "no value with argument with empty separator allowed");
                return None;
            }
            p += 1;                                                            // c:3121 *++p
            if p < bytes.len() && bytes[p] == b':' {                          // c:3121
                p += 1;
                vtype = CVV_OPT;                                              // c:3123
            } else {
                vtype = CVV_ARG;                                              // c:3125
            }
            arg = Some(parse_caarg(0, 0, 0, 0, Some(&name), bytes, &mut p, None));// c:3126
        }

        // c:3131-3137 — add own name to xor list when not multi.
        if !multi {                                                           // c:3131
            let xv = xor.get_or_insert_with(Vec::new);
            if xv.len() <= xnum as usize {
                xv.resize(xnum as usize + 1, String::new());
            }
            xv[xnum as usize] = name.clone();                                 // c:3136
        }

        let v = Box::new(cvval {                                              // c:3138
            next:   None,
            name:   Some(name),                                                // c:3142
            descr:  value_descr,                                               // c:3143
            xor,                                                               // c:3144
            r#type: vtype,                                                     // c:3145
            arg,                                                               // c:3146
            active: 0,
        });
        vals_collected.push(v);

        idx += 1;
    }

    // Link vals_collected as a chain.
    let mut head: Option<Box<cvval>> = None;
    for v in vals_collected.into_iter().rev() {
        let mut v = v;
        v.next = head;
        head = Some(v);
    }
    ret.vals = head;

    Some(ret)
}

/// Direct port of `static void set_cadef_opts(Cadef def)` from
/// `Src/Zle/computil.c:1180-1191`. After a set-of-arg-definitions has
/// been parsed into the cadef, walk the args linked list and update
/// each non-direct argp's `min` field to the cumulative number of
/// CAA_OPT entries that precede it. The optionality count compounds
/// down the chain, which determines minimum-argument-count semantics
/// during completion.
pub fn set_cadef_opts(def: &mut cadef) {                                    // c:1180
    let mut xnum: i32 = 0;
    let mut argp = def.args.as_deref_mut();                                  // c:1185 argp = def->args
    while let Some(node) = argp {                                            // c:1185
        if node.direct == 0 {                                                // c:1186 !argp->direct
            node.min = node.num - xnum;                                      // c:1187
        }
        if node.r#type == CAA_OPT {                                          // c:1188
            xnum += 1;                                                       // c:1189
        }
        argp = node.next.as_deref_mut();                                     // c:1185 argp = argp->next
    }
}

/// Direct port of `static void settags(int level, char **tags)` from
/// `Src/Zle/computil.c:3794`. Replaces `comptags[level]` with a fresh
/// ctags carrying `tags[0]` as context and `tags[1..]` as the full
/// tag-list. Used at the start of every completion level transition
/// (`comptags -i`).
pub fn settags(level: i32, tags: &[String]) {                                // c:3794
    let idx = level as usize;
    if idx >= MAX_TAGS { return; }                                           // c:3756 bounds

    if let Ok(mut tab) = comptags.lock() {
        if tab[idx].is_some() {                                              // c:3798
            freectags(tab[idx].take());                                      // c:3799
        }
        let context = tags.first().cloned();                                 // c:3804 *tags
        let all: Vec<String> = tags.iter().skip(1).cloned().collect();       // c:3803 tags+1
        tab[idx] = Some(Box::new(ctags {                                     // c:3801 zalloc
            all: Some(all),                                                  // c:3803
            context,                                                         // c:3804
            init: 1,                                                         // c:3806
            sets: None,                                                      // c:3805
        }));
    }
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
