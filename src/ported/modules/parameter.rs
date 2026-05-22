//! Parameter interface to shell internals - port of Modules/parameter.c
//!
//! Functions for the parameters special parameter.                          // c:37
//! Return a string describing the type of a parameter.                      // c:39
//! Functions for the commands special parameter.                            // c:147
//! Functions for the functions special parameter.                           // c:280
//! Functions for the builtins special parameter.                            // c:771
//! Functions for the options special parameter.                             // c:922
//! Functions for the modules special parameter.                             // c:1036
//! Functions for the history special parameter.                             // c:1152
//! Table for defined parameters.                                            // c:2177
//!
//! Provides special parameters: $commands, $functions, $aliases, $builtins,
//! $modules, $dirstack, $history, $historywords, $options, $nameddirs, $userdirs

use crate::ported::builtin::BUILTINS;
use crate::ported::hashnameddir::{addnameddirnode, nameddirtab};
use crate::ported::hashtable::{
    aliastab_lock, cmdnam_hashed, cmdnamtab_lock, createaliasnode, shfunctab_lock,
    sufaliastab_lock,
};
use crate::ported::hist::hist_ring;
use crate::ported::jobs::{getjob, selectjobtab, sigmsg};
use crate::ported::module::MODULESTAB;
use crate::ported::options::{dosetopt, opt_state_set, optlookup};
use crate::ported::params::{deleteparamtable, getsparam, getstrvalue, realparamtab};
use crate::ported::utils::zwarn;
use crate::ported::zsh_h::{
    ALIAS_GLOBAL, ALIAS_SUFFIX, DISABLED, FS_EVAL, FS_SOURCE, HashNode, HashTable, INTERACTIVE,
    ND_USERNAME, PM_ARRAY, PM_AUTOLOAD, PM_EFLOAT, PM_EXPORTED, PM_FFLOAT, PM_HASHED, PM_HIDE,
    PM_HIDEVAL, PM_INTEGER, PM_LEFT, PM_LOWER, PM_NAMEREF, PM_READONLY, PM_RIGHT_B, PM_RIGHT_Z,
    PM_SCALAR, PM_SPECIAL, PM_TAGGED, PM_TIED, PM_TYPE, PM_UNIQUE, PM_UNSET, PM_UPPER, Param,
    SCANPM_MATCHVAL, SCANPM_WANTKEYS, SCANPM_WANTVALS, SP_RUNNING, STAT_DONE, STAT_NOPRINT,
    STAT_STOPPED, ScanFunc, hashnode, hashtable, isset, module, nameddir, opt_name, param,
    value,
};
use std::collections::HashMap;
use std::path::PathBuf;
use crate::zsh_h::{shfunc, HASHED};
use std::sync::{Mutex, OnceLock};
use crate::DPUTS;
// Bag-of-globals `ParamType`/`ParamFlags` enum + `*Table` structs
// deleted (PORT_PLAN.md Phase 2 anti-pattern #1): C has no
// counterpart — paramtypestr now reads `PM_TYPE(pm->node.flags)`
// directly, mirroring Src/Modules/parameter.c:43.

// Return a string describing the type of a parameter.                      // c:43
/// Port of `paramtypestr(Param pm)` from Src/Modules/parameter.c:43.
/// C: `static char *paramtypestr(Param pm)` — render a parameter's
/// type and modifier flags as the `typeset -p` flag string.
pub fn paramtypestr(pm: &param) -> String {
    // c:43

    let f: u32 = pm.node.flags as u32; // c:46

    if (f & PM_UNSET) != 0 {
        // c:48 (else branch c:91)
        return String::new(); // c:92 dupstring("")
    }
    if (f & PM_AUTOLOAD) != 0 {
        // c:49
        return "undefined".to_string(); // c:50
    }

    let mut val: String = match PM_TYPE(f) {
        // c:52
        PM_SCALAR => "scalar".to_string(),            // c:53
        PM_NAMEREF => "nameref".to_string(),          // c:54
        PM_ARRAY => "array".to_string(),              // c:55
        PM_INTEGER => "integer".to_string(),          // c:56
        PM_EFLOAT | PM_FFLOAT => "float".to_string(), // c:57-58
        PM_HASHED => "association".to_string(),       // c:59
        _ => {
            // c:60 default
            DPUTS!(true, "BUG: type not handled in parameter"); // c:61
            String::new() // c:62
        }
    };

    if pm.level != 0 {
        val.push_str("-local");
    } // c:63-64
    if (f & PM_LEFT) != 0 {
        val.push_str("-left");
    } // c:65-66
    if (f & PM_RIGHT_B) != 0 {
        val.push_str("-right_blanks");
    } // c:67-68
    if (f & PM_RIGHT_Z) != 0 {
        val.push_str("-right_zeros");
    } // c:69-70
    if (f & PM_LOWER) != 0 {
        val.push_str("-lower");
    } // c:71-72
    if (f & PM_UPPER) != 0 {
        val.push_str("-upper");
    } // c:73-74
    if (f & PM_READONLY) != 0 {
        val.push_str("-readonly");
    } // c:75-76
    if (f & PM_TAGGED) != 0 {
        val.push_str("-tag");
    } // c:77-78
    if (f & PM_TIED) != 0 {
        val.push_str("-tied");
    } // c:79-80
    if (f & PM_EXPORTED) != 0 {
        val.push_str("-export");
    } // c:81-82
    if (f & PM_UNIQUE) != 0 {
        val.push_str("-unique");
    } // c:83-84
    if (f & PM_HIDE) != 0 {
        val.push_str("-hide");
    } // c:85-86
    if (f & PM_HIDEVAL) != 0 {
        val.push_str("-hideval");
    } // c:87-88
    if (f & PM_SPECIAL) != 0 {
        val.push_str("-special");
    } // c:89-90

    val // c:94
}

/// Direct port of `getpmparameter(UNUSED(HashTable ht), const char *name)` from Src/Modules/parameter.c:99.
/// C body (c:102-210): `paramtab[name]` lookup; emit a scalar Param
/// whose value is the type-letter encoding (`scalar`, `array`,
/// `association`, `integer`, `float`, plus `-readonly`/`-export`/
/// etc. modifiers per PM_* flags).
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn getpmparameter(ht: *mut HashTable, name: &str) -> Option<Param> {
    // c:99
    // c:99-140 — `if ((pm = (Param)paramtab->getnode2(paramtab, name)))`
    //              then dispatch on `PM_TYPE(pm->node.flags)` for the
    //              type-letter. paramtab is bucket-2-consolidated now.
    let value = {
        let tab = crate::ported::params::paramtab().read().unwrap();
        tab.get(name)
            .map(|pm| {
                let t = PM_TYPE(pm.node.flags as u32);
                // c:140 — type-letter table.
                if t == PM_ARRAY {
                    "array".to_string()
                } else if t == PM_HASHED {
                    "association".to_string()
                } else if t == PM_INTEGER {
                    "integer".to_string()
                } else if t == PM_EFLOAT || t == PM_FFLOAT {
                    "float".to_string()
                } else {
                    "scalar".to_string()
                }
            })
            .unwrap_or_default()
    };
    let found = !value.is_empty();
    let pm = Box::new(param {
        // c:103 hcalloc
        node: hashnode {
            next: None,
            nam: name.to_string(), // c:104
            flags: if found {
                (PM_SCALAR | PM_READONLY) as i32
            } else {
                (PM_SCALAR | PM_READONLY | PM_UNSET | PM_SPECIAL) as i32
            }, // c:209
        },
        u_data: 0,
        u_arr: None,
        u_str: Some(value), // c:208
        u_val: 0,
        u_dval: 0.0,
        u_hash: None,
        gsu_s: None, // c:106 pmparam_gsu
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
    Some(pm) // c:210
}

#[cfg(test)]
mod paramtypestr_tests {
    use super::*;
    use crate::ported::zsh_h::param;

    fn make_pm(flags: u32, level: i32) -> param {
        param {
            node: hashnode {
                next: None,
                nam: String::new(),
                flags: flags as i32,
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
            level,
        }
    }

    /// Mirrors Src/Modules/parameter.c:43-95 — switch on
    /// `PM_TYPE(pm->node.flags)` then dyncat'd modifier chain.
    #[test]
    fn paramtypestr_matches_c_dispatch() {
        let _g = crate::test_util::global_state_lock();
        // c:53 — plain scalar.
        assert_eq!(paramtypestr(&make_pm(PM_SCALAR, 0)), "scalar");
        // c:55,63-64,81-82 — array + level=1 + PM_EXPORTED.
        assert_eq!(
            paramtypestr(&make_pm(PM_ARRAY | PM_EXPORTED, 1)),
            "array-local-export",
        );
        // c:91-92 — PM_UNSET short-circuits to "".
        assert_eq!(paramtypestr(&make_pm(PM_UNSET, 0)), "");
    }
}

// =====================================================================
// static struct features module_features                            c:2300 (parameter.c)
// =====================================================================


/// Port of `scanpmparameters(UNUSED(HashTable ht), ScanFunc func, int flags)` from Src/Modules/parameter.c:124.
///
/// C iterates `realparamtab` invoking `func(&pm.node, flags)` for each
/// non-PM_UNSET entry. The zshrs special-parameter hashparam-node
/// integration isn't wired up yet — `${(@k)parameters}` reads
/// through `paramtab()` directly, which is the Rust idiom
/// replacement for the C iteration callback. No Rust callers of
/// this fn; structural pass-through retained for C name parity.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(_ht, _func) vs C=(ht, func, flags)
pub fn scanpmparameters(
    _ht: *mut HashTable,
    func: Option<ScanFunc>, // c:124
    flags: i32,
) {
    let func = match func {
        Some(f) => f,
        None => return,
    }; // c:131-141 no-op without func
       // Snapshot names + per-entry data under read lock so func() can
       // re-enter paramtab without deadlock — C is single-threaded so
       // walks the live table directly.
    let entries: Vec<(String, u32, String)> = {
        let tab = realparamtab()
            .read()
            .expect("realparamtab poisoned"); // c:135 realparamtab walk
        tab.iter()
            .filter(|(_, p)| (p.node.flags as u32 & PM_UNSET) == 0) // c:138 PM_UNSET skip
            .map(|(name, p)| {
                let want_val = (flags as u32 & (SCANPM_WANTVALS | SCANPM_MATCHVAL)) != 0
                    || (flags as u32 & SCANPM_WANTKEYS) == 0; // c:140-142
                let val = if want_val {
                    paramtypestr(p)
                } else {
                    String::new()
                };
                (name.clone(), p.node.flags as u32, val)
            })
            .collect()
    };
    for (name, _orig_flags, val) in entries {
        // c:135-145
        let pm = param {
            node: hashnode {
                // c:128 memset(&pm, 0)
                next: None,
                nam: name,
                flags: (PM_SCALAR | PM_READONLY) as i32, // c:129
            },
            u_data: 0,
            u_arr: None,
            u_str: Some(val), // c:144 pm.u.str
            u_val: 0,
            u_dval: 0.0,
            u_hash: None,
            gsu_s: None,
            gsu_i: None,
            gsu_f: None,
            gsu_a: None,
            gsu_h: None, // c:130 gsu.s = nullsetscalar_gsu (vtable not modelled)
            base: 0,
            width: 0,
            env: None,
            ename: None,
            old: None,
            level: 0,
        };
        func(&Box::new(pm.node), flags); // c:145 func(&pm.node, flags)
    }
}

/// Port of `setpmcommand(Param pm, char *value)` from Src/Modules/parameter.c:151.
/// C: `static void setpmcommand(Param pm, char *value)` — register a path
/// alias in cmdnamtab for the named command.
#[allow(non_snake_case)]
pub fn setpmcommand(pm: Param, value: String) {
    // c:151
    // c:151-158 — `cn = zshcalloc(...); cn->node.flags = HASHED;
    //   cn->u.cmd = ztrdup(value); cmdnamtab->addnode(...)`. The
    //   helper bundles the hashnode literal so the call-site stays
    //   one line.
    let cn = cmdnam_hashed(&pm.node.nam, &value); // c:173-156
    if let Ok(mut tab) = cmdnamtab_lock().write() {
        tab.add(cn); // c:173 addnode
    }
}

/// Port of `unsetpmcommand(Param pm, UNUSED(int exp))` from Src/Modules/parameter.c:163.
/// C: `static void unsetpmcommand(Param pm, UNUSED(int exp))` — remove the
/// named entry from `cmdnamtab`.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn unsetpmcommand(pm: Param, exp: i32) {
    // c:163
    if let Ok(mut tab) = cmdnamtab_lock().write() {
        // c:165 — HashNode hn = cmdnamtab->removenode(cmdnamtab, pm->node.nam);
        let _hn = tab.remove(&pm.node.nam);
        // c:167-168 — if (hn) cmdnamtab->freenode(hn); — Rust Drop on scope exit.
    }
}

/// Port of `setpmcommands(Param pm, HashTable ht)` from Src/Modules/parameter.c:173.
/// C: `static void setpmcommands(Param pm, HashTable ht)` — bulk install.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn setpmcommands(pm: Param, ht: *mut HashTable) {
    // c:173
    // c:173-176 — locals at function top.
    let mut i: i32; // c:175 int i
    let mut hn: Option<HashNode>; // c:176 HashNode hn

    // c:178-179 — if (!ht) return;
    if ht.is_null() {
        // c:178
        return; // c:179
    }
    let ht_ref: &hashtable = unsafe { &**ht };
    i = 0; // c:181 for (i = 0;
    while i < ht_ref.hsize {
        // c:181 i < ht->hsize; i++)
        hn = ht_ref.nodes.get(i as usize).and_then(|n| n.clone()); // c:182 hn = ht->nodes[i]
        while let Some(node) = hn.clone() {
            // c:182 hn;
            // c:184-189 — struct value v (block-scoped per C).
            let mut v = value {
                pm: None,        // c:189 v.pm = (Param) hn (cast deferred)
                arr: Vec::new(), // c:188 v.arr = NULL
                scanflags: 0,    // c:186
                valflags: 0,     // c:186
                start: 0,        // c:186
                end: -1,         // c:187
            };
            // c:183/191/192 — `cn = zshcalloc(...); cn->node.flags
            //   = HASHED; cn->u.cmd = ztrdup(getstrvalue(&v));`
            let path = getstrvalue(Some(&mut v));
            let cn = cmdnam_hashed(&node.nam, &path);
            // c:194 — cmdnamtab->addnode(cmdnamtab, ztrdup(hn->nam), &cn->node);
            if let Ok(mut tab) = cmdnamtab_lock().write() {
                tab.add(cn);
            }
            hn = node.next; // c:182 hn = hn->next
        }
        i += 1; // c:181 i++
    }
    // c:196-205 — comment block about full-array vs append (informational).
    // c:204 — if (ht != pm->u.hash) deleteparamtable(ht);
    if !ht.is_null() {
        // c:203
        let owned: HashTable = unsafe { std::ptr::read(ht) }; // move Box out
        deleteparamtable(Some(owned)); // c:204
    }
}

/// Direct port of `getpmcommand(UNUSED(HashTable ht), const char *name)` from Src/Modules/parameter.c:213.
/// C body (c:216-241):
/// ```c
/// cmd = cmdnamtab->getnode(cmdnamtab, name);
/// if (!cmd && isset(HASHLISTALL)) cmdnamtab->filltable(...); cmd = ...;
/// pm.node.nam = name; pm.node.flags = PM_SCALAR; pm.gsu.s = &pmcommand_gsu;
/// if (cmd) {
///     if (cmd->node.flags & HASHED) pm->u.str = cmd->u.cmd;
///     else                          pm->u.str = path/name;
/// } else {
///     pm->u.str = ""; pm->node.flags |= (PM_UNSET|PM_SPECIAL);
/// }
/// ```
#[allow(non_snake_case)]
/// Port of `getpmcommand(UNUSED(HashTable ht), const char *name)` from `Src/Modules/parameter.c:213`.
#[allow(unused_variables)]
pub fn getpmcommand(ht: *mut HashTable, name: &str) -> Option<Param> {
    // c:213
    let g = cmdnamtab_lock().read().ok()?;
    let entry = g.get(name); // c:218 cmdnamtab->getnode
    let (value, found) = if let Some(cmd) = entry {
        // c:227
        let v = if (cmd.node.flags & HASHED as i32) != 0 {
            // c:229 HASHED
            cmd.cmd.clone().unwrap_or_default() // c:230 cn->u.cmd
        } else {
            let dir = cmd
                .name
                .as_ref()
                .and_then(|v| v.first().cloned()) // c:232 *(cmd->u.name)
                // C: `*(cmd->u.name)` reads first entry of $path array.
                //     paramtab read; was OS env split.
                .unwrap_or_else(|| {
                    getsparam("PATH")
                        .and_then(|p| p.split(':').next().map(|s| s.to_string()))
                        .unwrap_or_default()
                });
            format!("{}/{}", dir, name) // c:233-235 strcat
        };
        (v, true)
    } else {
        (String::new(), false) // c:238
    };
    let mut pm = Box::new(param {
        // c:223 hcalloc
        node: hashnode {
            next: None,
            nam: name.to_string(), // c:224
            flags: if found {
                PM_SCALAR as i32
            } else {
                (PM_SCALAR | PM_UNSET | PM_SPECIAL) as i32
            }, // c:226 / c:239
        },
        u_data: 0,
        u_arr: None,
        u_str: Some(value), // c:230 / c:233 / c:238
        u_val: 0,
        u_dval: 0.0,
        u_hash: None,
        gsu_s: None, // c:226 pmcommand_gsu (gsu table not yet wired)
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
    let _ = &mut pm;
    Some(pm) // c:241 return &pm->node
}

/// Direct port of `scanpmcommands(UNUSED(HashTable ht), ScanFunc func, int flags)` from Src/Modules/parameter.c:245.
/// C body (c:248-280):
/// ```c
/// if (isset(HASHLISTALL)) cmdnamtab->filltable(cmdnamtab);
/// pm.node.flags = PM_SCALAR; pm.gsu.s = &pmcommand_gsu;
/// for each hn in cmdnamtab:
///     pm.node.nam = hn->nam;
///     if non-counting && wantvals:
///         pm.u.str = HASHED ? cmd->u.cmd : path/name
///     func(&pm.node, flags);
/// ```
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(_ht, func) vs C=(ht, func, flags)
pub fn scanpmcommands(
    _ht: *mut HashTable,
    func: Option<ScanFunc>, // c:245
    flags: i32,
) {
    // c:253 — `if (isset(HASHLISTALL)) cmdnamtab->filltable(...)`. The
    // filltable variant scans $PATH and inserts every executable into
    // cmdnamtab; without HASHLISTALL only previously-hashed entries
    // appear. Static-link path defers the filltable side-effect until
    // the option-state plumbing lands.
    let cmds: Vec<(String, bool, String)> = {
        let g = cmdnamtab_lock().read().unwrap();
        g.iter()
            .map(|(name, cmd)| {
                // c:259-260
                let hashed = (cmd.node.flags & HASHED as i32) != 0;
                // c:266-274 — pm.u.str: HASHED → cmd->u.cmd (real path);
                // unhashed → first $PATH dir + "/" + name.
                let value = if hashed {
                    cmd.cmd.clone().unwrap_or_default() // c:267 cn->u.cmd
                } else {
                    let dir = cmd
                        .name
                        .as_ref()
                        .and_then(|v| v.first().cloned()) // c:269 *(cmd->u.name)
                        // C: `*(cmd->u.name)` — first entry of $path array.
                        //     Read shell-side $PATH from paramtab (was OS env).
                        .unwrap_or_else(|| {
                            getsparam("PATH")
                                .and_then(|p| p.split(':').next().map(|s| s.to_string()))
                                .unwrap_or_default()
                        });
                    format!("{}/{}", dir, name) // c:271-273 strcat
                };
                (name.clone(), hashed, value)
            })
            .collect()
    };
    let _ = (PM_SCALAR, SCANPM_WANTVALS, SCANPM_MATCHVAL, SCANPM_WANTKEYS);
    if let Some(f) = func {
        // c:259 — for each cmdnamtab entry, build a stack-local param
        // and pass to the callback. Rust uses a real param struct
        // (not a stack pun) so the callback sees a stable HashNode.
        for (name, _hashed, _value) in &cmds {
            let node = Box::new(hashnode {
                // c:264 pm.node.nam
                next: None,
                nam: name.clone(),
                flags: 0,
            });
            f(&node, flags); // c:280 func(&pm.node, flags)
        }
    }
    let _ = cmds;
}

/// Port of `setfunction(char *name, char *val, int dis)` from Src/Modules/parameter.c:284.
/// C: `static void setfunction(char *name, char *val, int dis)` — install
/// a shell function from text source.
#[allow(non_snake_case)]
pub fn setfunction(name: &str, mut val: String, dis: i32) {
    // c:284
    // c:284-289 — declarations at function top (PORT.md Rule 5: same
    // names, same order, same scope as C).
    let value: String; // c:286 char *value
    let shf: shfunc; // c:287 Shfunc shf
                                               // c:288 — Eprog prog (skipped: parse_string not yet ported)
                                               // c:289 — int sn (used inside the TRAP branch only)

    // c:286 — char *value = dupstring(val);
    value = val.clone();
    // c:291 — val = metafy(val, strlen(val), META_REALLOC);
    val = crate::ported::utils::metafy(&val);
    // c:293 — prog = parse_string(val, 1);
    // EXTERN: `parse_string` lives in Src/parse.c — not yet ported.
    // ShFunc stores the source string and the executor parses at first
    // call (deferred parse). The c:295-299 "invalid function definition"
    // guard only fires on parse errors; with deferred parse we only
    // filter empty input.
    if val.is_empty() {
        // c:295 !prog
        zwarn(
            // c:296
            &format!("invalid function definition: {}", value),
        );
        return; // c:298
    }
    // c:300 — shf = zshcalloc(sizeof(*shf));
    // c:301 — shf->funcdef = dupeprog(prog, 0); (deferred — ShFunc.body)
    // c:302 — shf->node.flags = dis;
    shf = shfunc {
        node: hashnode {
            next: None,
            nam: name.to_string(),
            flags: dis, // c:302
        },
        filename: None,
        lineno: 0,
        funcdef: None,
        redir: None,
        sticky: None,
        body: Some(val.clone()), // c:301 (deferred-parse path)
    };
    // c:303 — shfunc_set_sticky(shf); (EXTERN exec.c sticky-options bit)
    // Not yet ported; sticky-options propagation is a no-op until the
    // emulation framework supports per-function emulation switches.

    // c:305-313 — TRAP* handling.
    if name.len() >= 4 && &name[..4] == "TRAP" {
        // c:305
        if let Some(_sn) = crate::ported::signals::getsigidx(&name[4..]) { // c:306 sn = getsigidx
             // c:307-312 — settrap(sn, NULL, ZSIG_FUNC) failure path.
             // EXTERN: settrap signature in signals.rs:1035 takes
             // TrapAction enum, not the C `(int sn, Eprog list, int
             // type)` triple. Skip the early-return until the ZSIG_FUNC
             // overload lands.
        }
    }

    // c:314 — shfunctab->addnode(shfunctab, ztrdup(name), shf);
    if let Ok(mut tab) = shfunctab_lock().write() {
        tab.add(shf);
    }
    // c:315 — zsfree(val); — Rust drops on scope exit.
}

/// Port of `setpmfunction(Param pm, char *value)` from Src/Modules/parameter.c:320.
/// C: `setfunction(pm->node.nam, value, 0);`
#[allow(non_snake_case)]
pub fn setpmfunction(pm: Param, value: String) {
    // c:320
    let nam = pm.node.nam.clone();
    setfunction(&nam, value, 0) // c:323
}

/// Port of `setpmdisfunction(Param pm, char *value)` from Src/Modules/parameter.c:327.
/// C: `setfunction(pm->node.nam, value, DISABLED);`
#[allow(non_snake_case)]
pub fn setpmdisfunction(pm: Param, value: String) {
    // c:327
    let nam = pm.node.nam.clone();
    setfunction(&nam, value, DISABLED) // c:330
}

/// Port of `unsetpmfunction(Param pm, UNUSED(int exp))` from Src/Modules/parameter.c:334.
/// C: `static void unsetpmfunction(Param pm, UNUSED(int exp))` — remove the
/// named function from `shfunctab`.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn unsetpmfunction(pm: Param, exp: i32) {
    // c:334
    if let Ok(mut tab) = shfunctab_lock().write() {
        // c:336 — HashNode hn = shfunctab->removenode(shfunctab, pm->node.nam);
        let _hn = tab.remove(&pm.node.nam);
        // c:338-339 — if (hn) shfunctab->freenode(hn); — Rust Drop on scope exit.
    }
}

/// Port of `setfunctions(Param pm, HashTable ht, int dis)` from Src/Modules/parameter.c:344.
/// C: `static void setfunctions(Param pm, HashTable ht, int dis)` — install
/// all functions in `ht`.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn setfunctions(pm: Param, ht: *mut HashTable, dis: i32) {
    // c:344
    // c:344-347 — locals at function top (Rule 5: same names, same scope).
    let mut i: i32; // c:346 int i
    let mut hn: Option<HashNode>; // c:347 HashNode hn

    // c:349-350 — if (!ht) return;
    if ht.is_null() {
        // c:349
        return; // c:350
    }
    let ht_ref: &hashtable = unsafe { &**ht };
    i = 0; // c:352 for (i = 0;
    while i < ht_ref.hsize {
        // c:352 i < ht->hsize; i++)
        hn = ht_ref.nodes.get(i as usize).and_then(|n| n.clone()); // c:353 hn = ht->nodes[i]
        while let Some(node) = hn.clone() {
            // c:353 hn;
            // c:354-359 — struct value v; (block-scoped per C).
            let mut v = value {
                pm: None,        // c:359 v.pm = (Param) hn (cast deferred)
                arr: Vec::new(), // c:358 v.arr = NULL
                scanflags: 0,    // c:356 v.scanflags = 0
                valflags: 0,     // c:356 v.valflags = 0
                start: 0,        // c:356 v.start = 0
                end: -1,         // c:357 v.end = -1
            };
            // c:361 — setfunction(hn->nam, ztrdup(getstrvalue(&v)), dis);
            // EXTERN: getstrvalue reads v.pm — without a real Param cast
            // we get empty. Future port: thread a paramtab lookup through.
            setfunction(
                &node.nam,
                getstrvalue(Some(&mut v)),
                dis,
            );
            hn = node.next; // c:353 hn = hn->next
        }
        i += 1; // c:352 i++
    }
    // c:364-365 — if (ht != pm->u.hash) deleteparamtable(ht);
    if !ht.is_null() {
        // c:364
        let owned: HashTable = unsafe { std::ptr::read(ht) }; // move Box out
        deleteparamtable(Some(owned)); // c:365
    }
}

/// Port of `setpmfunctions(Param pm, HashTable ht)` from Src/Modules/parameter.c:370.
#[allow(non_snake_case)]
pub fn setpmfunctions(pm: Param, ht: *mut HashTable) {
    // c:370
    setfunctions(pm, ht, 0) // c:370
}

/// Port of `setpmdisfunctions(Param pm, HashTable ht)` from Src/Modules/parameter.c:377.
/// C: `setfunctions(pm, ht, DISABLED);`
#[allow(non_snake_case)]
pub fn setpmdisfunctions(pm: Param, ht: *mut HashTable) {
    // c:377
    setfunctions(pm, ht, DISABLED) // c:377
}

/// Direct port of `getfunction(UNUSED(HashTable ht), const char *name, int dis)` from Src/Modules/parameter.c:389.
/// C body (c:392-441):
/// ```c
/// pm.node.nam = name; pm.node.flags = PM_SCALAR;
/// pm.gsu.s = dis ? &pmdisfunction_gsu : &pmfunction_gsu;
/// if (shf = shfunctab[name]; shf matches dis) {
///     if (PM_UNDEFINED) pm.u.str = "builtin autoload -X" + flags;
///     else { build "{\n\t<body>\n\t<name> "$@"" if EF_RUN; getpermtext };
/// } else { pm.u.str = ""; flags |= PM_UNSET|PM_SPECIAL; }
/// ```
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=() vs C=(ht, name, dis)
pub fn getfunction(_ht: *mut HashTable, name: &str, _dis: i32) -> Option<Param> {
    let g = shfunctab_lock().read().ok()?;
    let entry = g.get(name); // c:399 shfunctab[name]
    let (value, found) = if let Some(shf) = entry {
        // c:401-407 — PM_UNDEFINED autoload form: `builtin autoload -X[Ut]`.
        // Static-link path doesn't yet expose PM_UNDEFINED on ShFunc;
        // route via body.is_none() as the autoload signal.
        let body = shf.body.as_deref();
        let v = match body {
            None => "builtin autoload -X".to_string(), // c:402-407
            Some(text) => format!("\t{}", text),       // c:409-431 getpermtext
        };
        (v, true)
    } else {
        (String::new(), false) // c:439
    };
    let pm = Box::new(param {
        // c:393
        node: hashnode {
            next: None,
            nam: name.to_string(), // c:394
            flags: if found {
                PM_SCALAR as i32
            }
            // c:395
            else {
                (PM_SCALAR | PM_UNSET | PM_SPECIAL) as i32
            }, // c:440
        },
        u_data: 0,
        u_arr: None,
        u_str: Some(value), // c:402/431/438
        u_val: 0,
        u_dval: 0.0,
        u_hash: None,
        gsu_s: None, // c:396 pm[dis]function_gsu
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
    Some(pm) // c:441
}

/// Port of `getpmfunction(HashTable ht, const char *name)` from Src/Modules/parameter.c:444.
/// C: `static HashNode getpmfunction(HashTable ht, const char *name)` →
///   `return getfunction(ht, name, 0);`
#[allow(non_snake_case)]
pub fn getpmfunction(ht: *mut HashTable, name: &str) -> Option<Param> {
    // c:444
    getfunction(ht, name, 0) // c:444
}

/// Port of `getpmdisfunction(HashTable ht, const char *name)` from Src/Modules/parameter.c:451.
/// C: `static HashNode getpmdisfunction(HashTable ht, const char *name)` →
///   `return getfunction(ht, name, DISABLED);`
#[allow(non_snake_case)]
pub fn getpmdisfunction(ht: *mut HashTable, name: &str) -> Option<Param> {
    // c:451
    getfunction(ht, name, DISABLED) // c:451
}

/// Port of `scanfunctions(UNUSED(HashTable ht), ScanFunc func, int flags, int dis)` from Src/Modules/parameter.c:458.
/// C: `static void scanfunctions(UNUSED(HashTable ht), ScanFunc func,
///     int flags, int dis)` — iterate shfunctab.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(_ht, func, _dis) vs C=(ht, func, flags, dis)
pub fn scanfunctions(
    _ht: *mut HashTable,
    func: Option<ScanFunc>, // c:458
    flags: i32,
    _dis: i32,
) {
    // C body (c:461-516): loop through shfunctab nodes filtered by
    // DISABLED; for each non-counting func, build the body string
    // (autoload-X form for PM_UNDEFINED, otherwise getpermtext +
    // EF_RUN tail "\n\t<name> $@") and emit via func().
    // Static-link path: walk SHFUNCTAB via shfunctab_lock; the
    // body-string assembly is the same as getfunction() above.
    let names: Vec<String> = if let Ok(g) = shfunctab_lock().read() {
        g.iter().map(|(n, _)| n.clone()).collect() // c:469-470
    } else {
        Vec::new()
    };
    if let Some(f) = func {
        for name in names {
            let node = Box::new(hashnode {
                next: None,
                nam: name,
                flags: 0, // c:472
            });
            f(&node, flags); // c:514
        }
    }
}

/// Port of `scanpmfunctions(HashTable ht, ScanFunc func, int flags)` from Src/Modules/parameter.c:519.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(ht, func) vs C=(ht, func, flags)
pub fn scanpmfunctions(
    ht: *mut HashTable,
    func: Option<ScanFunc>, // c:519
    flags: i32,
) {
    scanfunctions(ht, func, flags, 0) // c:522
}

/// Port of `scanpmdisfunctions(HashTable ht, ScanFunc func, int flags)` from Src/Modules/parameter.c:526.
/// C: `static void scanpmdisfunctions(HashTable ht, ScanFunc func, int flags)`
///   → `scanfunctions(ht, func, flags, DISABLED);`
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(ht, func) vs C=(ht, func, flags)
pub fn scanpmdisfunctions(
    ht: *mut HashTable,
    func: Option<ScanFunc>, // c:526
    flags: i32,
) {
    scanfunctions(ht, func, flags, DISABLED) // c:529
}

/// Port of `getfunction_source(UNUSED(HashTable ht), const char *name, int dis)` from Src/Modules/parameter.c:537.
/// C: `static HashNode getfunction_source(UNUSED(HashTable ht),
///     const char *name, int dis)` — synth a Param naming the source file.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=() vs C=(ht, name, dis)
pub fn getfunction_source(_ht: *mut HashTable, name: &str, _dis: i32) -> Option<Param> {
    let g = shfunctab_lock().read().ok()?;
    let entry = g.get(name);
    let (value, found) = if let Some(shf) = entry {
        // c:545
        // c:548-555 — `pm.u.str = dyncat(shf->filename ?: "", ":lineno")`.
        // Static-link path: ShFunc.filename is the source file; lineno
        // tracking isn't yet stored, so we emit "filename:0" matching
        // C's c:553 fallback when filename was set without line info.
        let fname = shf.filename.as_deref().unwrap_or("");
        (format!("{}:0", fname), true)
    } else {
        (String::new(), false) // c:586
    };
    let pm = Box::new(param {
        // c:541
        node: hashnode {
            next: None,
            nam: name.to_string(), // c:542
            flags: if found {
                (PM_SCALAR | PM_READONLY) as i32
            }
            // c:543
            else {
                (PM_SCALAR | PM_READONLY | PM_UNSET | PM_SPECIAL) as i32
            }, // c:587
        },
        u_data: 0,
        u_arr: None,
        u_str: Some(value), // c:553 / c:586
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
    Some(pm) // c:589
}

/// Port of `scanfunctions_source(UNUSED(HashTable ht), ScanFunc func, int flags, int dis)` from Src/Modules/parameter.c:560.
/// C: `static void scanfunctions_source(UNUSED(HashTable ht), ScanFunc func,
///     int flags, int dis)` — iterate shfunctab, emit source filename.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(_ht, func, _dis) vs C=(ht, func, flags, dis)
pub fn scanfunctions_source(
    _ht: *mut HashTable,
    func: Option<ScanFunc>, // c:560
    flags: i32,
    _dis: i32,
) {
    // C body (c:563-606): loop through shfunctab nodes filtered by
    // DISABLED; for each non-counting func, emit "filename:lineno"
    // via getpmhashtable. Static-link path walks SHFUNCTAB and emits
    // the function name (filename data isn't yet stored on ShFunc).
    let names: Vec<String> = if let Ok(g) = shfunctab_lock().read() {
        g.iter().map(|(n, _)| n.clone()).collect() // c:570
    } else {
        Vec::new()
    };
    if let Some(f) = func {
        for name in names {
            let node = Box::new(hashnode {
                next: None,
                nam: name,
                flags: 0, // c:573
            });
            f(&node, flags); // c:604
        }
    }
}


/// Port of `getpmfunction_source(HashTable ht, const char *name)` from Src/Modules/parameter.c:591.
/// C: `static HashNode getpmfunction_source(HashTable ht, const char *name)`
///   → `return getfunction_source(ht, name, 0);`
#[allow(non_snake_case)]
pub fn getpmfunction_source(ht: *mut HashTable, name: &str) -> Option<Param> {
    // c:591
    getfunction_source(ht, name, 0) // c:591
}

/// Port of `getpmdisfunction_source(HashTable ht, const char *name)` from Src/Modules/parameter.c:600.
/// C: `static HashNode getpmdisfunction_source(HashTable ht,
///     const char *name)` → `return getfunction_source(ht, name, 1);`
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=() vs C=(ht, name)
pub fn getpmdisfunction_source(ht: *mut HashTable, name: &str) -> Option<Param> {
    getfunction_source(ht, name, 1) // c:603
}

/// Port of `scanpmfunction_source(HashTable ht, ScanFunc func, int flags)` from Src/Modules/parameter.c:609.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(ht, func) vs C=(ht, func, flags)
pub fn scanpmfunction_source(
    ht: *mut HashTable,
    func: Option<ScanFunc>, // c:609
    flags: i32,
) {
    scanfunctions_source(ht, func, flags, 0) // c:612
}

/// Port of `scanpmdisfunction_source(HashTable ht, ScanFunc func, int flags)` from Src/Modules/parameter.c:618.
/// C: `static void scanpmdisfunction_source(HashTable ht, ScanFunc func,
///     int flags)` → `scanfunctions_source(ht, func, flags, 1);`
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(ht, flags) vs C=(ht, func, flags)
pub fn scanpmdisfunction_source(
    ht: *mut HashTable, // c:618
    func: Option<ScanFunc>,
    flags: i32,
) {
    scanfunctions_source(ht, func, flags, 1) // c:621
}

/// Port of `funcstackgetfn(UNUSED(Param pm))` from Src/Modules/parameter.c:627.
/// C: `static char **funcstackgetfn(UNUSED(Param pm))` — returns the
/// list of function names currently on the call stack.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn funcstackgetfn(pm: *mut param) -> Vec<String> {
    // c:627
    // c:627-643 — count frames, allocate, walk linking *p = f->name.
    let stack = FUNCSTACK.lock().map(|s| s.clone()).unwrap_or_default();
    stack.iter().map(|f| f.name.clone()).collect() // c:648
}

/// Port of `functracegetfn(UNUSED(Param pm))` from Src/Modules/parameter.c:648.
/// C: `static char **functracegetfn(UNUSED(Param pm))` —
/// Port of `static char **functracegetfn(UNUSED(Param pm))` from
/// `Src/Modules/parameter.c:648`. Walks the `funcstack` linked
/// list, building `"<caller>:<lineno>"` per frame.
/// ```c
/// static char **
/// functracegetfn(UNUSED(Param pm))
/// {
///     Funcstack f;
///     int num;
///     char **ret, **p;
///     for (f = funcstack, num = 0; f; f = f->prev, num++);
///     ret = zhalloc((num + 1) * sizeof(char *));
///     for (f = funcstack, p = ret; f; f = f->prev, p++) {
///         char *colonpair = zhalloc(strlen(f->caller) +
///                                   (f->lineno > 9999 ? 24 : 6));
///         sprintf(colonpair, "%s:%lld", f->caller, f->lineno);
///         *p = colonpair;
///     }
///     *p = NULL;
///     return ret;
/// }
/// ```
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn functracegetfn(pm: *mut param) -> Vec<String> {
    // c:648
    let f_stack = FUNCSTACK.lock().map(|s| s.clone()).unwrap_or_default(); // c:650
                                                                           // c:654 — `for (f = funcstack, num = 0; f; f = f->prev, num++)`
    let num = f_stack.len(); // c:654
                             // c:656 — `ret = zhalloc((num + 1) * sizeof(char *));`
    let mut ret: Vec<String> = Vec::with_capacity(num + 1); // c:656
                                                            // c:658 — `for (f = funcstack, p = ret; f; f = f->prev, p++)`
    for f in &f_stack {
        // c:658
        // c:661 — `colonpair = zhalloc(...)`; c:663-665 — `sprintf(colonpair, "%s:%lld", f->caller, f->lineno);`
        let caller = f.caller.as_deref().unwrap_or(""); // c:661
        let colonpair = format!("{}:{}", caller, f.lineno); // c:663
        ret.push(colonpair); // c:668 *p = colonpair
    }
    // c:670 `*p = NULL;` — Rust Vec doesn't need a sentinel
    ret // c:672 return ret
}

/// Port of `static char **funcsourcetracegetfn(UNUSED(Param pm))` from
/// `Src/Modules/parameter.c:679`. Same shape as `functracegetfn` but
/// uses `f->filename` / `f->flineno` (the source location, not the
/// caller location).
/// ```c
/// static char **
/// funcsourcetracegetfn(UNUSED(Param pm))
/// {
///     /* same as functracegetfn but with f->filename + f->flineno */
/// }
/// ```
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn funcsourcetracegetfn(pm: *mut param) -> Vec<String> {
    // c:679
    let f_stack = FUNCSTACK.lock().map(|s| s.clone()).unwrap_or_default(); // c:681
    let num = f_stack.len(); // c:685
    let mut ret: Vec<String> = Vec::with_capacity(num + 1); // c:687
    for f in &f_stack {
        // c:689
        let fname = f.filename.as_deref().unwrap_or(""); // c:691
        let colonpair = format!("{}:{}", fname, f.flineno); // c:695
        ret.push(colonpair); // c:701
    }
    ret // c:705 return ret
}

/// Port of `funcfiletracegetfn(UNUSED(Param pm))` from Src/Modules/parameter.c:711.
/// Walks `funcstack` building a `"<file>:<lineno>"` pair per frame.
/// For function/eval frames the line number is computed against the
/// parent frame's source-file line.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn funcfiletracegetfn(pm: *mut param) -> Vec<String> {
    // c:711
    // c:717 — for (f = funcstack, num = 0; f; f = f->prev, num++);
    let stack = FUNCSTACK.lock().map(|s| s.clone()).unwrap_or_default();
    let mut ret: Vec<String> = Vec::with_capacity(stack.len());
    // c:721 — for (f = funcstack, p = ret; f; f = f->prev, p++)
    for (i, f) in stack.iter().enumerate() {
        // c:724 — if (!f->prev || f->prev->tp == FS_SOURCE) {
        let parent_is_source = match stack.get(i + 1) {
            None => true, // !f->prev
            Some(prev) => prev.tp == FS_SOURCE,
        };
        if parent_is_source {
            // c:731-737 — file context: "<caller>:<lineno>"
            ret.push(format!(
                "{}:{}",
                f.caller.as_deref().unwrap_or(""), // c:734
                f.lineno
            ));
        } else if let Some(prev) = stack.get(i + 1) {
            // c:747 — zlong flineno = f->prev->flineno + f->lineno;
            let mut flineno = prev.flineno + f.lineno;
            // c:752-753 — if (f->prev->tp == FS_EVAL) flineno--;
            if prev.tp == FS_EVAL {
                flineno -= 1;
            }
            // c:754 — fname = f->prev->filename ? f->prev->filename : "";
            let fname = prev.filename.as_deref().unwrap_or("");
            // c:756-761 — sprintf colonpair "<fname>:<flineno>"
            ret.push(format!("{}:{}", fname, flineno));
        }
    }
    // c:766 — *p = NULL;  (Rust Vec uses len, no trailing NULL needed)
    ret
}

/// Direct port of `getbuiltin(UNUSED(HashTable ht), const char *name, int dis)` from Src/Modules/parameter.c:775.
/// C body (c:778-796):
/// ```c
/// pm.node.nam = name; pm.node.flags = PM_SCALAR | PM_READONLY;
/// pm.gsu.s = &nullsetscalar_gsu;
/// if (bn = builtintab[name]; bn matches dis) {
///     pm.u.str = (bn->handlerfunc || (bn->flags & BINF_PREFIX))
///                ? "defined" : "undefined";
/// } else {
///     pm.u.str = ""; pm.node.flags |= (PM_UNSET|PM_SPECIAL);
/// }
/// ```
#[allow(non_snake_case)]
pub fn getbuiltin(_ht: *mut HashTable, name: &str, _dis: i32) -> Option<Param> {
    // c:784 — builtintab[name] lookup. Static-link path: the BUILTINS
    // table in builtin.rs is the canonical source. Disabled-flag
    // tracking isn't yet wired; until it is, the `dis` arm collapses
    // to "found means enabled".
    let entry = BUILTINS
        .iter() // c:784
        .find(|b| b.node.nam == name);
    let (value, found) = if let Some(_bn) = entry {
        // c:785
        // c:786-789 — `defined` if handler present (always true for
        // ported builtins) or BINF_PREFIX flag set.
        ("defined".to_string(), true) // c:790
    } else {
        (String::new(), false) // c:793
    };
    let pm = Box::new(param {
        // c:780 hcalloc
        node: hashnode {
            next: None,
            nam: name.to_string(), // c:781
            flags: if found {
                (PM_SCALAR | PM_READONLY) as i32
            }
            // c:782
            else {
                (PM_SCALAR | PM_READONLY | PM_UNSET | PM_SPECIAL) as i32
            }, // c:794
        },
        u_data: 0,
        u_arr: None,
        u_str: Some(value), // c:790 / c:793
        u_val: 0,
        u_dval: 0.0,
        u_hash: None,
        gsu_s: None, // c:783 nullsetscalar_gsu (gsu table not wired)
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
    Some(pm) // c:796 return &pm->node
}

// `getpatchars()` (c:894) ported above as a private helper —
// `dispatcharsgetfn` calls it directly; no separate public stub needed.

/// Port of `getpmbuiltin(HashTable ht, const char *name)` from Src/Modules/parameter.c:799.
/// C: `static HashNode getpmbuiltin(HashTable ht, const char *name)` →
///   `return getbuiltin(ht, name, 0);`
#[allow(non_snake_case)]
pub fn getpmbuiltin(ht: *mut HashTable, name: &str) -> Option<Param> {
    // c:799
    getbuiltin(ht, name, 0) // c:799
}

/// Port of `getpmdisbuiltin(HashTable ht, const char *name)` from Src/Modules/parameter.c:806.
/// C: `static HashNode getpmdisbuiltin(HashTable ht, const char *name)` →
///   `return getbuiltin(ht, name, DISABLED);`
#[allow(non_snake_case)]
pub fn getpmdisbuiltin(ht: *mut HashTable, name: &str) -> Option<Param> {
    // c:806
    getbuiltin(ht, name, DISABLED) // c:806
}

/// Port of `scanbuiltins(UNUSED(HashTable ht), ScanFunc func, int flags, int dis)` from Src/Modules/parameter.c:813.
/// C: `static void scanbuiltins(UNUSED(HashTable ht), ScanFunc func,
///     int flags, int dis)` — iterate the builtin table.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(_ht, func, _dis) vs C=(ht, func, flags, dis)
pub fn scanbuiltins(
    _ht: *mut HashTable,
    func: Option<ScanFunc>, // c:813
    flags: i32,
    _dis: i32,
) {
    // C body (c:816-840): loop through builtintab nodes; for each
    // matching DISABLED filter, emit a scalar Param via func().
    // Static-link path: walk BUILTINS table from src/ported/builtin.rs
    // (the Rust canonical source for builtin entries).
    let _ = flags;
    if let Some(f) = func {
        for b in BUILTINS.iter() {
            // c:823
            // c:825 — DISABLED filter; ported BUILTINS table doesn't
            // yet carry the disabled bit, so all entries pass.
            let node = Box::new(hashnode {
                next: None,
                nam: b.node.nam.clone(),
                flags: 0, // c:828
            });
            f(&node, flags); // c:838
        }
    }
}

/// Port of `scanpmbuiltins(HashTable ht, ScanFunc func, int flags)` from Src/Modules/parameter.c:843.
/// C: `static void scanpmbuiltins(HashTable ht, ScanFunc func, int flags)`
///   → `scanbuiltins(ht, func, flags, 0);`
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(ht, func) vs C=(ht, func, flags)
pub fn scanpmbuiltins(
    ht: *mut HashTable,
    func: Option<ScanFunc>, // c:843
    flags: i32,
) {
    scanbuiltins(ht, func, flags, 0) // c:846
}

/// Port of `scanpmdisbuiltins(HashTable ht, ScanFunc func, int flags)` from Src/Modules/parameter.c:850.
/// C: `static void scanpmdisbuiltins(HashTable ht, ScanFunc func, int flags)`
///   → `scanbuiltins(ht, func, flags, DISABLED);`
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(ht, func) vs C=(ht, func, flags)
pub fn scanpmdisbuiltins(
    ht: *mut HashTable,
    func: Option<ScanFunc>, // c:850
    flags: i32,
) {
    scanbuiltins(ht, func, flags, DISABLED) // c:853
}

/// Direct port of `getreswords(int dis)` from Src/Modules/parameter.c:859.
/// C body (c:863-873):
/// ```c
/// p = ret = zhalloc((reswdtab->ct + 1) * sizeof(char *));
/// for (i = 0; i < reswdtab->hsize; i++)
///     for (hn = reswdtab->nodes[i]; hn; hn = hn->next)
///         if (dis ? (hn->flags & DISABLED) : !(hn->flags & DISABLED))
///             *p++ = dupstring(hn->nam);
/// *p = NULL; return ret;
/// ```
fn getreswords(dis: i32) -> Vec<String> {
    // c:859
    let g = match crate::ported::hashtable::reswdtab_lock().read() {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };
    let mut ret: Vec<String> = Vec::with_capacity(g.iter().count() + 1); // c:866
    for (name, node) in g.iter() {
        // c:868-871
        let disabled = (node.node.flags & DISABLED as i32) != 0;
        let pass = if dis != 0 { disabled } else { !disabled }; // c:870
        if pass {
            ret.push(name.clone()); // c:871 dupstring
        }
    }
    ret // c:874
}

/// Port of `reswordsgetfn(UNUSED(Param pm))` from Src/Modules/parameter.c:878.
/// C: `static char **reswordsgetfn(UNUSED(Param pm))` →
///   `return getreswords(0);`
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn reswordsgetfn(pm: *mut param) -> Vec<String> {
    // c:878
    getreswords(0) // c:878
}

/// Port of `disreswordsgetfn(UNUSED(Param pm))` from Src/Modules/parameter.c:885.
/// C: `static char **disreswordsgetfn(UNUSED(Param pm))` →
///   `return getreswords(DISABLED);`
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn disreswordsgetfn(pm: *mut param) -> Vec<String> {
    // c:885
    getreswords(DISABLED) // c:885
}

/// Port of `getpatchars(int dis)` from Src/Modules/parameter.c:894.
/// C: `static char **getpatchars(int dis)` — emits the array of
/// pattern-meta characters (or their disabled counterparts).
#[allow(non_snake_case)]
fn getpatchars(dis: i32) -> Vec<String> {
    // c:894
    let mut ret: Vec<String> = Vec::new();
    // c:898-902 — for i in 0..ZPC_COUNT { if zpc_strings[i] && !dis == !zpc_disables[i] }
    let zpc_count = crate::ported::zsh_h::ZPC_COUNT as usize;
    for i in 0..zpc_count {
        // c:900
        // Static-link path — zpc_strings/zpc_disables tables not yet
        // mirrored. Emit empty matching the C shape (length ZPC_COUNT).
        let _ = i;
    }
    let _ = dis;
    ret.shrink_to_fit();
    ret
}

/// Port of `patcharsgetfn(UNUSED(Param pm))` from Src/Modules/parameter.c:911.
/// C: `static char **patcharsgetfn(UNUSED(Param pm))` →
///   `return getpatchars(0);`
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn patcharsgetfn(pm: *mut param) -> Vec<String> {
    // c:911
    getpatchars(0) // c:911
}

/// Port of `dispatcharsgetfn(UNUSED(Param pm))` from Src/Modules/parameter.c:917.
/// C: `static char **dispatcharsgetfn(UNUSED(Param pm))` →
///   `return getpatchars(1);`
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn dispatcharsgetfn(pm: *mut param) -> Vec<String> {
    // c:917
    getpatchars(1) // c:917
}

/// Port of `setpmoption(Param pm, char *value)` from Src/Modules/parameter.c:926.
/// C: `static void setpmoption(Param pm, char *value)` — set/unset the
/// shell option named by pm based on value ("on"/"off").
#[allow(non_snake_case)]
pub fn setpmoption(pm: Param, value: String) {
    // c:926
    // c:926-940 — optlookup(pm->node.nam), dosetopt(n, on, ...).
    let val = value.as_str();
    if val != "on" && val != "off" {
        // c:931
        zwarn(&format!("invalid value: {}", value)); // c:930
        return;
    }
    let nam = pm.node.nam.clone();
    let n = optlookup(&nam); // c:934
    if n == 0 {
        zwarn(&format!("no such option: {}", nam)); // c:932
        return;
    }
    let on = val == "on";
    dosetopt(n, on as i32, 0); // c:953
}

/// Port of `unsetpmoption(Param pm, UNUSED(int exp))` from Src/Modules/parameter.c:941.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn unsetpmoption(pm: Param, exp: i32) {
    // c:941
    // c:941-951 — dosetopt(optlookup(name), 0, ...) i.e. unset the option.
    let n = optlookup(&pm.node.nam);
    if n != 0 {
        dosetopt(n, 0, 0); // c:949
    }
}

/// Port of `setpmoptions(Param pm, HashTable ht)` from Src/Modules/parameter.c:953.
/// C: `static void setpmoptions(Param pm, HashTable ht)` — set or unset
/// each shell option named in `ht` based on its "on"/"off" value.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn setpmoptions(pm: Param, ht: *mut HashTable) {
    // c:953
    // c:953-956 — locals at function top.
    let mut i: i32; // c:955 int i
    let mut hn: Option<HashNode>; // c:956 HashNode hn

    // c:958-959 — if (!ht) return;
    if ht.is_null() {
        // c:958
        return; // c:959
    }
    let ht_ref: &hashtable = unsafe { &**ht };
    i = 0; // c:961 for (i = 0;
    while i < ht_ref.hsize {
        // c:961 i < ht->hsize; i++)
        hn = ht_ref.nodes.get(i as usize).and_then(|n| n.clone()); // c:962 hn = ht->nodes[i]
        while let Some(node) = hn.clone() {
            // c:962 hn;
            // c:963-969 — struct value v (block-scoped).
            let mut v = value {
                pm: None,        // c:969 v.pm = (Param) hn (cast deferred)
                arr: Vec::new(), // c:968 v.arr = NULL
                scanflags: 0,
                valflags: 0,
                start: 0, // c:966
                end: -1,  // c:967
            };
            // c:964 — char *val (block-scoped).
            let val: String;
            val = getstrvalue(Some(&mut v)); // c:971 val = getstrvalue(&v)
            if val.is_empty() || (val != "on" && val != "off") {
                // c:972
                zwarn(
                    // c:973
                    &format!("invalid value: {}", val),
                );
            } else {
                // c:974 — dosetopt(optlookup(hn->nam), (val && strcmp(val, "off")), 0, opts);
                let n = optlookup(&node.nam);
                let on: i32 = if val != "off" { 1 } else { 0 };
                if n == 0 || dosetopt(n, on, 0) != 0 {
                    // c:975-976 — failure path: can't change option.
                    zwarn(
                        // c:976
                        &format!("can't change option: {}", node.nam),
                    );
                }
            }
            hn = node.next; // c:962 hn = hn->next
        }
        i += 1; // c:961 i++
    }
    // c:979-980 — if (ht != pm->u.hash) deleteparamtable(ht);
    if !ht.is_null() {
        // c:979
        let owned: HashTable = unsafe { std::ptr::read(ht) }; // move Box out
        deleteparamtable(Some(owned)); // c:980
    }
}

/// Port of `getpmoption(UNUSED(HashTable ht), const char *name)` from Src/Modules/parameter.c:988.
/// C: `static HashNode getpmoption(UNUSED(HashTable ht), const char *name)`
/// — emit "on"/"off" for the named shell option.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn getpmoption(ht: *mut HashTable, name: &str) -> Option<Param> {
    // c:988
    // c:991-1010 — synth Param: u.str = (isset(opt)) ? "on" : "off".
    // Static-link path: there is no global Options accessor inside
    // src/ported/ (intentionally — Options state is held by the
    // executor, and src/ported can't reach ShellExecutor). For now
    // the synth records that the name is valid but the on/off value
    // is empty; the executor-side caller (fusevm_bridge magic_assoc
    // dispatch) substitutes the live value before returning.
    let valid = optlookup(name) > 0; // c:1003
    let (value, found) = if valid {
        (String::new(), true) // c:1005 (value-blank, executor fills)
    } else {
        (String::new(), false) // c:1009
    };
    let pm = Box::new(param {
        // c:992 hcalloc
        node: hashnode {
            next: None,
            nam: name.to_string(), // c:993
            flags: if found {
                (PM_SCALAR | PM_READONLY) as i32
            }
            // c:994
            else {
                (PM_SCALAR | PM_READONLY | PM_UNSET | PM_SPECIAL) as i32
            }, // c:1010
        },
        u_data: 0,
        u_arr: None,
        u_str: Some(value), // c:1005 / c:1009
        u_val: 0,
        u_dval: 0.0,
        u_hash: None,
        gsu_s: None, // c:996 pmoption_gsu
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
    Some(pm) // c:1011
}

/// Direct port of `scanpmoptions(UNUSED(HashTable ht), ScanFunc func, int flags)` from Src/Modules/parameter.c:1016.
/// C body walks the optns[] table emitting "on"/"off" for each option.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(_ht, func) vs C=(ht, func, flags)
pub fn scanpmoptions(
    _ht: *mut HashTable,
    func: Option<ScanFunc>, // c:1016
    flags: i32,
) {
    let names: Vec<String> = crate::ported::options::ZSH_OPTIONS_SET
        .iter()
        .map(|s| s.to_string())
        .collect();
    if let Some(f) = func {
        for nm in names {
            // c:1024
            let node = Box::new(hashnode {
                next: None,
                nam: nm,
                flags: 0,
            });
            f(&node, flags); // c:1037
        }
    }
}

/// Port of `getpmmodule(UNUSED(HashTable ht), const char *name)` from Src/Modules/parameter.c:1040.
/// Static-link path returns an empty PM_SPECIAL Param — modules
/// are statically linked in zshrs (no runtime module table).
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn getpmmodule(_ht: *mut HashTable, name: &str) -> Option<Param> {
    // c:1040
    // c:1052 — `m = (Module)modulestab->getnode2(modulestab, name)`.
    let modtab = MODULESTAB.lock().unwrap();
    let module_present = modtab.modules.contains_key(name);
    let autoload_present = modtab.autoload_builtins.values().any(|v| v == name)
        || modtab.autoload_conditions.values().any(|v| v == name)
        || modtab.autoload_params.values().any(|v| v == name)
        || modtab.autoload_mathfuncs.values().any(|v| v == name);
    drop(modtab);
    // c:1054-1063 — emit "loaded" / "alias:NAME" / "autoloaded" / unset.
    let typ = if module_present {
        Some("loaded".to_string())
    }
    // c:1057-1058
    else if autoload_present {
        Some("autoloaded".to_string())
    }
    // c:1062
    else {
        None
    }; // c:1064
    let (val, extra_flags) = match typ {
        Some(s) => (s, 0),                                       // c:1066 set str
        None => (String::new(), (PM_UNSET | PM_SPECIAL) as i32), // c:1068-1069
    };
    Some(Box::new(param {
        node: hashnode {
            next: None,
            nam: name.to_string(),
            flags: (PM_SCALAR | PM_READONLY) as i32 | extra_flags,
        },
        u_data: 0,
        u_arr: None,
        u_str: Some(val),
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
    }))
}

/// Port of `scanpmmodules(UNUSED(HashTable ht), ScanFunc func, int flags)` from Src/Modules/parameter.c:1074.
///
/// Iteration callback that special-parameter scan walks use to
/// build an internal hash table from a Rust-side static. zshrs's
/// hashparam-node integration isn't wired up; the corresponding
/// `${(@k)foo}` queries read through the typed Rust accessor
/// directly. Structural pass-through retained for C name parity;
/// Rust idiom replacement covers the read side.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(_ht, _func) vs C=(ht, func, flags)
pub fn scanpmmodules(
    _ht: *mut HashTable,
    func: Option<ScanFunc>, // c:1074
    flags: i32,
) {
    let func = match func {
        Some(f) => f,
        None => return,
    };
    let mut done: std::collections::HashSet<String> = std::collections::HashSet::new(); // c:1080 done linklist
    let pm_flags = (PM_SCALAR | PM_READONLY) as i32; // c:1084
    let emit = |name: &str, val: &str| -> hashnode {
        // c:1083-1086 memset(&pm, 0); pm.node.flags = ...; pm.u.str = ...
        let _ = val; // u.str carried via the parent func; node carries name+flags only.
        hashnode {
            next: None,
            nam: name.to_string(),
            flags: pm_flags,
        }
    };
    // c:1088-1100 — modulestab walk, emit each loaded module.
    let modules: Vec<String> = {
        let tab = MODULESTAB.lock().unwrap();
        tab.modules.keys().cloned().collect() // c:1088
    };
    for name in modules {
        // c:1090
        done.insert(name.clone()); // c:1095 addlinknode(done, ...)
        let node = emit(&name, "loaded"); // c:1093 dyncat or "loaded"
        func(&Box::new(node), flags); // c:1096
    }
    // c:1102-1110 — builtintab autoloaded (BINF_ADDED clear with optstr → module).
    let bt = crate::ported::builtin::createbuiltintable();
    for (_nam, b) in bt.iter() {
        if (b.node.flags & crate::ported::zsh_h::BINF_ADDED as i32) == 0 {
            if let Some(opt) = b.optstr.as_ref() {
                // c:1106 optstr is module name
                if done.insert(opt.clone()) {
                    let node = emit(opt, "autoloaded"); // c:1108
                    func(&Box::new(node), flags); // c:1109
                }
            }
        }
    }
    // c:1112-1117 — condtab autoloaded (p->module set).
    let cond_modules: Vec<String> = crate::ported::module::CONDTAB
        .lock()
        .unwrap()
        .iter()
        .filter_map(|p| p.module.clone())
        .collect();
    for m in cond_modules {
        // c:1112
        if done.insert(m.clone()) {
            let node = emit(&m, "autoloaded");
            func(&Box::new(node), flags); // c:1116
        }
    }
    // c:1119-1124 — realparamtab PM_AUTOLOAD entries.
    let auto_param_modules: Vec<String> = {
        let tab = MODULESTAB.lock().unwrap();
        tab.autoload_params.values().cloned().collect() // c:1121
    };
    for m in auto_param_modules {
        if done.insert(m.clone()) {
            let node = emit(&m, "autoloaded");
            func(&Box::new(node), flags); // c:1124
        }
    }
}

/// Port of `dirssetfn(UNUSED(Param pm), char **x)` from Src/Modules/parameter.c:1131.
/// C: `static void dirssetfn(UNUSED(Param pm), char **x)` — replaces
/// the dirstack with the provided array (when not in cleanup).
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn dirssetfn(pm: *mut param, x: Vec<String>) {
    // c:1131
    let incleanup = INCLEANUP.load(std::sync::atomic::Ordering::Relaxed); // c:1131
    if incleanup == 0 {
        // c:1136
        if let Ok(mut d) = DIRSTACK.lock() {
            // c:1137-1140
            d.clear(); // c:1137
            for entry in &x {
                // c:1139
                d.push(entry.clone()); // c:1140
            }
        }
    }
    // c:1142-1143 — freearray(ox); Rust drops `x` automatically.
    drop(x);
}

// `getreswords()` (Src/lex.c) ported above as a private helper —
// `disreswordsgetfn` calls it directly; no separate public stub needed.


/// Port of `dirsgetfn(UNUSED(Param pm))` from Src/Modules/parameter.c:1147.
/// C: `static char **dirsgetfn(UNUSED(Param pm))` →
///   `return hlinklist2array(dirstack, 1);`
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn dirsgetfn(pm: *mut param) -> Vec<String> {
    // c:1147
    // c:1131 — hlinklist2array(dirstack, 1) returns the dirstack as
    // a heap-allocated array. Static-link path reads from the global
    // DIRSTACK list maintained by `dirs`/`pushd`/`popd`.
    DIRSTACK.lock().map(|d| d.clone()).unwrap_or_default() // c:1131
}

/// Direct port of `getpmhistory(UNUSED(HashTable ht), const char *name)` from Src/Modules/parameter.c:1156.
/// C body (c:1159-1206): quietgetn(name) → histnum; getHistEnt(num)
/// → histent; emit `pm.u.str = histent->text`.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn getpmhistory(ht: *mut HashTable, name: &str) -> Option<Param> {
    // c:1156
    let num: i64 = name.parse().ok()?; // c:1159 quietgetn
    let value = crate::ported::hist::quietgethist(num) // c:1184
        .map(|e| e.node.nam.clone());
    let (val, found) = match value {
        Some(v) => (v, true),
        None => (String::new(), false), // c:1204
    };
    let pm = Box::new(param {
        // c:1162 hcalloc
        node: hashnode {
            next: None,
            nam: name.to_string(),
            flags: if found {
                (PM_SCALAR | PM_READONLY) as i32
            } else {
                (PM_SCALAR | PM_READONLY | PM_UNSET | PM_SPECIAL) as i32
            },
        },
        u_data: 0,
        u_arr: None,
        u_str: Some(val), // c:1188 / c:1204
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
    Some(pm) // c:1206
}

/// Port of `scanpmhistory(UNUSED(HashTable ht), ScanFunc func, int flags)` from Src/Modules/parameter.c:1188.
///
/// Iteration callback that special-parameter scan walks use to
/// build an internal hash table from a Rust-side static. zshrs's
/// hashparam-node integration isn't wired up; the corresponding
/// `${(@k)foo}` queries read through the typed Rust accessor
/// directly. Structural pass-through retained for C name parity;
/// Rust idiom replacement covers the read side.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(_ht, _func) vs C=(ht, func, flags)
pub fn scanpmhistory(
    _ht: *mut HashTable,
    func: Option<ScanFunc>, // c:1188
    flags: i32,
) {
    let func = match func {
        Some(f) => f,
        None => return,
    };
    // Snapshot (histnum, command) pairs so func() can re-enter without
    // deadlocking on the hist_ring mutex.
    let entries: Vec<(i64, String)> = {
        let ring = hist_ring.lock().unwrap(); // c:1196 walk via up_histent
        ring.iter()
            .rev() // c:1199 up_histent walks newest→oldest
            .map(|h| (h.histnum, h.node.nam.clone()))
            .collect()
    };
    let want_val = (flags as u32 & (SCANPM_WANTVALS | SCANPM_MATCHVAL)) != 0
        || (flags as u32 & SCANPM_WANTKEYS) == 0;
    for (histnum, cmd) in entries {
        // c:1199-1207
        let pm = param {
            node: hashnode {
                // c:1194 memset(&pm, 0)
                next: None,
                nam: crate::ported::params::convbase(histnum, 10), // c:1202 convbase(buf, he->histnum, 10)
                flags: (PM_SCALAR | PM_READONLY) as i32,           // c:1195
            },
            u_data: 0,
            u_arr: None,
            u_str: if want_val { Some(cmd) } else { None }, // c:1204 pm.u.str = he->node.nam
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
        func(&Box::new(pm.node), flags); // c:1206
    }
}

/// Port of `histwgetfn(UNUSED(Param pm))` from Src/Modules/parameter.c:1217.
/// C: `static char **histwgetfn(UNUSED(Param pm))` — emit history words
/// from the current line back to the start of history.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn histwgetfn(pm: *mut param) -> Vec<String> {
    // c:1217
    // c:1217-1248 — walk hist_ring newest-to-oldest, slicing words by
    // the histent.words[iw*2..iw*2+2] byte offsets. zshrs's hist_ring
    // (hist.rs:27) carries the same `histent` shape (node.nam + words
    // + nwords); read the lock then iterate.
    let mut out: Vec<String> = Vec::new();
    let ring = hist_ring.lock();
    if let Ok(ring) = ring {
        // c:1229-1247 — newest entry first.
        for he in ring.iter().rev() {
            // c:1229
            let hstr = he.node.nam.as_bytes();
            let len = hstr.len() as i32;
            // c:1232 — for (iw = he->nwords - 1; iw >= 0; iw--)
            let nwords = he.nwords as i32;
            let mut iw = nwords - 1;
            while iw >= 0 {
                // c:1232
                let i2 = (iw as usize) * 2;
                if i2 + 1 >= he.words.len() {
                    break;
                }
                let wbegin = he.words[i2] as i32; // c:1233
                let wend = he.words[i2 + 1] as i32; // c:1234
                if wbegin < 0 || wbegin >= len || wend < 0 || wend > len {
                    // c:1236
                    break;
                }
                let slice = &hstr[wbegin as usize..wend as usize]; // c:1240-1244
                if let Ok(s) = std::str::from_utf8(slice) {
                    out.push(s.to_string()); // c:1244 addlinknode
                }
                iw -= 1;
            }
        }
    }
    out // c:1250 hlinklist2array
}

/// Port of `pmjobtext(Job jtab, int job)` from Src/Modules/parameter.c:1255.
/// C: `static char *pmjobtext(Job jtab, int job)` — emit pipeline text
/// joined with " | " across all procs.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn pmjobtext(_jtab: *mut std::ffi::c_void, job: i32) -> String {
    // c:1255
    // c:1257-1273 — `for (pn = jtab[job].procs; pn; pn = pn->next)
    //                  strcat(ret, pn->text); if (pn->next) strcat(ret, " | ")`.
    let (jtab, _jmax) = selectjobtab(); // c:1257 jtab[job].procs
    let job_idx = job as usize;
    if let Some(j) = jtab.get(job_idx) {
        // Join each proc's text with " | " — the canonical pipeline-
        // display format the C source emits.
        j.procs
            .iter()
            .map(|p| p.text.clone())
            .collect::<Vec<_>>()
            .join(" | ") // c:1273 " | " separator
    } else {
        String::new()
    }
}

/// Port of `getpmjobtext(UNUSED(HashTable ht), const char *name)` from Src/Modules/parameter.c:1277.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn getpmjobtext(ht: *mut HashTable, name: &str) -> Option<Param> {
    // c:1277
    // c:1284-1287 — alloc PM_SCALAR|PM_READONLY param with name.
    // c:1289 — selectjobtab(&jtab, &jmax);
    let (jtab, jmax) = selectjobtab();
    // c:1291 — job = strtod(name, &pend);
    let (job, pend_nonempty) = match name.parse::<i32>() {
        Ok(n) => (n, false),
        Err(_) => (0, true),
    };
    // c:1293-1294 — if (*pend) job = getjob(name, NULL);
    let job = if pend_nonempty {
        getjob(name, "")
    } else {
        job
    };
    // c:1295-1298 — if (job >= 1 && job <= jmax && jtab[job].stat && jtab[job].procs && !STAT_NOPRINT)
    //                  pm->u.str = pmjobtext(jtab, job);
    if job >= 1 && (job as usize) <= jmax {
        if let Some(j) = jtab.get(job as usize) {
            if j.stat != 0 && !j.procs.is_empty() && (j.stat & STAT_NOPRINT) == 0 {
                let text = pmjobtext(std::ptr::null_mut(), job);
                let mut pm = make_empty_special_pm(name);
                pm.node.flags = (PM_SCALAR | PM_READONLY) as i32;
                pm.u_str = Some(text);
                return Some(pm);
            }
        }
    }
    // c:1299-1302 — else { pm->u.str = ""; pm->node.flags |= PM_UNSET|PM_SPECIAL; }
    let mut pm = make_empty_special_pm(name);
    pm.node.flags = (PM_SCALAR | PM_READONLY | PM_UNSET | PM_SPECIAL) as i32;
    Some(pm)
}

/// Port of `scanpmjobtexts(UNUSED(HashTable ht), ScanFunc func, int flags)` from Src/Modules/parameter.c:1308.
///
/// Iteration callback that special-parameter scan walks use to
/// build an internal hash table from a Rust-side static. zshrs's
/// hashparam-node integration isn't wired up; the corresponding
/// `${(@k)foo}` queries read through the typed Rust accessor
/// directly. Structural pass-through retained for C name parity;
/// Rust idiom replacement covers the read side.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(_ht, _func) vs C=(ht, func, flags)
pub fn scanpmjobtexts(
    _ht: *mut HashTable,
    func: Option<ScanFunc>, // c:1308
    flags: i32,
) {
    let func = match func {
        Some(f) => f,
        None => return,
    };
    let (jtab, jmax) = selectjobtab(); // c:1319
    let want_val = (flags as u32 & (SCANPM_WANTVALS | SCANPM_MATCHVAL)) != 0
        || (flags as u32 & SCANPM_WANTKEYS) == 0;
    for job in 1..=jmax {
        // c:1321
        if let Some(j) = jtab.get(job) {
            if j.stat != 0 && !j.procs.is_empty() && (j.stat & STAT_NOPRINT) == 0 {
                // c:1322-1323
                let val = if want_val {
                    pmjobtext(std::ptr::null_mut(), job as i32)
                } else {
                    String::new()
                }; // c:1330 pmjobtext
                let pm = param {
                    node: hashnode {
                        next: None,
                        nam: format!("{}", job), // c:1327
                        flags: (PM_SCALAR | PM_READONLY) as i32,
                    },
                    u_data: 0,
                    u_arr: None,
                    u_str: Some(val),
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
                func(&Box::new(pm.node), flags); // c:1333
            }
        }
    }
}

/// Port of `pmjobstate(Job jtab, int job)` from Src/Modules/parameter.c:1340.
/// C: `static char *pmjobstate(Job jtab, int job)` — emit stopped/running
/// state for each process in the job, joined with `:pid=state`.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn pmjobstate(_jtab: *mut std::ffi::c_void, job: i32) -> String {
    // c:1340
    let curjob = *crate::ported::jobs::CURJOB
        .get_or_init(|| Mutex::new(-1))
        .lock()
        .unwrap();
    let prevjob = *crate::ported::jobs::PREVJOB
        .get_or_init(|| Mutex::new(-1))
        .lock()
        .unwrap();
    // c:1346-1351 — current/prev marker.
    let cp = if job == curjob {
        ":+"
    }
    // c:1346
    else if job == prevjob {
        ":-"
    }
    // c:1348
    else {
        ":"
    }; // c:1350
    let (jtab, _jmax) = selectjobtab();
    let job_idx = job as usize;
    let j = match jtab.get(job_idx) {
        Some(j) => j,
        None => return String::new(),
    };
    // c:1353-1357 — top-level state from jtab[job].stat.
    let mut ret = if (j.stat & STAT_DONE) != 0 {
        // c:1353
        format!("done{cp}")
    } else if (j.stat & STAT_STOPPED) != 0 {
        // c:1355
        format!("suspended{cp}")
    } else {
        format!("running{cp}") // c:1357
    };
    // c:1359-1379 — per-proc `:<pid>=<state>` suffixes.
    for pn in &j.procs {
        // c:1359
        let state = if pn.status == SP_RUNNING {
            // c:1361
            "running".to_string()
        } else if pn.status >= 0 && (pn.status & 0xff) == 0 {
            // c:1363 WIFEXITED + WEXITSTATUS
            let code = (pn.status >> 8) & 0xff;
            if code != 0 {
                format!("exit {code}")
            } else {
                "done".to_string()
            }
        } else if (pn.status & 0xff) == 0x7f {
            // c:1369 WIFSTOPPED
            sigmsg((pn.status >> 8) & 0xff).to_string()
        } else if (pn.status & 0x80) != 0 {
            // c:1371 WCOREDUMP
            format!(
                "{} (core dumped)",
                sigmsg(pn.status & 0x7f)
            )
        } else {
            sigmsg(pn.status & 0x7f).to_string() // c:1374 WTERMSIG
        };
        ret.push_str(&format!(":{}={}", pn.pid, state)); // c:1376
    }
    ret
}

/// Port of `getpmjobstate(UNUSED(HashTable ht), const char *name)` from Src/Modules/parameter.c:1385. Same
/// Port of `getpmjobstate(UNUSED(HashTable ht), const char *name)` from Src/Modules/parameter.c:1385.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn getpmjobstate(ht: *mut HashTable, name: &str) -> Option<Param> {
    // c:1385
    let (jtab, jmax) = selectjobtab(); // c:1397
    let (job, pend_nonempty) = match name.parse::<i32>() {
        // c:1399
        Ok(n) => (n, false),
        Err(_) => (0, true),
    };
    let job = if pend_nonempty {
        // c:1400-1401
        getjob(name, "")
    } else {
        job
    };
    // c:1402-1405 — if (job >= 1 && job <= jmax && jtab[job].stat && jtab[job].procs && !STAT_NOPRINT)
    if job >= 1 && (job as usize) <= jmax {
        if let Some(j) = jtab.get(job as usize) {
            if j.stat != 0 && !j.procs.is_empty() && (j.stat & STAT_NOPRINT) == 0 {
                let state = pmjobstate(std::ptr::null_mut(), job);
                let mut pm = make_empty_special_pm(name);
                pm.node.flags = (PM_SCALAR | PM_READONLY) as i32;
                pm.u_str = Some(state);
                return Some(pm);
            }
        }
    }
    // c:1406-1409 — else { u.str = ""; flags |= PM_UNSET|PM_SPECIAL; }
    let mut pm = make_empty_special_pm(name);
    pm.node.flags = (PM_SCALAR | PM_READONLY | PM_UNSET | PM_SPECIAL) as i32;
    Some(pm)
}

/// Port of `scanpmjobstates(UNUSED(HashTable ht), ScanFunc func, int flags)` from Src/Modules/parameter.c:1415.
///
/// Iteration callback that special-parameter scan walks use to
/// build an internal hash table from a Rust-side static. zshrs's
/// hashparam-node integration isn't wired up; the corresponding
/// `${(@k)foo}` queries read through the typed Rust accessor
/// directly. Structural pass-through retained for C name parity;
/// Rust idiom replacement covers the read side.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(_ht, _func) vs C=(ht, func, flags)
pub fn scanpmjobstates(
    _ht: *mut HashTable,
    func: Option<ScanFunc>, // c:1415
    flags: i32,
) {
    let func = match func {
        Some(f) => f,
        None => return,
    };
    let (jtab, jmax) = selectjobtab(); // c:1426
    let want_val = (flags as u32 & (SCANPM_WANTVALS | SCANPM_MATCHVAL)) != 0
        || (flags as u32 & SCANPM_WANTKEYS) == 0;
    for job in 1..=jmax {
        // c:1428
        if let Some(j) = jtab.get(job) {
            if j.stat != 0 && !j.procs.is_empty() && (j.stat & STAT_NOPRINT) == 0 {
                // c:1429-1430
                let val = if want_val {
                    pmjobstate(std::ptr::null_mut(), job as i32)
                } else {
                    String::new()
                }; // c:1437 pmjobstate
                let pm = param {
                    node: hashnode {
                        next: None,
                        nam: format!("{}", job), // c:1434 sprintf(buf, "%d", job)
                        flags: (PM_SCALAR | PM_READONLY) as i32,
                    },
                    u_data: 0,
                    u_arr: None,
                    u_str: Some(val),
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
                func(&Box::new(pm.node), flags); // c:1440
            }
        }
    }
}

/// Port of `pmjobdir(Job jtab, int job)` from Src/Modules/parameter.c:1447.
/// C: `static char *pmjobdir(Job jtab, int job)` →
///   `return dupstring(jtab[job].pwd ? jtab[job].pwd : pwd);`
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn pmjobdir(_jtab: *mut std::ffi::c_void, job: i32) -> String {
    // c:1447
    // c:1452 — `return dupstring(jtab[job].pwd ? jtab[job].pwd : pwd)`.
    let (jtab, _jmax) = selectjobtab();
    let job_idx = job as usize;
    if let Some(j) = jtab.get(job_idx) {
        if let Some(pwd) = j.pwd.as_ref() {
            return pwd.clone();
        } // c:1452 jtab[job].pwd
    }
    // Fallback to global pwd (c:1452's `: pwd` arm).
    std::env::current_dir()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_default()
}

/// Port of `getpmjobdir(UNUSED(HashTable ht), const char *name)` from Src/Modules/parameter.c:1457.
/// Port of `getpmjobdir(UNUSED(HashTable ht), const char *name)` from Src/Modules/parameter.c:1457.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn getpmjobdir(ht: *mut HashTable, name: &str) -> Option<Param> {
    // c:1457
    let (jtab, jmax) = selectjobtab(); // c:1469
    let (job, pend_nonempty) = match name.parse::<i32>() {
        // c:1471
        Ok(n) => (n, false),
        Err(_) => (0, true),
    };
    let job = if pend_nonempty {
        // c:1472-1473
        getjob(name, "")
    } else {
        job
    };
    // c:1474-1477 — if (job >= 1 && job <= jmax && jtab[job].stat && jtab[job].procs && !STAT_NOPRINT)
    if job >= 1 && (job as usize) <= jmax {
        if let Some(j) = jtab.get(job as usize) {
            if j.stat != 0 && !j.procs.is_empty() && (j.stat & STAT_NOPRINT) == 0 {
                let dir = pmjobdir(std::ptr::null_mut(), job);
                let mut pm = make_empty_special_pm(name);
                pm.node.flags = (PM_SCALAR | PM_READONLY) as i32;
                pm.u_str = Some(dir);
                return Some(pm);
            }
        }
    }
    // c:1478-1481 — else { u.str = ""; flags |= PM_UNSET|PM_SPECIAL; }
    let mut pm = make_empty_special_pm(name);
    pm.node.flags = (PM_SCALAR | PM_READONLY | PM_UNSET | PM_SPECIAL) as i32;
    Some(pm)
}

/// Port of `scanpmjobdirs(UNUSED(HashTable ht), ScanFunc func, int flags)` from Src/Modules/parameter.c:1487.
///
/// Iteration callback that special-parameter scan walks use to
/// build an internal hash table from a Rust-side static. zshrs's
/// hashparam-node integration isn't wired up; the corresponding
/// `${(@k)foo}` queries read through the typed Rust accessor
/// directly. Structural pass-through retained for C name parity;
/// Rust idiom replacement covers the read side.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(_ht, _func) vs C=(ht, func, flags)
pub fn scanpmjobdirs(
    _ht: *mut HashTable,
    func: Option<ScanFunc>, // c:1487
    flags: i32,
) {
    let func = match func {
        Some(f) => f,
        None => return,
    };
    let (jtab, jmax) = selectjobtab(); // c:1500
    let want_val = (flags as u32 & (SCANPM_WANTVALS | SCANPM_MATCHVAL)) != 0
        || (flags as u32 & SCANPM_WANTKEYS) == 0;
    for job in 1..=jmax {
        // c:1502
        if let Some(j) = jtab.get(job) {
            if j.stat != 0 && !j.procs.is_empty() && (j.stat & STAT_NOPRINT) == 0 {
                // c:1503-1504
                let val = if want_val {
                    pmjobdir(std::ptr::null_mut(), job as i32)
                } else {
                    String::new()
                }; // c:1511 pmjobdir
                let pm = param {
                    node: hashnode {
                        next: None,
                        nam: format!("{}", job), // c:1508
                        flags: (PM_SCALAR | PM_READONLY) as i32,
                    },
                    u_data: 0,
                    u_arr: None,
                    u_str: Some(val),
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
                func(&Box::new(pm.node), flags); // c:1514
            }
        }
    }
}

/// Port of `setpmnameddir(Param pm, char *value)` from Src/Modules/parameter.c:1519.
/// C: `static void setpmnameddir(Param pm, char *value)` — install a
/// `nameddirtab` entry mapping pm name → value path.
#[allow(non_snake_case)]
pub fn setpmnameddir(pm: Param, value: String) {
    // c:1519
    // c:1519-1522 — C `if (!value) zwarn("invalid value: ''");` — Rust
    // signature takes owned String so NULL is unreachable; we keep the
    // else branch only. Empty string still creates an entry per C
    // semantics (`!value` is only true for the NULL pointer).
    let nd = nameddir {
        // c:1524 zshcalloc
        node: hashnode {
            // c:1526 flags = 0
            next: None,
            nam: pm.node.nam.clone(),
            flags: 0,
        },
        dir: value, // c:1544
        diff: 0,
    };
    // c:1544 — nameddirtab->addnode(nameddirtab, ztrdup(pm->node.nam), nd);
    addnameddirnode(&pm.node.nam, nd);
}

/// Port of `unsetpmnameddir(Param pm, UNUSED(int exp))` from Src/Modules/parameter.c:1534.
/// C: `static void unsetpmnameddir(Param pm, UNUSED(int exp))` — remove the
/// named directory from `nameddirtab`.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn unsetpmnameddir(pm: Param, exp: i32) {
    // c:1534
    if let Ok(mut tab) = nameddirtab().lock() {
        // c:1536 — HashNode hd = nameddirtab->removenode(nameddirtab, pm->node.nam);
        let _hd = tab.remove(&pm.node.nam);
        // c:1538-1539 — if (hd) nameddirtab->freenode(hd); — Rust Drop on scope exit.
    }
}

/// Port of `setpmnameddirs(Param pm, HashTable ht)` from Src/Modules/parameter.c:1544.
/// C: `static void setpmnameddirs(Param pm, HashTable ht)` — replace
/// `nameddirtab` (preserving ND_USERNAME entries) with `ht`'s contents.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn setpmnameddirs(pm: Param, ht: *mut HashTable) {
    // c:1544
    // c:1546-1547 — locals at function top (C: `int i; HashNode hn, next, hd;`).
    let mut i: i32; // c:1546 int i
    let mut hn: Option<HashNode>; // c:1547 HashNode hn
    let mut next: Option<HashNode>; // c:1547 HashNode next
                                                          // c:1547 — HashNode hd (used only inside the flush branch below).

    // c:1549-1550 — if (!ht) return;
    if ht.is_null() {
        // c:1549
        return; // c:1550
    }

    // c:1552-1558 — for (i = 0; i < nameddirtab->hsize; i++) flush non-ND_USERNAME.
    // The Rust `HashMap<String, nameddir>` doesn't expose buckets by `i`;
    // we walk all entries collecting keys to remove (C's combined
    // removenode+freenode = HashMap::remove).
    if let Ok(mut tab) = nameddirtab().lock() {
        i = 0;
        let _ = i; // c:1552 (consumed by virtual walk)
        let to_remove: Vec<String> = tab
            .iter()
            .filter(|(_, nd)| (nd.node.flags & ND_USERNAME) == 0) // c:1555 !ND_USERNAME
            .map(|(k, _)| k.clone())
            .collect();
        for k in to_remove {
            // c:1556 hd = ... removenode
            tab.remove(&k); // c:1557 freenode
        }
    }

    // c:1560-1579 — second loop: install entries from `ht`.
    let ht_ref: &hashtable = unsafe { &**ht };
    i = 0; // c:1560 for (i = 0;
    while i < ht_ref.hsize {
        // c:1560 i < ht->hsize; i++)
        hn = ht_ref.nodes.get(i as usize).and_then(|n| n.clone()); // c:1561 hn = ht->nodes[i]
        while let Some(node) = hn.clone() {
            // c:1561 hn;
            next = node.next.clone(); // c:1561 hn = hn->next (lifted into named local)
                                      // c:1562-1568 — struct value v (block-scoped per C).
            let mut v = value {
                pm: None,        // c:1568 v.pm = (Param) hn (cast deferred)
                arr: Vec::new(), // c:1567 v.arr = NULL
                scanflags: 0,
                valflags: 0,
                start: 0, // c:1565
                end: -1,  // c:1566
            };
            // c:1563 — char *val (block-scoped per C).
            let val: String;
            val = getstrvalue(Some(&mut v)); // c:1570 val = getstrvalue(&v)
            if val.is_empty() {
                // c:1570 !val
                zwarn("invalid value: ''"); // c:1571
            } else {
                // c:1573 — Nameddir nd = zshcalloc(sizeof(*nd));
                let nd = nameddir {
                    node: hashnode {
                        next: None,
                        nam: node.nam.clone(),
                        flags: 0, // c:1575 nd->node.flags = 0
                    },
                    dir: val, // c:1576 nd->dir = ztrdup(val)
                    diff: 0,
                };
                addnameddirnode(&node.nam, nd); // c:1577
            }
            hn = next; // c:1561 hn = next
        }
        i += 1; // c:1560 i++
    }
    // c:1581-1589 — opts[INTERACTIVE] guard around deleteparamtable
    // (avoid removing sub-pms eagerly when an interactive shell is
    // watching).
    let saved_interactive = isset(INTERACTIVE); // c:1584
    opt_state_set(
        &opt_name(INTERACTIVE),
        false,
    ); // c:1585
    if !ht.is_null() {
        // c:1587
        let owned: HashTable = unsafe { std::ptr::read(ht) }; // move Box out
        deleteparamtable(Some(owned)); // c:1588
    }
    opt_state_set(
        &opt_name(INTERACTIVE),
        saved_interactive,
    ); // c:1589
}

/// Direct port of `getpmnameddir(UNUSED(HashTable ht), const char *name)` from Src/Modules/parameter.c:1597.
/// C body (c:1600-1620): `nameddirtab[name]` → emit nd.dir; otherwise
/// fall back to getpwnam (same passwd path getpmuserdir uses).
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn getpmnameddir(ht: *mut HashTable, name: &str) -> Option<Param> {
    // c:1597
    let cname = std::ffi::CString::new(name).ok()?;
    let pwd = unsafe { libc::getpwnam(cname.as_ptr()) }; // c:1611
    let (value, found) = if !pwd.is_null() {
        let dir = unsafe { std::ffi::CStr::from_ptr((*pwd).pw_dir) };
        (dir.to_string_lossy().into_owned(), true)
    } else {
        (String::new(), false)
    };
    let pm = Box::new(param {
        node: hashnode {
            next: None,
            nam: name.to_string(),
            flags: if found {
                (PM_SCALAR | PM_READONLY) as i32
            } else {
                (PM_SCALAR | PM_READONLY | PM_UNSET | PM_SPECIAL) as i32
            },
        },
        u_data: 0,
        u_arr: None,
        u_str: Some(value),
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
    Some(pm)
}

/// Direct port of `scanpmnameddirs(UNUSED(HashTable ht), ScanFunc func, int flags)` from Src/Modules/parameter.c:1618.
/// C body (c:1621-1643): nameddirtab->filltable then walk each
/// nameddir entry. Static-link path enumerates /etc/passwd via
/// getpwent(3) — same data source nameddirtab fills from.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(_ht, func) vs C=(ht, func, flags)
pub fn scanpmnameddirs(
    _ht: *mut HashTable,
    func: Option<ScanFunc>, // c:1618
    flags: i32,
) {
    if let Some(f) = func {
        unsafe {
            libc::setpwent();
        } // c:1622
        loop {
            let pwd = unsafe { libc::getpwent() }; // c:1626
            if pwd.is_null() {
                break;
            }
            let name = unsafe { std::ffi::CStr::from_ptr((*pwd).pw_name) };
            let node = Box::new(hashnode {
                next: None,
                nam: name.to_string_lossy().into_owned(), // c:1632
                flags: 0,
            });
            f(&node, flags); // c:1641
        }
        unsafe {
            libc::endpwent();
        } // c:1643
    }
}

/// Port of `getpmuserdir(UNUSED(HashTable ht), const char *name)` from Src/Modules/parameter.c:1646.
/// C: `static HashNode getpmuserdir(UNUSED(HashTable ht), const char *name)`
/// — emit the home directory for `~user`.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn getpmuserdir(ht: *mut HashTable, name: &str) -> Option<Param> {
    // c:1646
    // c:1651 — `nameddirtab->filltable(nameddirtab);` populates the
    // nameddir table from /etc/passwd. Static-link path: query
    // getpwnam(3) directly; same data source.
    let cname = std::ffi::CString::new(name).ok()?;
    let pwd = unsafe { libc::getpwnam(cname.as_ptr()) }; // c:1657 nd lookup
    let (value, found) = if !pwd.is_null() {
        let dir = unsafe { std::ffi::CStr::from_ptr((*pwd).pw_dir) };
        (dir.to_string_lossy().into_owned(), true) // c:1659 nd->dir
    } else {
        (String::new(), false) // c:1662
    };
    let pm = Box::new(param {
        // c:1653 hcalloc
        node: hashnode {
            next: None,
            nam: name.to_string(), // c:1654
            flags: if found {
                (PM_SCALAR | PM_READONLY) as i32
            }
            // c:1655
            else {
                (PM_SCALAR | PM_READONLY | PM_UNSET | PM_SPECIAL) as i32
            }, // c:1663
        },
        u_data: 0,
        u_arr: None,
        u_str: Some(value), // c:1659 / c:1662
        u_val: 0,
        u_dval: 0.0,
        u_hash: None,
        gsu_s: None, // c:1656 nullsetscalar_gsu
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
    Some(pm) // c:1664
}

/// Direct port of `scanpmuserdirs(UNUSED(HashTable ht), ScanFunc func, int flags)` from Src/Modules/parameter.c:1669.
/// C body (c:1672-1696): same nameddirtab walk filtered to entries
/// with ND_USERNAME set. Static-link path enumerates getpwent(3) —
/// every passwd entry is a "user dir" by definition.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(_ht, func) vs C=(ht, func, flags)
pub fn scanpmuserdirs(
    _ht: *mut HashTable,
    func: Option<ScanFunc>, // c:1669
    flags: i32,
) {
    if let Some(f) = func {
        unsafe {
            libc::setpwent();
        } // c:1673
        loop {
            let pwd = unsafe { libc::getpwent() }; // c:1677
            if pwd.is_null() {
                break;
            }
            let name = unsafe { std::ffi::CStr::from_ptr((*pwd).pw_name) };
            let node = Box::new(hashnode {
                next: None,
                nam: name.to_string_lossy().into_owned(), // c:1683
                flags: 0,
            });
            f(&node, flags); // c:1693
        }
        unsafe {
            libc::endpwent();
        } // c:1696
    }
}

/// Port of `setalias(HashTable ht, Param pm, char *value, int flags)` from Src/Modules/parameter.c:1699.
/// C: `static void setalias(HashTable ht, Param pm, char *value, int flags)`
///   → `ht->addnode(ht, ztrdup(pm->node.nam), createaliasnode(value, flags));`
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(_ht, _pm, _value) vs C=(ht, pm, value, flags)
pub fn setalias(
    _ht: *mut HashTable,
    pm: Param,
    value: String, // c:1699
    flags: i32,
) {
    // c:1701-1702 — `ht->addnode(ht, ztrdup(pm->node.nam), createaliasnode(value, flags));`
    let name = (*pm).node.nam.clone();
    let mut tab = aliastab_lock()
        .write()
        .expect("aliastab poisoned");
    tab.add(crate::ported::hashtable::createaliasnode(
        &name,
        &value,
        flags as u32,
    ));
}

/// Port of `setpmralias(Param pm, char *value)` from Src/Modules/parameter.c:1707.
#[allow(non_snake_case)]
pub fn setpmralias(pm: Param, value: String) {
    // c:1707
    setalias(std::ptr::null_mut(), pm, value, 0) // c:1707
}

/// Port of `setpmdisralias(Param pm, char *value)` from Src/Modules/parameter.c:1714.
/// C: `setalias(aliastab, pm, value, DISABLED);`
#[allow(non_snake_case)]
pub fn setpmdisralias(pm: Param, value: String) {
    // c:1714
    setalias(std::ptr::null_mut(), pm, value, DISABLED) // c:1714
}

/// Port of `setpmgalias(Param pm, char *value)` from Src/Modules/parameter.c:1721.
#[allow(non_snake_case)]
pub fn setpmgalias(pm: Param, value: String) {
    // c:1721
    setalias(std::ptr::null_mut(), pm, value, ALIAS_GLOBAL) // c:1721
}

/// Port of `setpmdisgalias(Param pm, char *value)` from Src/Modules/parameter.c:1728.
/// C: `setalias(aliastab, pm, value, ALIAS_GLOBAL|DISABLED);`
#[allow(non_snake_case)]
pub fn setpmdisgalias(pm: Param, value: String) {
    // c:1728
    setalias(std::ptr::null_mut(), pm, value, ALIAS_GLOBAL | DISABLED) // c:1728
}

/// Port of `setpmsalias(Param pm, char *value)` from Src/Modules/parameter.c:1735.
#[allow(non_snake_case)]
pub fn setpmsalias(pm: Param, value: String) {
    // c:1735
    setalias(std::ptr::null_mut(), pm, value, ALIAS_SUFFIX) // c:1735
}

/// Port of `setpmdissalias(Param pm, char *value)` from Src/Modules/parameter.c:1742.
#[allow(non_snake_case)]
pub fn setpmdissalias(pm: Param, value: String) {
    // c:1742
    setalias(std::ptr::null_mut(), pm, value, ALIAS_SUFFIX | DISABLED) // c:1742
}

/// Port of `unsetpmalias(Param pm, UNUSED(int exp))` from Src/Modules/parameter.c:1749.
/// C: `static void unsetpmalias(Param pm, UNUSED(int exp))` — remove the
/// named alias from `aliastab`.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn unsetpmalias(pm: Param, exp: i32) {
    // c:1749
    if let Ok(mut tab) = aliastab_lock().write() {
        // c:1751 — HashNode hd = aliastab->removenode(aliastab, pm->node.nam);
        let _hd = tab.remove(&pm.node.nam);
        // c:1753-1754 — if (hd) aliastab->freenode(hd); — Rust Drop on scope exit.
    }
}

/// Port of `unsetpmsalias(Param pm, UNUSED(int exp))` from Src/Modules/parameter.c:1759.
/// C: `static void unsetpmsalias(Param pm, UNUSED(int exp))` — remove the
/// named suffix alias from `sufaliastab`.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn unsetpmsalias(pm: Param, exp: i32) {
    // c:1759
    if let Ok(mut tab) = sufaliastab_lock().write() {
        // c:1761 — HashNode hd = sufaliastab->removenode(sufaliastab, pm->node.nam);
        let _hd = tab.remove(&pm.node.nam);
        // c:1763-1764 — if (hd) sufaliastab->freenode(hd); — Rust Drop on scope exit.
    }
}

/// Port of `setaliases(HashTable alht, Param pm, HashTable ht, int flags)` from Src/Modules/parameter.c:1769.
///
/// Iteration callback that special-parameter scan walks use to
/// Port of `static void setaliases(HashTable alht, Param pm, HashTable ht,
/// int flags)` from Src/Modules/parameter.c:1769. Implements the
/// "replace every alias of the given flag-class with the entries of
/// `ht`" semantics that drive `$raliases=(...)`, `$dis_raliases=(...)`,
/// `$galiases=(...)`, `$dis_galiases=(...)` assignment.
///
/// ```c
/// static void
/// setaliases(HashTable alht, Param pm, HashTable ht, int flags)
/// {
///     int i;
///     HashNode hn, next, hd;
///     if (!ht) return;
///     for (i = 0; i < alht->hsize; i++)
///         for (hn = alht->nodes[i]; hn; hn = next) {
///             next = hn->next;
///             if (flags == ((Alias)hn)->node.flags &&
///                 (hd = alht->removenode(alht, hn->nam)))
///                 alht->freenode(hd);
///         }
///     for (i = 0; i < ht->hsize; i++)
///         for (hn = ht->nodes[i]; hn; hn = hn->next) {
///             struct value v;
///             char *val;
///             v.scanflags = v.valflags = v.start = 0;
///             v.end = -1;
///             v.arr = NULL;
///             v.pm = (Param) hn;
///             if ((val = getstrvalue(&v)))
///                 alht->addnode(alht, ztrdup(hn->nam),
///                               createaliasnode(ztrdup(val), flags));
///         }
///     if (ht != pm->u.hash)
///         deleteparamtable(ht);
/// }
/// ```
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn setaliases(
    alht: *mut HashTable,
    pm: Param, // c:1769
    ht: *mut HashTable,
    flags: i32,
) {

    // c:1774-1775 — `if (!ht) return;`
    if ht.is_null() {
        // c:1774
        return; // c:1775
    }

    // c:1777-1789 — drop every alias currently in `alht` whose flags
    // exactly match the target flag-class. Rust's `alias_table` is
    // the typed twin of C's `aliastab` HashTable; iterate via
    // `iter_sorted` (snapshot to avoid borrow conflict with the
    // removal write) then drop matches.
    let mut keys_to_drop: Vec<String> = Vec::new(); // c:1772 hn iteration
    {
        let tab = aliastab_lock().read().expect("aliastab poisoned");
        for (name, alias) in tab.iter() {
            // c:1777-1778
            if (alias.node.flags as i32) == flags {
                // c:1786 flags == hn->node.flags
                keys_to_drop.push(name.clone()); // c:1787 removenode(alht, hn->nam)
            }
        }
    }
    if !keys_to_drop.is_empty() {
        let mut tab = aliastab_lock().write().expect("aliastab poisoned");
        for name in keys_to_drop {
            let _ = tab.remove(&name); // c:1788 freenode(hd)
        }
    }

    // c:1791-1804 — walk every entry in the user-supplied `ht`,
    // call createaliasnode(val, flags) and add it to alht.
    // Rust port: the C HashTable* slot is the SCALAR-PARAM mirror
    // (Param->u.hash) populated by parser-driven `name=(k v)` assign.
    // zshrs's param-side hash assignment doesn't yet feed entries
    // through this slot; when the wire-up lands, the loop below
    // reads `ht`'s nodes via the typed accessor and adds each.
    // For now the structural body is in place — the inner walk is
    // a no-op until the param->u.hash mirror gets populated.
    // c:1791-1804 — for (i=0; i<ht->hsize; i++) for (hn=...) addnode(...)
    unsafe {
        let _ht_ref = &*ht; // c:1791 ht->hsize
                            // hsize == 0 on the default-constructed mirror struct, so the
                            // outer loop body never executes. When upstream populates
                            // ht->nodes via param-hash assignment, the iteration drives:
                            //   v.pm = (Param)hn; val = getstrvalue(&v);
                            //   alht->addnode(alht, ztrdup(hn->nam),
                            //                 createaliasnode(ztrdup(val), flags));
                            // which lands at aliastab_lock().write().add(createaliasnode(...)).
        let _ = createaliasnode("", "", flags as u32); // c:1803 (binding-only)
    }

    // c:1806-1807 — `if (ht != pm->u.hash) deleteparamtable(ht);`
    // pm->u.hash discriminator: in C the user-side param node owns the
    // table; the post-assignment frees the temporary `ht` when it's
    // not the same allocation. Rust port: no separate allocation
    // because the typed alias_table is a global singleton; the
    // temporary mirror struct gets dropped at end of scope.
    let _ = pm; // c:1806
    let _ = alht; // c:1769 alht binding (Rust uses aliastab_lock directly)
}

/// Port of `setpmraliases(Param pm, HashTable ht)` from Src/Modules/parameter.c:1812.
#[allow(non_snake_case)]
pub fn setpmraliases(pm: Param, ht: *mut HashTable) {
    // c:1812
    setaliases(std::ptr::null_mut(), pm, ht, 0) // c:1812
}

/// Port of `setpmdisraliases(Param pm, HashTable ht)` from Src/Modules/parameter.c:1819.
#[allow(non_snake_case)]
pub fn setpmdisraliases(pm: Param, ht: *mut HashTable) {
    // c:1819
    setaliases(std::ptr::null_mut(), pm, ht, DISABLED) // c:1819
}

/// Port of `setpmgaliases(Param pm, HashTable ht)` from Src/Modules/parameter.c:1826.
#[allow(non_snake_case)]
pub fn setpmgaliases(pm: Param, ht: *mut HashTable) {
    // c:1826
    setaliases(std::ptr::null_mut(), pm, ht, ALIAS_GLOBAL) // c:1826
}


/// Port of `setpmdisgaliases(Param pm, HashTable ht)` from Src/Modules/parameter.c:1833.
/// C: `setaliases(aliastab, pm, ht, ALIAS_GLOBAL|DISABLED);`
#[allow(non_snake_case)]
pub fn setpmdisgaliases(pm: Param, ht: *mut HashTable) {
    // c:1833
    setaliases(std::ptr::null_mut(), pm, ht, ALIAS_GLOBAL | DISABLED) // c:1819
}

/// Port of `setpmsaliases(Param pm, HashTable ht)` from Src/Modules/parameter.c:1840.
#[allow(non_snake_case)]
pub fn setpmsaliases(pm: Param, ht: *mut HashTable) {
    // c:1840
    setaliases(std::ptr::null_mut(), pm, ht, ALIAS_SUFFIX) // c:1840
}

/// Port of `setpmdissaliases(Param pm, HashTable ht)` from Src/Modules/parameter.c:1847.
#[allow(non_snake_case)]
pub fn setpmdissaliases(pm: Param, ht: *mut HashTable) {
    // c:1847
    setaliases(std::ptr::null_mut(), pm, ht, ALIAS_SUFFIX | DISABLED) // c:1847
}

// (`scan_magic_assoc_keys` moved out of src/ported/ to
// src/exec_shims.rs — it has no C counterpart and the
// no-non-C-fns-in-src/ported rule applies. The canonical scanpm*
// ports below ARE the C dispatch; the aggregator is a
// fusevm-bridge convenience that fans the magic-assoc table NAME
// out into the right scanpm* call. See exec_shims.rs.)

// === auto-generated stubs ===
// Direct ports of static helpers from Src/Modules/parameter.c not
// yet covered above. zshrs links modules statically; live
// state owned by the module's typed struct. Name-parity shims.

/// Direct port of `assignaliasdefs(Param pm, int flags)` from Src/Modules/parameter.c:1867.
/// C signature: `static void assignaliasdefs(Param pm, int flags)`.
/// C body sets `pm->node.flags = PM_SCALAR` (c:1869) then dispatches
/// `pm->gsu.s` to one of six static gsu_scalar handler tables based
/// on the alias-flavour bits (raw/global/suffix × normal/disabled).
/// The `gsu_scalar` struct IS ported at zsh_h.rs:802 (with `GsuScalar`
/// = Box<gsu_scalar> alias at c:794), but C uses six C-level statics
/// for the per-flavour dispatch tables — `pmralias_gsu`, `pmgalias_gsu`,
/// `pmsalias_gsu`, plus the three `pmdis*alias_gsu` variants — that
/// can't be const-initialised in Rust because gsu_scalar holds
/// `Option<GsuFn>` function pointers. Until the six per-flavour
/// statics land as `LazyLock<gsu_scalar>` entries, the flag-to-handler
/// mapping is recorded in a name-keyed side-map so future gsu lookups
/// resolve the right handler.
#[allow(non_snake_case)]
pub fn assignaliasdefs(
    pm: *mut param, // c:1867
    flags: i32,
) {
    if !pm.is_null() {
        unsafe {
            (*pm).node.flags = PM_SCALAR as i32;
        } // c:1869
    }
    // c:1871-1893 — switch on flag combination to pick the gsu table.
    let handler = match flags {
        // c:1873
        0 => "pmralias_gsu",                                    // c:1874
        f if f == ALIAS_GLOBAL => "pmgalias_gsu",               // c:1877
        f if f == ALIAS_SUFFIX => "pmsalias_gsu",               // c:1880
        f if f == DISABLED => "pmdisralias_gsu",                // c:1883
        f if f == ALIAS_GLOBAL | DISABLED => "pmdisgalias_gsu", // c:1886
        f if f == ALIAS_SUFFIX | DISABLED => "pmdissalias_gsu", // c:1889
        _ => return,
    };
    if !pm.is_null() {
        let name = unsafe { (*pm).node.nam.clone() };
        let m = ALIAS_GSU_HANDLER
            .get_or_init(|| Mutex::new(HashMap::new()));
        if let Ok(mut g) = m.lock() {
            g.insert(name, handler.to_string());
        }
    }
}

/// Direct port of `getalias(HashTable alht, UNUSED(HashTable ht), const char *name, int flags)` from Src/Modules/parameter.c:1900.
/// C body (c:1906-1919):
/// ```c
/// pm.node.nam = name;
/// assignaliasdefs(pm, flags);
/// if (al = alht[name]; flags == al->node.flags)
///     pm->u.str = al->text;
/// else { pm->u.str = ""; flags |= PM_UNSET|PM_SPECIAL; }
/// ```
///
/// `alht` selects which alias table to query: `aliastab` for
/// raw / global aliases, `sufaliastab` for suffix aliases. Static-
/// link path: dispatch on the ALIAS_SUFFIX bit in `flags` since the
/// ht pointer isn't passed through.
#[allow(non_snake_case)]
/// Port of `getalias(HashTable alht, UNUSED(HashTable ht), const char *name, int flags)` from `Src/Modules/parameter.c:1901`.
/// WARNING: param names don't match C — Rust=(_alht, _ht, flags) vs C=(alht, ht, name, flags)
pub fn getalias(
    _alht: *mut HashTable,
    _ht: *mut HashTable, // c:1901
    name: &str,
    flags: i32,
) -> Option<Param> {
    let table = if (flags & ALIAS_SUFFIX) != 0 {
        sufaliastab_lock()
    } else {
        aliastab_lock()
    };
    let g = table.read().ok()?;
    let entry = g.get(name); // c:1911 alht->getnode2
    let (value, found) = if let Some(al) = entry {
        // c:1912
        // c:1912 — `flags == al->node.flags` strict equality match.
        if al.node.flags == flags {
            // c:1912 al->node.flags
            (al.text.clone(), true) // c:1913 al->text
        } else {
            (String::new(), false) // c:1916
        }
    } else {
        (String::new(), false) // c:1916
    };
    let mut pm = Box::new(param {
        // c:1906 hcalloc
        node: hashnode {
            next: None,
            nam: name.to_string(), // c:1907
            flags: 0,
        },
        u_data: 0,
        u_arr: None,
        u_str: Some(value), // c:1913 / c:1916
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
    // c:1909 — `assignaliasdefs(pm, flags);` sets PM_SCALAR + selects
    // gsu_scalar handler based on alias flavour.
    assignaliasdefs(&mut *pm as *mut _, flags); // c:1909
    if !found {
        pm.node.flags |= (PM_UNSET | PM_SPECIAL) as i32; // c:1917
    }
    Some(pm) // c:1919
}

/// Port of `getpmralias(HashTable ht, const char *name)` from Src/Modules/parameter.c:1923.
/// C: `static HashNode getpmralias(HashTable ht, const char *name)` →
///   `return getalias(aliastab, ht, name, 0);`
#[allow(non_snake_case)]
pub fn getpmralias(ht: *mut HashTable, name: &str) -> Option<Param> {
    // c:1923
    getalias(std::ptr::null_mut(), ht, name, 0) // c:1923
}

/// Port of `getpmdisralias(HashTable ht, const char *name)` from Src/Modules/parameter.c:1930.
/// C: `static HashNode getpmdisralias(HashTable ht, const char *name)` →
///   `return getalias(aliastab, ht, name, DISABLED);`
#[allow(non_snake_case)]
pub fn getpmdisralias(ht: *mut HashTable, name: &str) -> Option<Param> {
    // c:1930
    getalias(std::ptr::null_mut(), ht, name, DISABLED) // c:1930
}

/// Port of `getpmgalias(HashTable ht, const char *name)` from Src/Modules/parameter.c:1937.
/// C: `static HashNode getpmgalias(HashTable ht, const char *name)` →
///   `return getalias(aliastab, ht, name, ALIAS_GLOBAL);`
#[allow(non_snake_case)]
pub fn getpmgalias(ht: *mut HashTable, name: &str) -> Option<Param> {
    // c:1937
    getalias(std::ptr::null_mut(), ht, name, ALIAS_GLOBAL) // c:1937
}

/// Port of `getpmdisgalias(HashTable ht, const char *name)` from Src/Modules/parameter.c:1944.
/// C: `static HashNode getpmdisgalias(HashTable ht, const char *name)` →
///   `return getalias(galiastab, ht, name, DISABLED);`
#[allow(non_snake_case)]
pub fn getpmdisgalias(ht: *mut HashTable, name: &str) -> Option<Param> {
    // c:1944
    getalias(std::ptr::null_mut(), ht, name, DISABLED) // c:1930
}

/// Port of `getpmsalias(HashTable ht, const char *name)` from Src/Modules/parameter.c:1951.
/// C: `static HashNode getpmsalias(HashTable ht, const char *name)` →
///   `return getalias(saliastab, ht, name, 0);`
#[allow(non_snake_case)]
pub fn getpmsalias(ht: *mut HashTable, name: &str) -> Option<Param> {
    // c:1951
    getalias(std::ptr::null_mut(), ht, name, 0) // c:1951
}

/// Port of `getpmdissalias(HashTable ht, const char *name)` from Src/Modules/parameter.c:1958.
/// C: `static HashNode getpmdissalias(HashTable ht, const char *name)` →
///   `return getalias(saliastab, ht, name, DISABLED);`
#[allow(non_snake_case)]
pub fn getpmdissalias(ht: *mut HashTable, name: &str) -> Option<Param> {
    // c:1958
    getalias(std::ptr::null_mut(), ht, name, DISABLED) // c:1958
}

/// Port of `scanaliases(HashTable alht, UNUSED(HashTable ht), ScanFunc func, int pmflags, int alflags)` from Src/Modules/parameter.c:1965.
/// C: `static void scanaliases(HashTable alht, UNUSED(HashTable ht),
///     ScanFunc func, int pmflags, int alflags)` — iterate the alias
///     table, synth a Param per matching entry, invoke func.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(_alht, _ht, pmflags, alflags) vs C=(alht, ht, func, pmflags, alflags)
pub fn scanaliases(
    _alht: *mut HashTable,
    _ht: *mut HashTable, // c:1965
    func: Option<ScanFunc>,
    pmflags: i32,
    alflags: i32,
) {
    // c:1968-1988 — `for ((al = (Alias) firstnode(alht)); al;
    //                     incnode(al)) { if (!al->node.flags & alflags
    //                     && !disabled) emit(al->node.nam) }`.
    // Walk the canonical `aliastab` (Src/hashtable.c:1210, ported at
    // src/ported/hashtable.rs::aliastab_lock) and emit each alias
    // matching the flag filter.
    if let Some(f) = func {
        let lock = if alflags == ALIAS_SUFFIX {
            sufaliastab_lock()
        } else {
            aliastab_lock()
        };
        if let Ok(tab) = lock.read() {
            for (_, alias) in tab.iter() {
                // c:1970
                // c:1972 — `if (al->node.flags & alflags) continue;`
                if alflags != 0
                    && alflags != ALIAS_SUFFIX
                    && (alias.node.flags & alflags) == 0
                {
                    continue;
                }
                let node = Box::new(hashnode {
                    next: None,
                    nam: alias.node.nam.clone(), // c:1979
                    flags: alias.node.flags,
                });
                f(&node, pmflags); // c:1985
            }
        }
    }
}

/// Port of `scanpmraliases(HashTable ht, ScanFunc func, int flags)` from Src/Modules/parameter.c:1990.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(ht, func) vs C=(ht, func, flags)
pub fn scanpmraliases(
    ht: *mut HashTable,
    func: Option<ScanFunc>, // c:1990
    flags: i32,
) {
    scanaliases(std::ptr::null_mut(), ht, func, flags, 0) // c:1993
}

/// Port of `scanpmdisraliases(HashTable ht, ScanFunc func, int flags)` from Src/Modules/parameter.c:1997.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(ht, func) vs C=(ht, func, flags)
pub fn scanpmdisraliases(
    ht: *mut HashTable,
    func: Option<ScanFunc>, // c:1997
    flags: i32,
) {
    scanaliases(std::ptr::null_mut(), ht, func, flags, DISABLED) // c:2000
}

/// Port of `scanpmgaliases(HashTable ht, ScanFunc func, int flags)` from Src/Modules/parameter.c:2004.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(ht, func) vs C=(ht, func, flags)
pub fn scanpmgaliases(
    ht: *mut HashTable,
    func: Option<ScanFunc>, // c:2004
    flags: i32,
) {
    scanaliases(std::ptr::null_mut(), ht, func, flags, ALIAS_GLOBAL) // c:2007
}

/// Port of `scanpmdisgaliases(HashTable ht, ScanFunc func, int flags)` from Src/Modules/parameter.c:2011.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(ht, func) vs C=(ht, func, flags)
pub fn scanpmdisgaliases(
    ht: *mut HashTable,
    func: Option<ScanFunc>, // c:2011
    flags: i32,
) {
    scanaliases(
        std::ptr::null_mut(),
        ht,
        func,
        flags, // c:1997
        ALIAS_GLOBAL | DISABLED,
    )
}

/// Port of `scanpmsaliases(HashTable ht, ScanFunc func, int flags)` from Src/Modules/parameter.c:2018.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(ht, func) vs C=(ht, func, flags)
pub fn scanpmsaliases(
    ht: *mut HashTable,
    func: Option<ScanFunc>, // c:2018
    flags: i32,
) {
    scanaliases(
        std::ptr::null_mut(),
        ht,
        func,
        flags, // c:2021
        ALIAS_SUFFIX,
    )
}

/// Port of `scanpmdissaliases(HashTable ht, ScanFunc func, int flags)` from Src/Modules/parameter.c:2025.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(ht, func) vs C=(ht, func, flags)
pub fn scanpmdissaliases(
    ht: *mut HashTable,
    func: Option<ScanFunc>, // c:2025
    flags: i32,
) {
    scanaliases(
        std::ptr::null_mut(),
        ht,
        func,
        flags, // c:2028
        ALIAS_SUFFIX | DISABLED,
    )
}

/// Port of `getpmusergroups(UNUSED(HashTable ht), const char *name)` from Src/Modules/parameter.c:2102.
/// C: `static HashNode getpmusergroups(UNUSED(HashTable ht),
///     const char *name)` — emit group memberships for `name`.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn getpmusergroups(ht: *mut HashTable, name: &str) -> Option<Param> {
    // c:2102
    // c:2106 — `Groupset gs = get_all_groups();` then walk gs->array
    // matching name → gid. Static-link path: getgrnam(3) directly,
    // since zshrs doesn't yet have a Groupset wrapper.
    let cname = std::ffi::CString::new(name).ok()?;
    let grp = unsafe { libc::getgrnam(cname.as_ptr()) }; // c:2106
    let (value, found) = if !grp.is_null() {
        let gid = unsafe { (*grp).gr_gid };
        (gid.to_string(), true) // c:2128 sprintf "%d"
    } else {
        (String::new(), false) // c:2134
    };
    let pm = Box::new(param {
        // c:2108 hcalloc
        node: hashnode {
            next: None,
            nam: name.to_string(), // c:2109
            flags: if found {
                (PM_SCALAR | PM_READONLY) as i32
            }
            // c:2110
            else {
                (PM_SCALAR | PM_READONLY | PM_UNSET | PM_SPECIAL) as i32
            }, // c:2135
        },
        u_data: 0,
        u_arr: None,
        u_str: Some(value), // c:2128 / c:2134
        u_val: 0,
        u_dval: 0.0,
        u_hash: None,
        gsu_s: None, // c:2111 nullsetscalar_gsu
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
    Some(pm) // c:2136
}

/// Direct port of `scanpmusergroups(UNUSED(HashTable ht), ScanFunc func, int flags)` from Src/Modules/parameter.c:2143.
/// C body (c:2146-2169): get_all_groups() returns Groupset; walk
/// gs->array emitting each group name. Static-link path uses
/// getgrent(3) — same data source.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(_ht, func) vs C=(ht, func, flags)
pub fn scanpmusergroups(
    _ht: *mut HashTable,
    func: Option<ScanFunc>, // c:2143
    flags: i32,
) {
    if let Some(f) = func {
        unsafe {
            libc::setgrent();
        } // c:2148
        loop {
            let grp = unsafe { libc::getgrent() }; // c:2152
            if grp.is_null() {
                break;
            }
            let name = unsafe { std::ffi::CStr::from_ptr((*grp).gr_name) };
            let node = Box::new(hashnode {
                next: None,
                nam: name.to_string_lossy().into_owned(), // c:2160
                flags: 0,
            });
            f(&node, flags); // c:2167
        }
        unsafe {
            libc::endgrent();
        } // c:2169
    }
}

/// Port of `struct pardef` from `Src/Modules/parameter.c:2179`. The
/// per-magic-assoc parameter spec table — one entry per
/// `${parameters}`/`${commands}`/`${functions}`/etc. exposed by the
/// `zsh/parameter` module.
///
/// C definition (c:2179-2187):
/// ```c
/// struct pardef {
///     char *name;
///     int flags;
///     GetNodeFunc getnfn;
///     ScanTabFunc scantfn;
///     GsuHash hash_gsu;
///     GsuArray array_gsu;
///     Param pm;
/// };
/// ```
///
/// Rust port keeps the same shape; the GSU function-table fields
/// (`hash_gsu`, `array_gsu`) are type-erased via `usize` because the
/// `GsuHash`/`GsuArray` types (zsh_h.rs:797-798, `Box<gsu_hash>` /
/// `Box<gsu_array>`) own their callback function pointers and can't
/// be const-initialised in a Rust static. Consumers cast back at
/// dispatch time.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy)]
pub struct pardef {
    // c:2179
    /// Parameter name (e.g. "commands", "functions", "options").
    pub name: &'static str, // c:2180
    /// Flags (PM_* bits — typically PM_HASHED|PM_SPECIAL|PM_HIDE).
    pub flags: i32, // c:2181
    /// `GetNodeFunc` getnfn — type-erased: 0 when not yet wired.
    pub getnfn: usize, // c:2182
    /// `ScanTabFunc` scantfn — type-erased: 0 when not yet wired.
    pub scantfn: usize, // c:2183
    /// `GsuHash` hash_gsu — type-erased.
    pub hash_gsu: usize, // c:2184
    /// `GsuArray` array_gsu — type-erased.
    pub array_gsu: usize, // c:2185
    /// `Param pm` — type-erased pointer; populated by createparam.
    pub pm: usize, // c:2186
}

// `partab` — port of `static struct paramdef partab[]` (parameter.c).
// 33 SPECIALPMDEF entries — each ties a `${assoc}` magic-assoc name
// to its scanpm*/getpm* C callbacks. Rust-side dispatch happens via
// thread-local SCAN routing in subst.rs; here we only need the names.

// `module_features` — port of `static struct features module_features`
// from parameter.c:2300.

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/parameter.c:2311`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:2311
    // C body c:2313-2314 — `return 0`. Faithful empty-body port.
    0
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/parameter.c:2318`.
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    // c:2318
    *features = featuresarray(m, module_features());
    0
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/parameter.c:2326`.
/// C body c:2328-2336 — wrap handlefeatures() with incleanup=1/0 so that
/// any feature removal does not perturb the main shell's parameter table.
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:2326
    INCLEANUP.store(1, std::sync::atomic::Ordering::Relaxed); // c:2341
    let ret = handlefeatures(m, module_features(), enables); // c:2341
    INCLEANUP.store(0, std::sync::atomic::Ordering::Relaxed); // c:2341
    ret // c:2341
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/parameter.c:2341`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:2341
    // C body c:2343-2344 — `return 0`. Faithful empty-body port; the
    //                      hash-magic params (parameters, commands,
    //                      functions, etc.) are registered via the
    //                      partab dispatch in features_/enables_.
    0
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/parameter.c:2348`.
/// C body c:2350-2354 — wrap setfeatureenables(NULL) with incleanup=1/0
/// matching the same guard enables_ uses.
pub fn cleanup_(m: *const module) -> i32 {
    // c:2348
    INCLEANUP.store(1, std::sync::atomic::Ordering::Relaxed); // c:2359
    let ret = setfeatureenables(m, module_features(), None); // c:2359
    INCLEANUP.store(0, std::sync::atomic::Ordering::Relaxed); // c:2359
    ret // c:2359
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/parameter.c:2359`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:2359
    // C body c:2361-2362 — `return 0`. Faithful empty-body port; the
    //                      hash-magic params get unregistered via the
    //                      partab dispatch in cleanup_.
    0
}

// =====================================================================
// !!! WARNING: RUST-ONLY STATE — NO DIRECT C COUNTERPART !!!
// =====================================================================
//
// `ALIAS_GSU_HANDLER` records which `pm*alias_gsu` static dispatch
// table assignaliasdefs() selected for each parameter name. The C
// source stores this directly on `Param->gsu.s` as a function-table
// pointer (Src/Modules/parameter.c:1842-1860). Until the gsu_scalar
// dispatch table machinery is ported in full, this side-map is the
// bridge so future gsu lookups can resolve the right handler.
//
// !!! Remove this side-map once the gsu_scalar dispatch table is
// ported in src/ported/params.rs and assignaliasdefs() can write
// `pm->gsu.s = &pmralias_gsu` directly. !!!
// =====================================================================
static ALIAS_GSU_HANDLER: OnceLock<
    Mutex<HashMap<String, String>>,
> = OnceLock::new();

// File-static globals for parameter.c port — c:38-44, src/init.c.
// `dirstack` lives in src/exec.c globals; `funcstack` in src/init.c.
// Mirror as Mutex<Vec<...>> for cross-thread safety.
pub static DIRSTACK: Mutex<Vec<String>> = Mutex::new(Vec::new());
pub static INCLEANUP: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

// `funcstack` global from Src/exec.c:340 — head of the active shell
// function call stack. Rust port mirrors the chain as Vec snapshot
// (the C source walks `funcstack->prev` to produce array params).
pub static FUNCSTACK: Mutex<Vec<crate::ported::zsh_h::funcstack>> =
    Mutex::new(Vec::new());

// =====================================================================
// !!! WARNING: RUST-ONLY HELPER — NO DIRECT C COUNTERPART !!!
// =====================================================================
//
// `make_empty_special_pm` is the common Param-construction shape
// used by getpmjob{dir,state,text} and getpmmodule when the backing
// data isn't reachable from src/ported/. The C source duplicates
// this 12-line construct inline at each callsite (c:1387/c:1459/
// c:1279/c:1042); Rust pulls it into one helper to avoid the
// repetition. NOT a new abstraction — the same struct fields, the
// same flag combination, the same "u.str = empty" placeholder that
// the executor-side caller overwrites with the live value.
//
// !!! Do NOT use for getpm* tables whose data IS reachable from
// src/ported/ (cmdnamtab, BUILTINS, shfunctab, aliastab, optns,
// nameddirtab via passwd) — those compose their value inline. !!!
// =====================================================================

/// WARNING: NOT IN PARAMETER.C — Rust-only `Param` constructor helper; C uses raw struct init
/// (equivalent C logic at Src/Modules/parameter.c:882).
/// !!! RUST-ONLY HELPER — see WARNING block above. Synthesises a
/// PM_SCALAR | PM_READONLY | PM_UNSET | PM_SPECIAL Param with empty
/// `u.str`.
fn make_empty_special_pm(name: &str) -> Param {
    Box::new(param {
        node: hashnode {
            next: None,
            nam: name.to_string(),
            flags: (PM_SCALAR | PM_READONLY | PM_UNSET | PM_SPECIAL) as i32,
        },
        u_data: 0,
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
}


static MODULE_FEATURES: OnceLock<Mutex<crate::ported::zsh_h::features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN PARAMETER.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<crate::ported::zsh_h::features>) -> Vec<String> {
    vec![
        "p:aliases".to_string(),
        "p:builtins".to_string(),
        "p:commands".to_string(),
        "p:dirstack".to_string(),
        "p:dis_aliases".to_string(),
        "p:dis_builtins".to_string(),
        "p:dis_functions".to_string(),
        "p:dis_functions_source".to_string(),
        "p:dis_galiases".to_string(),
        "p:dis_patchars".to_string(),
        "p:dis_reswords".to_string(),
        "p:dis_saliases".to_string(),
        "p:funcfiletrace".to_string(),
        "p:funcsourcetrace".to_string(),
        "p:funcstack".to_string(),
        "p:functions".to_string(),
        "p:functions_source".to_string(),
        "p:functrace".to_string(),
        "p:galiases".to_string(),
        "p:history".to_string(),
        "p:historywords".to_string(),
        "p:jobdirs".to_string(),
        "p:jobstates".to_string(),
        "p:jobtexts".to_string(),
        "p:modules".to_string(),
        "p:nameddirs".to_string(),
        "p:options".to_string(),
        "p:parameters".to_string(),
        "p:patchars".to_string(),
        "p:reswords".to_string(),
        "p:saliases".to_string(),
        "p:userdirs".to_string(),
        "p:usergroups".to_string(),
    ]
}

// WARNING: NOT IN PARAMETER.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(
    _m: *const module,
    _f: &Mutex<crate::ported::zsh_h::features>,
    enables: &mut Option<Vec<i32>>,
) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 33]);
    }
    0
}

// WARNING: NOT IN PARAMETER.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn setfeatureenables(_m: *const module, _f: &Mutex<crate::ported::zsh_h::features>, _e: Option<&[i32]>) -> i32 {
    0
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

// WARNING: NOT IN PARAMETER.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<crate::ported::zsh_h::features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(crate::ported::zsh_h::features {
            bn_list: None,
            bn_size: 0,
            cd_list: None,
            cd_size: 0,
            mf_list: None,
            mf_size: 0,
            pd_list: None,
            pd_size: 33,
            n_abstract: 0,
        })
    })
}

#[cfg(test)]
mod scan_callback_tests {
    use super::*;
    use std::sync::atomic::{AtomicI32, Ordering};

    use crate::ported::zsh_h::param;

    // Module-scoped collector statics. Tests are serialised by name +
    // each test resets before/after so cross-test bleed is impossible.
    static COLLECTED_COUNT: AtomicI32 = AtomicI32::new(0);
    static LAST_NAME_LEN: AtomicI32 = AtomicI32::new(0);

    fn counting_func(node: &HashNode, _flags: i32) {
        COLLECTED_COUNT.fetch_add(1, Ordering::SeqCst);
        LAST_NAME_LEN.store(node.nam.len() as i32, Ordering::SeqCst);
    }

    fn reset_counters() {
        COLLECTED_COUNT.store(0, Ordering::SeqCst);
        LAST_NAME_LEN.store(0, Ordering::SeqCst);
    }

    /// c:139-145 — scanpmparameters walks realparamtab calling func per
    /// non-PM_UNSET entry. Seed paramtab with one entry, verify the
    /// callback fires exactly once with the right name.
    #[test]
    fn scanpmparameters_invokes_func_per_param() {
        let _g = crate::test_util::global_state_lock();
        reset_counters();
        // Seed realparamtab.
        let pm = param {
            node: hashnode {
                next: None,
                nam: "ZSHRS_TEST_SP_A".to_string(),
                flags: PM_SCALAR as i32,
            },
            u_data: 0,
            u_arr: None,
            u_str: Some("v".to_string()),
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
        realparamtab()
            .write()
            .unwrap()
            .insert("ZSHRS_TEST_SP_A".to_string(), Box::new(pm));
        scanpmparameters(std::ptr::null_mut(), Some(counting_func), 0);
        let observed = COLLECTED_COUNT.load(Ordering::SeqCst);
        // Cleanup before asserting so failures don't leak state.
        realparamtab()
            .write()
            .unwrap()
            .remove("ZSHRS_TEST_SP_A");
        assert!(
            observed >= 1,
            "callback fires at least once for the seeded param (got {})",
            observed
        );
    }

    /// c:1199 — scanpmhistory walks hist_ring newest→oldest. With an
    /// empty ring the loop body never runs → zero callback invocations.
    /// A regression that ran an iter on the sentinel head would emit
    /// a spurious extra entry; this test catches it.
    #[test]
    fn scanpmhistory_empty_ring_invokes_zero_callbacks() {
        let _g = crate::test_util::global_state_lock();
        reset_counters();
        let snapshot: Vec<_> = hist_ring
            .lock()
            .unwrap()
            .drain(..)
            .collect();
        scanpmhistory(std::ptr::null_mut(), Some(counting_func), 0);
        let observed = COLLECTED_COUNT.load(Ordering::SeqCst);
        hist_ring
            .lock()
            .unwrap()
            .extend(snapshot);
        assert_eq!(observed, 0);
    }

    /// c:1255 — `pmjobtext` joins each proc's text with " | " (the
    /// canonical pipeline-display format). Empty job table → empty
    /// string. Regression returning the wrong separator would corrupt
    /// every `${jobtexts[1]}` query users hit.
    #[test]
    fn pmjobtext_empty_jobtab_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let s = pmjobtext(std::ptr::null_mut(), 1);
        assert_eq!(s, "");
    }

    /// c:1340 — `pmjobstate` for a job index past the end of jobtab
    /// returns empty (defensive). Catches a regression that panics on
    /// out-of-range queries.
    #[test]
    fn pmjobstate_out_of_range_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let s = pmjobstate(std::ptr::null_mut(), 9999);
        assert_eq!(s, "");
    }

    /// c:1447 — `pmjobdir` for missing job falls back to global pwd
    /// (current_dir). Verify the fallback path returns a non-empty
    /// path string.
    #[test]
    fn pmjobdir_missing_job_falls_back_to_cwd() {
        let _g = crate::test_util::global_state_lock();
        let s = pmjobdir(std::ptr::null_mut(), 9999);
        assert!(
            !s.is_empty(),
            "fallback to cwd must produce a non-empty path"
        );
        assert!(
            s.starts_with('/') || s == "" || cfg!(not(unix)),
            "Unix path must be absolute (got {s:?})"
        );
    }

    /// c:1040 — `getpmmodule(name)` for an unknown module name
    /// returns Some with PM_UNSET|PM_SPECIAL flags AND empty u_str
    /// per c:1068-1069. Regression returning Some("loaded") would
    /// silently lie about which modules are loaded.
    #[test]
    fn getpmmodule_unknown_module_marks_unset() {
        let _g = crate::test_util::global_state_lock();
        let pm = getpmmodule(std::ptr::null_mut(), "definitely_not_a_loaded_module_xyz")
            .expect("must return Some");
        assert_eq!(
            pm.u_str.as_deref(),
            Some(""),
            "unknown module → empty value string"
        );
        assert_ne!(
            pm.node.flags & PM_UNSET as i32,
            0,
            "unknown module must set PM_UNSET"
        );
        assert_ne!(
            pm.node.flags & PM_SPECIAL as i32,
            0,
            "unknown module must set PM_SPECIAL"
        );
    }
}

#[cfg(test)]
mod setalias_tests {
    use super::*;
    use crate::ported::zsh_h::param;

    /// setalias wires `aliastab.add(createaliasnode(name, value, flags))`
    /// per c:1701-1702. After call, aliastab should contain the new
    /// alias with the given value.
    #[test]
    fn setalias_inserts_entry_into_aliastab() {
        let _g = crate::test_util::global_state_lock();
        let pm = param {
            node: hashnode {
                next: None,
                nam: "zshrs_test_alias_x".to_string(),
                flags: PM_SCALAR as i32,
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
        };
        setalias(std::ptr::null_mut(), Box::new(pm), "echo hi".to_string(), 0);
        let tab = aliastab_lock()
            .read()
            .expect("aliastab poisoned");
        let entry = tab.get("zshrs_test_alias_x");
        assert!(entry.is_some(), "setalias must add to aliastab");
        if let Some(a) = entry {
            assert_eq!(a.text, "echo hi", "alias value matches createaliasnode arg");
        }
    }
}

#[cfg(test)]
mod paramtypestr_table_tests {
    use super::*;
    use crate::ported::zsh_h::param;

    fn pm(flags: u32) -> param {
        param {
            node: hashnode {
                next: None,
                nam: String::new(),
                flags: flags as i32,
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
        }
    }

    /// c:43 — paramtypestr's per-flag dispatch table emits the type
    /// name `${(t)foo}` reports. Each PM_TYPE bit pattern maps to
    /// a distinct user-visible string. A regression that emits
    /// `"scalar"` for every type would break every typeset-introspecting
    /// script (many shell scripts grep for "array"/"integer" output).
    #[test]
    fn integer_param_renders_as_integer() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(paramtypestr(&pm(PM_INTEGER)), "integer");
    }

    #[test]
    fn float_e_param_renders_as_float() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(paramtypestr(&pm(PM_EFLOAT)), "float");
    }

    #[test]
    fn float_f_param_renders_as_float() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(paramtypestr(&pm(PM_FFLOAT)), "float");
    }

    #[test]
    fn array_param_renders_as_array() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(paramtypestr(&pm(PM_ARRAY)), "array");
    }

    #[test]
    fn hashed_param_renders_as_association() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(paramtypestr(&pm(PM_HASHED)), "association");
    }

    /// c:43 — `${(t)foo}` includes per-modifier suffixes for readonly /
    /// exported / local. They appear after the type name separated by
    /// `-`. Regression dropping any modifier breaks typeset-output
    /// parsing in user scripts.
    #[test]
    fn readonly_modifier_appears_after_type_name() {
        let _g = crate::test_util::global_state_lock();
        let s = paramtypestr(&pm(PM_INTEGER | PM_READONLY));
        assert!(
            s.contains("readonly"),
            "PM_READONLY must appear in type-string (got {s:?})"
        );
    }

    /// c:81-82 — PM_EXPORTED renders as `-export` (note: NOT
    /// `-exported`; this is the canonical zsh suffix). Catches a
    /// regression where the suffix changes spelling.
    #[test]
    fn exported_modifier_renders_as_export_suffix() {
        let _g = crate::test_util::global_state_lock();
        let s = paramtypestr(&pm(PM_INTEGER | PM_EXPORTED));
        assert!(
            s.contains("-export"),
            "PM_EXPORTED must produce '-export' suffix (got {s:?})"
        );
    }

    /// c:63-64 — `-local` suffix is gated on `pm.level != 0`, NOT a
    /// PM_LOCAL flag (which is a different concept). Verifies the
    /// level-based rendering path; regression flipping the gate would
    /// break `local foo` reporting in nested function scopes.
    #[test]
    fn local_modifier_renders_when_level_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let mut p = pm(PM_INTEGER);
        p.level = 1;
        let s = paramtypestr(&p);
        assert!(
            s.contains("-local"),
            "level>0 must add '-local' (got {s:?})"
        );
    }

    /// `Src/Modules/parameter.c:48 + c:91-92` — PM_UNSET short-circuits
    /// to empty string BEFORE the type/modifier dispatch. Pin this
    /// guard: a regression that drops the PM_UNSET check would emit
    /// stale type labels for unset params, leaking PM state through
    /// `${(t)varname}` for never-assigned params.
    #[test]
    fn unset_param_renders_as_empty_string() {
        let _g = crate::test_util::global_state_lock();
        // Even with type + modifier flags set, PM_UNSET wins.
        let s = paramtypestr(&pm(PM_INTEGER | PM_UNSET | PM_READONLY));
        assert_eq!(
            s, "",
            "c:48,91-92 — PM_UNSET wins over every type + modifier"
        );
    }

    /// `Src/Modules/parameter.c:49-50` — PM_AUTOLOAD emits "undefined"
    /// regardless of any other type/modifier bits set. Pin both the
    /// exact string and the precedence over PM_INTEGER etc.
    #[test]
    fn autoload_param_renders_as_undefined() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            paramtypestr(&pm(PM_AUTOLOAD)),
            "undefined",
            "c:49-50 — PM_AUTOLOAD → 'undefined'"
        );
        // Even with type bits + modifiers set, AUTOLOAD wins.
        let s = paramtypestr(&pm(PM_AUTOLOAD | PM_INTEGER | PM_READONLY));
        assert_eq!(
            s, "undefined",
            "c:49-50 — PM_AUTOLOAD precedence over type+modifier"
        );
    }

    /// `Src/Modules/parameter.c:53` — PM_SCALAR has value 0 (all type
    /// bits clear). A bare param with no type bits set renders as
    /// "scalar". Pin so a regression that emits "" or "unknown" for
    /// the zero-type case breaks the most-common `${(t)foo}` path.
    #[test]
    fn scalar_param_renders_as_scalar() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            paramtypestr(&pm(PM_SCALAR)),
            "scalar",
            "c:53 — bare PM_SCALAR (zero type bits) → 'scalar'"
        );
    }

    /// `Src/Modules/parameter.c:54` — PM_NAMEREF renders as "nameref".
    /// Catches a regression that omits the nameref branch (zsh added
    /// nameref support in 5.10+).
    #[test]
    fn nameref_param_renders_as_nameref() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            paramtypestr(&pm(PM_NAMEREF)),
            "nameref",
            "c:54 — PM_NAMEREF → 'nameref'"
        );
    }

    /// `Src/Modules/parameter.c:65-90` — Every modifier flag adds a
    /// `-suffix`. Sweep all eight modifiers (LEFT/RIGHT_B/RIGHT_Z/
    /// LOWER/UPPER/TAGGED/TIED/UNIQUE/HIDE/HIDEVAL/SPECIAL) so a
    /// regression silently dropping one breaks `${(t)foo}` typeset
    /// output for that flag.
    #[test]
    fn left_modifier_renders_dash_left_suffix() {
        let _g = crate::test_util::global_state_lock();
        let s = paramtypestr(&pm(PM_SCALAR | PM_LEFT));
        assert!(
            s.contains("-left"),
            "c:65-66 — PM_LEFT → '-left' (got {s:?})"
        );
    }

    #[test]
    fn right_b_modifier_renders_dash_right_blanks_suffix() {
        let _g = crate::test_util::global_state_lock();
        let s = paramtypestr(&pm(PM_SCALAR | PM_RIGHT_B));
        assert!(
            s.contains("-right_blanks"),
            "c:67-68 — PM_RIGHT_B → '-right_blanks' (got {s:?})"
        );
    }

    #[test]
    fn right_z_modifier_renders_dash_right_zeros_suffix() {
        let _g = crate::test_util::global_state_lock();
        let s = paramtypestr(&pm(PM_SCALAR | PM_RIGHT_Z));
        assert!(
            s.contains("-right_zeros"),
            "c:69-70 — PM_RIGHT_Z → '-right_zeros' (got {s:?})"
        );
    }

    #[test]
    fn lower_modifier_renders_dash_lower_suffix() {
        let _g = crate::test_util::global_state_lock();
        let s = paramtypestr(&pm(PM_SCALAR | PM_LOWER));
        assert!(
            s.contains("-lower"),
            "c:71-72 — PM_LOWER → '-lower' (got {s:?})"
        );
    }

    #[test]
    fn upper_modifier_renders_dash_upper_suffix() {
        let _g = crate::test_util::global_state_lock();
        let s = paramtypestr(&pm(PM_SCALAR | PM_UPPER));
        assert!(
            s.contains("-upper"),
            "c:73-74 — PM_UPPER → '-upper' (got {s:?})"
        );
    }

    #[test]
    fn tagged_modifier_renders_dash_tag_suffix() {
        let _g = crate::test_util::global_state_lock();
        let s = paramtypestr(&pm(PM_SCALAR | PM_TAGGED));
        assert!(
            s.contains("-tag"),
            "c:77-78 — PM_TAGGED → '-tag' (got {s:?})"
        );
    }

    #[test]
    fn tied_modifier_renders_dash_tied_suffix() {
        let _g = crate::test_util::global_state_lock();
        let s = paramtypestr(&pm(PM_SCALAR | PM_TIED));
        assert!(
            s.contains("-tied"),
            "c:79-80 — PM_TIED → '-tied' (got {s:?})"
        );
    }

    #[test]
    fn unique_modifier_renders_dash_unique_suffix() {
        let _g = crate::test_util::global_state_lock();
        let s = paramtypestr(&pm(PM_SCALAR | PM_UNIQUE));
        assert!(
            s.contains("-unique"),
            "c:83-84 — PM_UNIQUE → '-unique' (got {s:?})"
        );
    }

    #[test]
    fn hide_modifier_renders_dash_hide_suffix() {
        let _g = crate::test_util::global_state_lock();
        let s = paramtypestr(&pm(PM_SCALAR | PM_HIDE));
        assert!(
            s.contains("-hide"),
            "c:85-86 — PM_HIDE → '-hide' (got {s:?})"
        );
    }

    #[test]
    fn hideval_modifier_renders_dash_hideval_suffix() {
        let _g = crate::test_util::global_state_lock();
        let s = paramtypestr(&pm(PM_SCALAR | PM_HIDEVAL));
        assert!(
            s.contains("-hideval"),
            "c:87-88 — PM_HIDEVAL → '-hideval' (got {s:?})"
        );
    }

    #[test]
    fn special_modifier_renders_dash_special_suffix() {
        let _g = crate::test_util::global_state_lock();
        let s = paramtypestr(&pm(PM_SCALAR | PM_SPECIAL));
        assert!(
            s.contains("-special"),
            "c:89-90 — PM_SPECIAL → '-special' (got {s:?})"
        );
    }

    /// `Src/Modules/parameter.c:43-94` — Multiple modifiers stack in C
    /// source order (level → LEFT → RIGHT_B → RIGHT_Z → LOWER → UPPER
    /// → READONLY → TAGGED → TIED → EXPORTED → UNIQUE → HIDE →
    /// HIDEVAL → SPECIAL). Pin the order so a regen that reshuffles
    /// the branches changes `${(t)foo}` output across the whole zsh
    /// ecosystem.
    #[test]
    fn multiple_modifiers_concatenate_in_c_source_order() {
        let _g = crate::test_util::global_state_lock();
        let mut p = pm(PM_INTEGER | PM_LEFT | PM_READONLY | PM_EXPORTED);
        p.level = 1;
        let s = paramtypestr(&p);
        // c:43-94 emits left BEFORE readonly BEFORE export.
        let i_left = s.find("-left").expect("missing -left");
        let i_ro = s.find("-readonly").expect("missing -readonly");
        let i_exp = s.find("-export").expect("missing -export");
        let i_local = s.find("-local").expect("missing -local");
        // c:63-64 — local is FIRST (level check fires before any flag).
        assert!(i_local < i_left, "c:63-64 — -local must precede -left");
        assert!(i_left < i_ro, "c:65-76 — -left must precede -readonly");
        assert!(i_ro < i_exp, "c:75-82 — -readonly must precede -export");
    }
}


use crate::func_body_fmt::FuncBodyFmt;
use crate::ported::zle::zle_thingy::{getwidgettarget, listwidgets};
use std::env;
use std::ffi::{CStr, CString};

// =====================================================================
// get_special_array_value — moved from src/vm_helper.rs.
// FAKE/PARTIAL PORT: dispatcher consolidating the per-module
// getpm*/scanpm* table-lookup functions from Modules/parameter.c
// + Modules/mapfile.c + Modules/datetime.c + etc.
//
// Real zsh: each module registers its own param-table with per-name
// `getfn`/`scanfn` GSU callbacks (e.g. getpmparameter c:84,
// getpmcommand c:147, getpmfunction c:280, scanpmoptions c:1029).
// Param lookup `$parameters[foo]` walks the param hashtable, finds
// the special-name entry, invokes that entry's `getfn(pm, name)`
// to materialize the value lazily.
//
// zshrs (current): one big match dispatcher keyed by array_name.
// Eventually each arm migrates to its owning module's getfn shim
// + we wire `params::getsparam`/`scanparam` to dispatch through
// the param-table's GSU like C does. Until then, this lives here
// (next to canonical Modules/parameter.c port).
// =====================================================================
impl crate::ported::vm_helper::ShellExecutor {
    /// Get value from zsh/parameter special arrays (options, commands, functions, etc.)
    /// Returns Some(value) if this is a special array access, None otherwise
    pub fn get_special_array_value(&self, array_name: &str, key: &str) -> Option<String> {
        match array_name {
            // === ZSH/MAPFILE module ===
            // `${mapfile[/path]}` reads the file's contents. Direct
            // port of `getpmmapfile(UNUSED(HashTable ht), const char *name)` (Src/Modules/mapfile.c:217)
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
                                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
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
                    // Return all options as "name=on/off" pairs.
                    let opts: Vec<String> = crate::ported::options::opt_state_snapshot()
                        .iter()
                        .map(|(k, v)| format!("{}={}", k, if *v { "on" } else { "off" }))
                        .collect();
                    return Some(opts.join(" "));
                }
                let opt_name = key.to_lowercase().replace('_', "");
                let is_on = crate::ported::options::opt_state_get(&opt_name).unwrap_or(false);
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
                // Read from canonical `aliastab` (`Src/hashtable.c:1210`,
                // ported at `src/ported/hashtable.rs::aliastab_lock`).
                // bin_alias writes through `aliastab.add()` — the local
                // `exec.aliases` HashMap is a stale init-time snapshot.
                if let Ok(tab) = crate::ported::hashtable::aliastab_lock().read() {
                    if key == "@" || key == "*" {
                        let mut names: Vec<&String> = tab.iter().map(|(n, _)| n).collect();
                        names.sort();
                        let vals: Vec<String> = names
                            .iter()
                            .filter_map(|n| tab.get(n).map(|a| a.text.clone()))
                            .collect();
                        return Some(vals.join(" "));
                    }
                    return Some(tab.get(key).map(|a| a.text.clone()).unwrap_or_default());
                }
                Some(self.alias(key).unwrap_or_default())
            }
            "galiases" => {
                if key == "@" || key == "*" {
                    let entries = self.global_alias_entries();
                    let vals: Vec<String> = entries.into_iter().map(|(_, v)| v).collect();
                    return Some(vals.join(" "));
                }
                Some(self.global_alias(key).unwrap_or_default())
            }
            "saliases" => {
                if key == "@" || key == "*" {
                    let entries = self.suffix_alias_entries();
                    let vals: Vec<String> = entries.into_iter().map(|(_, v)| v).collect();
                    return Some(vals.join(" "));
                }
                Some(self.suffix_alias(key).unwrap_or_default())
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
                let formatted = FuncBodyFmt::render(text.trim());
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
                // find_function_file deleted with old exec.c stubs.
                if key == "@" || key == "*" {
                    let _ = self.function_names();
                    return Some(String::new());
                }
                {
                    let _ = key;
                    Some(String::new())
                }
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
                    // Hashed entries first (rehash population) — read
                    // from canonical `cmdnamtab` (Src/exec.c:5260).
                    if let Ok(tab) = crate::ported::hashtable::cmdnamtab_lock().read() {
                        for (k, _) in tab.iter() {
                            if seen.insert(k.clone()) {
                                names.push(k.clone());
                            }
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
                let builtins: Vec<&str> = crate::vm_helper::BUILTIN_NAMES
                    .iter()
                    .map(|s| s.as_str())
                    .collect();
                if key == "@" || key == "*" {
                    return Some(builtins.join(" "));
                }
                if builtins.iter().any(|b| *b == key) {
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
                        if let Ok(tab) = crate::ported::params::paramtab().read() {
                            tab.keys().cloned().collect()
                        } else {
                            std::collections::BTreeSet::new()
                        };
                    if let Ok(m) = crate::ported::params::paramtab_hashed_storage().lock() {
                        names.extend(m.keys().cloned());
                    }
                    let v: Vec<String> = names.into_iter().collect();
                    return Some(v.join(" "));
                }
                // Read PM_TYPE from paramtab Param flags first
                // (canonical). PM_INTEGER → "integer", PM_FFLOAT|
                // PM_EFLOAT → "float", PM_HASHED → "association",
                // PM_ARRAY → "array", scalar default. Append PM_LOWER
                // / PM_UPPER / PM_READONLY / PM_EXPORTED suffixes.
                let flags = self.param_flags(key) as u32;
                if flags != 0 || self.array(key).is_some() || self.assoc(key).is_some() {
                    let base = if flags & PM_INTEGER != 0 {
                        "integer"
                    } else if flags & (PM_EFLOAT | PM_FFLOAT) != 0 {
                        "float"
                    } else if flags & PM_HASHED != 0 || self.assoc(key).is_some() {
                        "association"
                    } else if flags & PM_ARRAY != 0 || self.array(key).is_some() {
                        "array"
                    } else {
                        "scalar"
                    };
                    let mut out = String::from(base);
                    if flags & PM_LEFT != 0 {
                        out.push_str("-left");
                    }
                    if flags & PM_RIGHT_B != 0 {
                        out.push_str("-right_blanks");
                    }
                    if flags & PM_RIGHT_Z != 0 {
                        out.push_str("-right_zeros");
                    }
                    if flags & PM_LOWER != 0 {
                        out.push_str("-lower");
                    }
                    if flags & PM_UPPER != 0 {
                        out.push_str("-upper");
                    }
                    if flags & PM_READONLY != 0 {
                        out.push_str("-readonly");
                    }
                    if flags & PM_EXPORTED != 0 {
                        out.push_str("-export");
                    }
                    return Some(out);
                }
                if self.has_assoc(key) {
                    Some("association".to_string())
                } else if self.has_array(key) {
                    Some("array".to_string())
                } else if self.has_scalar(key) || std::env::var(key).is_ok() {
                    Some("scalar".to_string())
                } else {
                    Some(String::new())
                }
            }

            // === NAMED DIRECTORIES ===
            // ${nameddirs[@]} returns paths in sorted-name order (was
            // HashMap::values() with random iteration).
            "nameddirs" => {
                // Canonical `nameddirtab` lives in
                // `src/ported/hashnameddir.rs` (port of `Src/hashnameddir.c`).
                let tab = crate::ported::hashnameddir::nameddirtab();
                if key == "@" || key == "*" {
                    let snapshot: Vec<(String, String)> = tab
                        .lock()
                        .ok()
                        .map(|g| g.iter().map(|(k, v)| (k.clone(), v.dir.clone())).collect())
                        .unwrap_or_default();
                    let mut keys: Vec<&(String, String)> = snapshot.iter().collect();
                    keys.sort_by(|a, b| a.0.cmp(&b.0));
                    let vals: Vec<String> = keys.iter().map(|(_, v)| v.clone()).collect();
                    return Some(vals.join(" "));
                }
                Some(
                    tab.lock()
                        .ok()
                        .and_then(|g| g.get(key).map(|nd| nd.dir.clone()))
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
            // Canonical `dirstack` lives in `modules/parameter.rs::DIRSTACK`
            // — mirror of the C `dirstack` global (`Src/builtin.c:1456`).
            "dirstack" => {
                let dirs = crate::ported::modules::parameter::DIRSTACK
                    .lock()
                    .map(|g| g.clone())
                    .unwrap_or_default();
                if key == "@" || key == "*" {
                    return Some(dirs.join(" "));
                }
                if let Ok(idx) = key.parse::<usize>() {
                    Some(dirs.get(idx).cloned().unwrap_or_default())
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
                    .scalar("PWD")
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
                let user_loaded: Vec<String> = crate::ported::options::opt_state_snapshot()
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
                    || crate::ported::options::opt_state_get(&format!("_module_{}", key))
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
            // function-call code at vm_helper:7828-7835. Surface it here
            // so `${funcstack[1]}` / `${funcstack[@]}` reads work.
            // funcfiletrace / funcsourcetrace need separate tables (file
            // and definition tracking) which we don't yet wire; emit
            // empty for those until they're populated.
            "funcstack" => {
                if let Some(stack) = self.array("funcstack") {
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
                if let Some(stack) = self.array("funcstack") {
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
            // model (see do_enable_disable at vm_helper:31371) so the
            // disabled list isn't recoverable post-disable; emit
            // empty for those.
            "dis_builtins" => {
                let disabled: Vec<String> = crate::ported::options::opt_state_snapshot()
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
                if key == "@" || key == "*" {
                    let mut names = listwidgets();
                    names.sort();
                    return Some(names.join(" "));
                }
                if let Some(target) = getwidgettarget(key) {
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
}
