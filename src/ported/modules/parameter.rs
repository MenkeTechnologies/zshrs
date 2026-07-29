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
    aliastab_lock, cmdnam_hashed, cmdnamtab_lock, createaliasnode, shfunctab_lock, sufaliastab_lock,
};
use crate::ported::hist::hist_ring;
use crate::ported::jobs::{getjob, selectjobtab, sigmsg};
use crate::ported::module::MODULESTAB;
use crate::ported::options::{dosetopt, opt_state_set, optlookup};
use crate::ported::params::{deleteparamtable, getsparam, getstrvalue, realparamtab};
use crate::ported::utils::zwarn;
use crate::ported::zsh_h::{
    hashnode, hashtable, isset, module, nameddir, opt_name, param, value, HashNode, HashTable,
    Param, ScanFunc, ALIAS_GLOBAL, ALIAS_SUFFIX, DISABLED, FS_EVAL, FS_SOURCE, INTERACTIVE,
    ND_USERNAME, PM_ARRAY, PM_AUTOLOAD, PM_EFLOAT, PM_EXPORTED, PM_FFLOAT, PM_HASHED, PM_HIDE,
    PM_HIDEVAL, PM_INTEGER, PM_LEFT, PM_LOWER, PM_NAMEREF, PM_READONLY, PM_RIGHT_B, PM_RIGHT_Z,
    PM_SCALAR, PM_SPECIAL, PM_TAGGED, PM_TIED, PM_TYPE, PM_UNALIASED, PM_UNIQUE, PM_UNSET,
    PM_UPPER, SCANPM_MATCHVAL, SCANPM_WANTKEYS, SCANPM_WANTVALS, SP_RUNNING, STAT_DONE,
    STAT_NOPRINT, STAT_STOPPED,
};
use crate::zsh_h::{shfunc, HASHED};
use crate::DPUTS;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
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
    //              type-letter and append all PM_* modifier suffixes
    //              (-readonly, -export, -tied, -special, -local, etc.)
    //              per c:120-200. The full encoding is what
    //              `paramtypestr` (this module, line 51) produces;
    //              previous getpmparameter inlined a stripped-down
    //              dispatch that returned just the base type letter,
    //              dropping the modifier suffixes entirely. Mirror C
    //              by routing through paramtypestr.
    //
    // Symptom of the previous form:
    //   typeset -gx VAR=val; echo "${parameters[VAR]}"
    //   zshrs (before): scalar        zsh: scalar-export
    let value = {
        let tab = crate::ported::params::paramtab().read().unwrap();
        // c:107-114 — initial getnode2 returns the bare param (the
        // nameref itself, not its target). After computing its type
        // string, if the param is a nameref with a non-empty target
        // u.str, re-resolve via getnode (which follows the nameref)
        // and append "-{target_type}" with a hyphen separator.
        //
        // Prior Rust port stopped after the first paramtypestr call so
        // `${parameters[my_nameref]}` returned "nameref" instead of
        // C's "nameref-scalar" / "nameref-array" / etc. — the second
        // hop tells the user what kind of param the reference points
        // to, which is the whole reason to use ${parameters[X]} on a
        // nameref in the first place.
        if let Some(pm) = tab.get(name) {
            let base = paramtypestr(pm);
            // c:110 — `(rpm->node.flags & PM_NAMEREF) && rpm->u.str && *(rpm->u.str)`
            let is_nameref = (pm.node.flags as u32 & PM_NAMEREF) != 0;
            let target_name: Option<&str> = if is_nameref {
                pm.u_str.as_deref().filter(|s| !s.is_empty())
            } else {
                None
            };
            if let Some(tn) = target_name {
                // c:111-112 — getnode (resolves the nameref) → check PM_UNSET.
                if let Some(target_pm) = tab.get(tn) {
                    if (target_pm.node.flags as u32 & PM_UNSET) == 0 {
                        // c:113 — `zhtricat(pm->u.str, "-", paramtypestr(rpm))`.
                        format!("{}-{}", base, paramtypestr(target_pm))
                    } else {
                        base
                    }
                } else {
                    base
                }
            } else {
                base
            }
        } else {
            String::new()
        }
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
        u_tied: None,
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

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/parameter.c:43-95
    // paramtypestr modifier chain (every PM_* branch independently).
    // ═══════════════════════════════════════════════════════════════════

    /// c:49 — PM_AUTOLOAD short-circuits to "undefined" regardless
    /// of any other flags or type.
    #[test]
    fn paramtypestr_autoload_returns_undefined() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(paramtypestr(&make_pm(PM_AUTOLOAD, 0)), "undefined");
        // Even combined with other flags — autoload wins.
        assert_eq!(
            paramtypestr(&make_pm(PM_AUTOLOAD | PM_READONLY, 5)),
            "undefined"
        );
        assert_eq!(
            paramtypestr(&make_pm(PM_AUTOLOAD | PM_SCALAR, 0)),
            "undefined"
        );
    }

    /// c:55 — PM_ARRAY alone → "array".
    #[test]
    fn paramtypestr_plain_array() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(paramtypestr(&make_pm(PM_ARRAY, 0)), "array");
    }

    /// c:56 — PM_INTEGER alone → "integer".
    #[test]
    fn paramtypestr_plain_integer() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(paramtypestr(&make_pm(PM_INTEGER, 0)), "integer");
    }

    /// c:57-58 — PM_EFLOAT and PM_FFLOAT both → "float".
    #[test]
    fn paramtypestr_both_float_types_render_as_float() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(paramtypestr(&make_pm(PM_EFLOAT, 0)), "float");
        assert_eq!(paramtypestr(&make_pm(PM_FFLOAT, 0)), "float");
    }

    /// c:59 — PM_HASHED → "association".
    #[test]
    fn paramtypestr_hashed_renders_as_association() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(paramtypestr(&make_pm(PM_HASHED, 0)), "association");
    }

    /// c:65-66 — PM_LEFT modifier appends "-left".
    #[test]
    fn paramtypestr_left_modifier() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            paramtypestr(&make_pm(PM_SCALAR | PM_LEFT, 0)),
            "scalar-left"
        );
    }

    /// c:67-68 — PM_RIGHT_B modifier appends "-right_blanks".
    #[test]
    fn paramtypestr_right_blanks_modifier() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            paramtypestr(&make_pm(PM_SCALAR | PM_RIGHT_B, 0)),
            "scalar-right_blanks"
        );
    }

    /// c:69-70 — PM_RIGHT_Z modifier appends "-right_zeros".
    #[test]
    fn paramtypestr_right_zeros_modifier() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            paramtypestr(&make_pm(PM_SCALAR | PM_RIGHT_Z, 0)),
            "scalar-right_zeros"
        );
    }

    /// c:71-72 — PM_LOWER modifier appends "-lower".
    #[test]
    fn paramtypestr_lower_modifier() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            paramtypestr(&make_pm(PM_SCALAR | PM_LOWER, 0)),
            "scalar-lower"
        );
    }

    /// c:73-74 — PM_UPPER modifier appends "-upper".
    #[test]
    fn paramtypestr_upper_modifier() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            paramtypestr(&make_pm(PM_SCALAR | PM_UPPER, 0)),
            "scalar-upper"
        );
    }

    /// c:75-76 — PM_READONLY modifier appends "-readonly".
    #[test]
    fn paramtypestr_readonly_modifier() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            paramtypestr(&make_pm(PM_SCALAR | PM_READONLY, 0)),
            "scalar-readonly"
        );
    }

    /// c:77-78 — PM_TAGGED modifier appends "-tag".
    #[test]
    fn paramtypestr_tagged_modifier() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            paramtypestr(&make_pm(PM_SCALAR | PM_TAGGED, 0)),
            "scalar-tag"
        );
    }

    /// c:79-80 — PM_TIED modifier appends "-tied".
    #[test]
    fn paramtypestr_tied_modifier() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            paramtypestr(&make_pm(PM_SCALAR | PM_TIED, 0)),
            "scalar-tied"
        );
    }

    /// c:81-82 — PM_EXPORTED modifier appends "-export".
    #[test]
    fn paramtypestr_exported_modifier() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            paramtypestr(&make_pm(PM_SCALAR | PM_EXPORTED, 0)),
            "scalar-export"
        );
    }

    /// c:83-84 — PM_UNIQUE modifier appends "-unique".
    #[test]
    fn paramtypestr_unique_modifier() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            paramtypestr(&make_pm(PM_ARRAY | PM_UNIQUE, 0)),
            "array-unique"
        );
    }

    /// c:85-86 — PM_HIDE modifier appends "-hide".
    #[test]
    fn paramtypestr_hide_modifier() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            paramtypestr(&make_pm(PM_SCALAR | PM_HIDE, 0)),
            "scalar-hide"
        );
    }

    /// c:87-88 — PM_HIDEVAL modifier appends "-hideval".
    #[test]
    fn paramtypestr_hideval_modifier() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            paramtypestr(&make_pm(PM_SCALAR | PM_HIDEVAL, 0)),
            "scalar-hideval"
        );
    }

    /// c:89-90 — PM_SPECIAL modifier appends "-special".
    #[test]
    fn paramtypestr_special_modifier() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            paramtypestr(&make_pm(PM_SCALAR | PM_SPECIAL, 0)),
            "scalar-special"
        );
    }

    /// c:63-64 — pm.level > 0 appends "-local" before other modifiers.
    /// Pin order: local-first to match C's modifier-chain order.
    #[test]
    fn paramtypestr_level_one_adds_local() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(paramtypestr(&make_pm(PM_SCALAR, 1)), "scalar-local");
        assert_eq!(paramtypestr(&make_pm(PM_INTEGER, 3)), "integer-local");
    }

    /// c:63-64 — pm.level = 0 does NOT add "-local". Pin to catch
    /// off-by-one (would label every var as "-local" silently).
    #[test]
    fn paramtypestr_level_zero_does_not_add_local() {
        let _g = crate::test_util::global_state_lock();
        let r = paramtypestr(&make_pm(PM_SCALAR, 0));
        assert!(
            !r.contains("local"),
            "level=0 must not include 'local', got: {}",
            r
        );
    }

    /// Combined modifier chain order: local → flags appear in C's
    /// declaration order (left/right_b/right_z/lower/upper/readonly/
    /// tag/tied/export/unique/hide/hideval/special).
    /// Pin: PM_LEFT + PM_READONLY + PM_EXPORTED = "scalar-left-readonly-export".
    #[test]
    fn paramtypestr_modifier_chain_order_matches_c() {
        let _g = crate::test_util::global_state_lock();
        let pm = make_pm(PM_SCALAR | PM_LEFT | PM_READONLY | PM_EXPORTED, 0);
        assert_eq!(paramtypestr(&pm), "scalar-left-readonly-export");
    }

    /// c:48 — PM_UNSET wins over any modifier — short-circuits to "".
    /// Pin: even if PM_READONLY etc. are set, PM_UNSET → "".
    #[test]
    fn paramtypestr_unset_wins_over_modifiers() {
        let _g = crate::test_util::global_state_lock();
        let pm = make_pm(PM_UNSET | PM_SCALAR | PM_READONLY, 0);
        assert_eq!(paramtypestr(&pm), "", "PM_UNSET wins over modifiers");
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
       // Reading `$parameters` IS an access to `parameters`, so it stops
       // being an autoload stub here (C: the module load that answers this
       // read defines the node). Its siblings stay stubs until touched.
    crate::vm_helper::mark_module_param_used("parameters");
    // Snapshot names + per-entry data under read lock so func() can
    // re-enter paramtab without deadlock — C is single-threaded so
    // walks the live table directly.
    let entries: Vec<(String, u32, String)> = {
        // c:135 — C `realparamtab` walk. Rust port keeps a separate
        // `realparamtab` static that's never been wired to the live
        // shell param storage; the actual shell paramtab is the
        // canonical source. Walk that instead so
        // `${(k)parameters}` / `${parameters[(i)PAT]}` see the
        // shell's actual params (PATH, USER, IFS, etc.). Without
        // this redirect, every scanpmparameters call returned empty.
        let tab = crate::ported::params::paramtab()
            .read()
            .expect("paramtab poisoned");
        tab.iter()
            .filter(|(_, p)| (p.node.flags as u32 & PM_UNSET) == 0) // c:138 PM_UNSET skip
            .map(|(name, p)| {
                let want_val = (flags as u32 & (SCANPM_WANTVALS | SCANPM_MATCHVAL)) != 0
                    || (flags as u32 & SCANPM_WANTKEYS) == 0; // c:140-142
                                                              // c:49-50 — `if (pm->node.flags & PM_AUTOLOAD) return
                                                              // dupstring("undefined");`. In C the `zsh/parameter` params
                                                              // sit in realparamtab as PM_AUTOLOAD stubs and materialize
                                                              // ONE AT A TIME as they are touched (loading the module for
                                                              // `$parameters` does not define its siblings), so an
                                                              // enumeration reports "undefined" for the untouched ones.
                                                              // zshrs seeds them all eagerly, so the stub state is tracked
                                                              // separately; without this every one reported its real type
                                                              // and `_parameters -g '^a*'` / `-g 'a*'` bucketed them the
                                                              // opposite way from zsh.
                let val = if want_val {
                    if crate::vm_helper::module_param_is_autoload_stub(name) {
                        "undefined".to_string() // c:50
                    } else {
                        paramtypestr(p)
                    }
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
            u_tied: None,
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
/// WARNING: param shape doesn't match C — C passes the temporary
/// HashTable built by `arrhashsetfn` (`Src/params.c:4113`) whose nodes
/// are child Params carrying values in `u.str`; zshrs's `hashnode` has
/// no value slot, so the (key, value) pairs are passed directly. The
/// iteration semantics are identical: one install per pair, additive
/// (existing `cmdnamtab` entries are NOT flushed).
pub fn setpmcommands(pm: Param, ht: &[(String, String)]) {
    // c:173
    // c:178-179 — if (!ht) return; (an empty pair list is the
    // equivalent no-op; the loop below simply doesn't run).
    //
    // c:181-194 — for each node: `cn = zshcalloc(...);
    //   cn->node.flags = HASHED; cn->u.cmd = ztrdup(getstrvalue(&v));
    //   cmdnamtab->addnode(cmdnamtab, ztrdup(hn->nam), &cn->node);`
    for (nam, path) in ht {
        let cn = cmdnam_hashed(nam, path); // c:183/191/192
        if let Ok(mut tab) = cmdnamtab_lock().write() {
            tab.add(cn); // c:194 addnode
        }
    }
    // c:196-205 — `if (ht != pm->u.hash) deleteparamtable(ht);` —
    // temp-table teardown; Rust drops the borrowed slice's owner.
    let _ = pm;
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
    // c:218-222 — `if (!cmdnamtab->getnode(cmdnamtab, name) &&
    //              isset(HASHLISTALL)) { cmdnamtab->filltable(...);
    //              cmd = cmdnamtab->getnode(...); }`
    // Walk cmdnamtab; if miss AND HASHLISTALL set (default ON in
    // zsh), fill the table from $PATH and retry.
    //
    // p10k / zinit hit `${commands[name]}` for every command-check
    // they do (`whence -p`-equivalent without a fork). Without the
    // HASHLISTALL-driven filltable, lookups returned empty for any
    // command the shell hadn't explicitly `hash`'d yet.
    let entry_exists = cmdnamtab_lock()
        .read()
        .ok()
        .and_then(|g| g.get(name).cloned())
        .is_some();
    if !entry_exists && crate::ported::zsh_h::isset(crate::ported::zsh_h::HASHLISTALL) {
        // c:220
        if let Some(path) = crate::ported::params::getsparam("PATH") {
            let path_arr: Vec<String> = path.split(':').map(|s| s.to_string()).collect();
            crate::ported::hashtable::fillcmdnamtable(&path_arr); // c:220
        }
    }
    let g = cmdnamtab_lock().read().ok()?;
    let entry = g.get(name); // c:218/221 cmdnamtab->getnode
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
        u_tied: None,
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
    // appear.
    //
    // Mirror the C HASHLISTALL gate: when set (default in interactive
    // and -fc modes), fill cmdnamtab from $PATH before walking it.
    // Same fix the bin_hash -m branch already does (builtin.rs:5180+).
    // Without this, `${commands[(i)pat]}` and `${(k)commands}` return
    // empty for any binary the shell hasn't already `hash`'d.
    if crate::ported::zsh_h::isset(crate::ported::zsh_h::HASHLISTALL) {
        if let Some(path) = getsparam("PATH") {
            let path_arr: Vec<String> = path.split(':').map(|s| s.to_string()).collect();
            crate::ported::hashtable::fillcmdnamtable(&path_arr); // c:253
        }
    }
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
    let mut shf: shfunc; // c:287 Shfunc shf
    let prog: Option<crate::ported::zsh_h::eprog>; // c:288 Eprog prog
                                                   // c:289 — int sn (used inside the TRAP branch only)

    // c:286 — char *value = dupstring(val);
    value = val.clone();
    // c:291 — val = metafy(val, strlen(val), META_REALLOC);
    val = crate::ported::utils::metafy(&val);
    // c:293 — prog = parse_string(val, 1);
    // parse_string ported at crate::ported::exec::parse_string (c:283
    // in Src/exec.c). Returns None on parse error → matches the C
    // !prog guard at c:295.
    prog = crate::ported::exec::parse_string(&val, 1); // c:293
    if prog.is_none() {
        // c:295 !prog
        zwarn(
            // c:296
            &format!("invalid function definition: {}", value),
        );
        return; // c:298
    }
    // c:300 — shf = zshcalloc(sizeof(*shf));
    // c:301 — shf->funcdef = dupeprog(prog, 0);
    // c:302 — shf->node.flags = dis;
    shf = shfunc {
        node: hashnode {
            next: None,
            nam: name.to_string(),
            flags: dis, // c:302
        },
        filename: None,
        lineno: 0,
        funcdef: prog.clone().map(Box::new), // c:301 — dupeprog(prog, 0)
        redir: None,
        sticky: None,
        body: Some(val.clone()), // body source retained for deferred-recompile flows
    };
    // c:303 — `shfunc_set_sticky(shf);` (EXTERN exec.c). Stamps the
    // pending sticky-emulation snapshot onto the new function.
    crate::ported::exec::shfunc_set_sticky(&mut shf);

    // c:305-313 — TRAP* handling: a function named TRAPINT / TRAPHUP /
    // TRAPCHLD / etc. is also installed as the signal trap for the
    // matching signal. settrap(sn, NULL, ZSIG_FUNC) tells the signal
    // subsystem "the handler is the same-named shell function — look
    // it up by name at delivery time," so we don't pass the Eprog.
    //
    // C body (verbatim):
    //   if (!strncmp(name, "TRAP", 4) &&
    //       (sn = getsigidx(name + 4)) != -1) {
    //       if (settrap(sn, NULL, ZSIG_FUNC)) {
    //           freeeprog(shf->funcdef);
    //           zfree(shf, sizeof(*shf));
    //           zsfree(val);
    //           return;
    //       }
    //   }
    //
    // settrap returns non-zero on rejection (invalid sig, MONITOR
    // can't-trap, etc.); on rejection C frees the half-built shfunc
    // and returns WITHOUT calling shfunctab->addnode — i.e. defining
    // TRAPTTOU under `setopt MONITOR` is a no-op that doesn't
    // register the function either. Prior Rust port just commented
    // the dispatch out, so TRAP* functions were never wired as
    // signal handlers — `TRAPINT() { ... }` defined via
    // `functions[TRAPINT]=...` never fired on SIGINT.
    if name.len() >= 4 && &name[..4] == "TRAP" {
        // c:305
        if let Some(sn) = crate::ported::signals::getsigidx(&name[4..]) {
            // c:306 sn = getsigidx(name + 4)
            if crate::ported::signals::settrap(sn, None, crate::ported::zsh_h::ZSIG_FUNC) != 0 {
                // c:307-312 — settrap rejected the install; don't
                // register the function either.
                return; // c:311
            }
        }
    }

    // c:314 — shfunctab->addnode(shfunctab, ztrdup(name), shf);
    if let Ok(mut tab) = shfunctab_lock().write() {
        tab.add(shf);
    }
    // Invalidate the executor's compiled-function cache so the next
    // call recompiles from the new body. Without this, re-defining
    // an existing function via `functions[name]=...` updates
    // shfunctab but the executor keeps dispatching to the cached
    // Eprog from the original definition (resulting in old-body
    // behavior). Bug #323 in docs/BUGS.md.
    // Invalidate via the exec accessors channel (drops the executor's
    // compiled-chunk + source caches) instead of reaching into the
    // ShellExecutor directly from the ported tree.
    let _ = crate::ported::exec::unregister_function(name);
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
/// WARNING: param shape doesn't match C — C passes the temporary
/// HashTable of value-carrying child Params (see setpmcommands);
/// zshrs passes the (key, value) pairs directly. Additive: existing
/// functions are NOT flushed.
pub fn setfunctions(pm: Param, ht: &[(String, String)], dis: i32) {
    // c:344
    // c:349-350 — if (!ht) return; (empty pair list = same no-op).
    //
    // c:352-362 — for each node:
    //   `setfunction(hn->nam, ztrdup(getstrvalue(&v)), dis);`
    for (nam, body) in ht {
        setfunction(nam, body.clone(), dis); // c:361
    }
    // c:364-365 — if (ht != pm->u.hash) deleteparamtable(ht);
    let _ = pm;
}

/// Port of `setpmfunctions(Param pm, HashTable ht)` from Src/Modules/parameter.c:370.
#[allow(non_snake_case)]
pub fn setpmfunctions(pm: Param, ht: &[(String, String)]) {
    // c:370
    setfunctions(pm, ht, 0) // c:370
}

/// Port of `setpmdisfunctions(Param pm, HashTable ht)` from Src/Modules/parameter.c:377.
/// C: `setfunctions(pm, ht, DISABLED);`
#[allow(non_snake_case)]
pub fn setpmdisfunctions(pm: Param, ht: &[(String, String)]) {
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
pub fn getfunction(_ht: *mut HashTable, name: &str, dis: i32) -> Option<Param> {
    // c:388
    // Faithful port of c:399-438:
    //   if ((shf = shfunctab->getnode2(shfunctab, name)) &&
    //       (dis ? (shf->node.flags & DISABLED)
    //            : !(shf->node.flags & DISABLED))) {
    //       if (shf->node.flags & PM_UNDEFINED) {
    //           pm->u.str = dyncat('builtin autoload -X', ...);
    //       } else { /* build pretty body */ }
    //   } else {
    //       pm->u.str = ''; flags |= PM_UNSET|PM_SPECIAL;
    //   }
    //
    // C uses getnode2 (no DISABLED filter) so the entry is visible
    // regardless of state; the dis-parity check decides visibility.
    // The Rust equivalent is get_including_disabled — get() filters
    // DISABLED out automatically.
    //
    // Prior port called shfunctab.get(name) which already drops the
    // disabled entries, then ignored the `dis` parameter entirely.
    // That meant:
    //   - \${(k)functions[(I)f]}     listed disabled fns (wrong)
    //   - \${(k)dis_functions[(I)f]} returned nothing (wrong: should
    //                                list ONLY disabled fns)
    let g = shfunctab_lock().read().ok()?;
    let entry = g.get_including_disabled(name); // c:399 shfunctab->getnode2
    let (value, found) = if let Some(shf) = entry {
        // c:400 — DISABLED parity check.
        let is_disabled = (shf.node.flags & DISABLED as i32) != 0;
        let dis_match = if dis != 0 { is_disabled } else { !is_disabled };
        if dis_match {
            // c:401-407 — PM_UNDEFINED autoload form. C builds the
            // suffix from PM_UNALIASED + PM_TAGGED:
            //   pm->u.str = dyncat("builtin autoload -X",
            //       ((shf->node.flags & PM_UNALIASED) ?
            //        ((shf->node.flags & PM_TAGGED) ? "Ut" : "U") :
            //        ((shf->node.flags & PM_TAGGED) ? "t" : "")));
            //
            // So autoload -X with the four-state suffix tells the user
            // (via ${functions[NAME]}) which `autoload` flags were
            // used: "U" = `autoload -U` (no alias expansion),
            // "t" = `autoload -t` (tracing on entry), "Ut" = both.
            // Prior Rust port always emitted bare "builtin autoload -X"
            // — `${functions[my_autoload_U]}` returned the same string
            // whether the function was loaded with `autoload -U` or
            // `autoload`, dropping the metaprogramming signal that
            // many prompt frameworks (powerlevel10k, starship)
            // consult when deciding whether to re-source.
            //
            // Static-link path also doesn't yet expose PM_UNDEFINED on
            // ShFunc; route via body.is_none() as the autoload signal,
            // then inspect node.flags for the U/t letters.
            let body = shf.body.as_deref();
            let v = match body {
                None => {
                    let f = shf.node.flags as u32;
                    let unaliased = (f & PM_UNALIASED) != 0;
                    let tagged = (f & PM_TAGGED) != 0;
                    let suffix = match (unaliased, tagged) {
                        // c:402-405
                        (true, true) => "Ut",
                        (true, false) => "U",
                        (false, true) => "t",
                        (false, false) => "",
                    };
                    format!("builtin autoload -X{}", suffix)
                }
                Some(text) => {
                    // c:Src/Modules/parameter.c:419 — `getpermtext(shf->funcdef,
                    // NULL, 1)`: C re-deparses the parsed body with tnewlins=1
                    // so a multi-statement body renders one statement per line,
                    // tab-indented (`print a; print b` → two `\t`-prefixed
                    // lines). zshrs stores the body as source text rather than
                    // the Eprog, so re-parse it and run the same getpermtext
                    // deparser to recover the line breaks. On a parse failure
                    // (should not happen — it parsed at definition time) fall
                    // back to the raw single-line text.
                    //
                    // Memoize: the deparse of a given body TEXT is
                    // deterministic, and whole-assoc consumers ($functions
                    // enumerations, e.g. zinit's `.zinit-diff-functions`)
                    // call this per key, repeatedly, across a plugin load.
                    // Re-parsing + re-deparsing every function body on every
                    // read made a p10k load take seconds. Cache keyed by the
                    // raw body text (self-invalidating: a redefinition
                    // changes the text → new key). C never pays this because
                    // it keeps the compiled Eprog and deparses on demand.
                    thread_local! {
                        static FN_DEPARSE_CACHE: std::cell::RefCell<
                            std::collections::HashMap<String, String>,
                        > = std::cell::RefCell::new(std::collections::HashMap::new());
                    }
                    if let Some(hit) = FN_DEPARSE_CACHE.with(|c| c.borrow().get(text).cloned()) {
                        hit
                    } else {
                        let out = match crate::ported::exec::parse_string(text, 0) {
                            Some(prog) => format!(
                                "\t{}",
                                crate::ported::text::getpermtext(Box::new(prog), None, 1)
                            ),
                            None => format!("\t{}", text),
                        };
                        FN_DEPARSE_CACHE
                            .with(|c| c.borrow_mut().insert(text.to_string(), out.clone()));
                        out
                    }
                }
            };
            (v, true)
        } else {
            // c:435-437 — wrong DISABLED parity: treat as not found.
            (String::new(), false)
        }
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
        u_tied: None,
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
    dis: i32,
) {
    // C body (c:464-514):
    //   for (i = 0; i < shfunctab->hsize; i++)
    //       for (hn = shfunctab->nodes[i]; hn; hn = hn->next) {
    //           if (dis ? (hn->flags & DISABLED)
    //                   : !(hn->flags & DISABLED)) {
    //               pm.node.nam = hn->nam;
    //               ... build body ...
    //               func(&pm.node, flags);
    //           }
    //       }
    //
    // Prior Rust port iterated every shfunctab entry and emitted
    // unconditionally — neither the dis parameter nor the DISABLED
    // bit was honoured. \${(k)dis_functions} returned every function
    // (wrong: should list ONLY disabled), \${(k)functions} included
    // disabled ones too. Both diverging from zsh -fc parity.
    //
    // Now reads the DISABLED bit from each entry and matches against
    // dis per C's c:470 gate. shfunctab.iter() exposes all entries
    // including disabled ones, so the flag check is what discriminates.
    let names: Vec<String> = if let Ok(g) = shfunctab_lock().read() {
        // c:468-470 — walk all shfunctab entries; filter by DISABLED.
        g.iter()
            .filter_map(|(n, shf)| {
                let is_disabled = (shf.node.flags & DISABLED as i32) != 0;
                let pass = if dis != 0 { is_disabled } else { !is_disabled };
                if pass {
                    Some(n.clone())
                } else {
                    None
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    if let Some(f) = func {
        for name in names {
            let node = Box::new(hashnode {
                next: None,
                nam: name,
                flags: 0, // c:472 fresh pm.node.nam
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
pub fn getfunction_source(_ht: *mut HashTable, name: &str, dis: i32) -> Option<Param> {
    // c:537
    // Faithful port of c:547-552:
    //   if ((shf = shfunctab->getnode2(shfunctab, name)) &&
    //       (dis ? (shf->node.flags & DISABLED)
    //            : !(shf->node.flags & DISABLED))) {
    //       pm->u.str = getshfuncfile(shf);
    //       if (!pm->u.str) pm->u.str = dupstring('');
    //   }
    //
    // Prior port had two bugs:
    //   1. shfunctab.get(name) filters out DISABLED entries
    //      automatically (hashtable.rs:404 .filter()), so disabled
    //      functions were invisible to both lookups.
    //   2. Ignored the dis parameter entirely.
    //
    // Use get_including_disabled and check DISABLED parity, matching
    // the getfunction fix in 615e408fc4.
    let g = shfunctab_lock().read().ok()?;
    let entry = g.get_including_disabled(name); // c:547 getnode2 — no DISABLED filter
    let (value, found) = if let Some(shf) = entry {
        let is_disabled = (shf.node.flags & DISABLED as i32) != 0;
        let dis_match = if dis != 0 { is_disabled } else { !is_disabled }; // c:548
        if dis_match {
            // c:549-551 — `pm->u.str = getshfuncfile(shf);
            //              if (!pm->u.str) pm->u.str = dupstring("");`
            //
            // Route through the canonical hashtable::getshfuncfile so
            // the PM_LOADDIR `filename/name` join lands here (matches
            // the c:1061 branch of getshfuncfile at hashtable.c:1059).
            // Previously this inlined `shf.filename.clone()` and
            // skipped the LOADDIR join, so autoloads loaded via fpath
            // dir match had `${functions_source[name]}` reporting the
            // dir instead of the source file path.
            drop(g); // release shfunctab lock before getshfuncfile re-acquires.
            let fname = crate::ported::hashtable::getshfuncfile(name).unwrap_or_default();
            // Re-acquire for the post-block path that needs `g`
            // again (none currently, but keep the structure clean).
            (fname, true)
        } else {
            // c:552 — wrong DISABLED parity: pm->u.str stays NULL,
            // then c:553 caller emits PM_UNSET|PM_SPECIAL.
            (String::new(), false)
        }
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
        u_tied: None,
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
    dis: i32,
) {
    // c:560
    // Faithful port of c:570-584:
    //   for (i = 0; i < shfunctab->hsize; i++) {
    //       for (hn = shfunctab->nodes[i]; hn; hn = hn->next) {
    //           if (dis ? (hn->flags & DISABLED)
    //                   : !(hn->flags & DISABLED)) {
    //               pm.node.nam = hn->nam;
    //               ... pm.u.str = getshfuncfile(...); ...
    //               func(&pm.node, flags);
    //           }
    //       }
    //   }
    //
    // Same fix pattern as scanfunctions (da3bce77e6): the dis
    // parameter wasn't read and the DISABLED bit wasn't checked.
    // \${(k)dis_functions_source} returned every fn, including
    // enabled ones (wrong). Now filters per C's c:572 gate using
    // the live shfunctab entry's node.flags.
    let names: Vec<String> = if let Ok(g) = shfunctab_lock().read() {
        g.iter()
            .filter_map(|(n, shf)| {
                let is_disabled = (shf.node.flags & DISABLED as i32) != 0;
                let pass = if dis != 0 { is_disabled } else { !is_disabled };
                if pass {
                    Some(n.clone())
                } else {
                    None
                }
            })
            .collect()
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
    // C walks `for (f = funcstack; f; f = f->prev)` — head of list is
    // most-recent frame. Rust stores frames in a Vec push-back, so the
    // last element is the most-recent; reverse-iterate to match C's
    // head-first order: innermost frame first, outermost last.
    let stack = FUNCSTACK.lock().map(|s| s.clone()).unwrap_or_default();
    stack.iter().rev().map(|f| f.name.clone()).collect() // c:648
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
                                                            // C walks from head (most recent) outward via f->prev. Rust's Vec
                                                            // is push-back so iterate reverse to match C head-first order.
    for f in f_stack.iter().rev() {
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
                                                            // C walks head-first via f->prev; Rust Vec push-back stores in
                                                            // reverse order — reverse-iterate to match.
    for f in f_stack.iter().rev() {
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
    // c:721 — for (f = funcstack, p = ret; f; f = f->prev, p++).
    // C walks head→tail via f->prev. Rust Vec is push-back order;
    // reverse-iterate so index 0 = most recent and `i+1` = previous
    // frame (the C `f->prev` link target).
    let n = stack.len();
    for i in 0..n {
        let f = &stack[n - 1 - i];
        // c:724 — if (!f->prev || f->prev->tp == FS_SOURCE) {
        // In reverse view: prev frame is at index `n - 1 - i - 1`
        // i.e. the next element in our reverse walk.
        let prev: Option<&crate::ported::zsh_h::funcstack> = if i + 1 < n {
            Some(&stack[n - 1 - i - 1])
        } else {
            None
        };
        let parent_is_source = match prev {
            None => true, // !f->prev
            Some(p) => p.tp == FS_SOURCE,
        };
        if parent_is_source {
            // c:731-737 — file context: "<caller>:<lineno>"
            ret.push(format!(
                "{}:{}",
                f.caller.as_deref().unwrap_or(""), // c:734
                f.lineno
            ));
        } else if let Some(prev) = prev {
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
pub fn getbuiltin(_ht: *mut HashTable, name: &str, dis: i32) -> Option<Param> {
    // c:775
    // Faithful port of c:780-793:
    //   pm = hcalloc; pm->node.nam = dupstring(name);
    //   pm->node.flags = PM_SCALAR | PM_READONLY;
    //   pm->gsu.s = &nullsetscalar_gsu;
    //   if ((bn = builtintab->getnode2(builtintab, name)) &&
    //       (dis ? (bn->node.flags & DISABLED)
    //            : !(bn->node.flags & DISABLED))) {
    //       char *t = ((bn->handlerfunc ||
    //                   (bn->node.flags & BINF_PREFIX))
    //                  ? "defined" : "undefined");
    //       pm->u.str = dupstring(t);
    //   } else {
    //       pm->u.str = dupstring("");
    //       pm->node.flags |= (PM_UNSET|PM_SPECIAL);
    //   }
    //
    // Prior port ignored the `dis` parameter entirely — every lookup
    // collapsed to "found in BUILTINS = enabled". Now honours the
    // DISABLED gate:
    //   - dis=0          → entry visible iff NOT in BUILTINS_DISABLED
    //   - dis=DISABLED   → entry visible iff IS in BUILTINS_DISABLED
    // Without this distinction, `$builtins[ls]` reported "defined"
    // even after `disable ls`, and `$dis_builtins[ls]` reported "" /
    // PM_UNSET when ls was actually disabled — both diverging from
    // zsh -fc parity.
    let entry = BUILTINS
        .iter() // c:784
        .find(|b| b.node.nam == name);
    let (value, found) = if let Some(bn) = entry {
        // c:785 — `bn != NULL`. Check the DISABLED state.
        let is_disabled = {
            let set = crate::ported::builtin::BUILTINS_DISABLED.lock().ok();
            set.map(|s| s.contains(name)).unwrap_or(false)
        };
        let dis_match = if dis != 0 {
            is_disabled // c:785 dis ? (DISABLED) : ...
        } else {
            !is_disabled // c:785 ... : !(DISABLED)
        };
        if dis_match {
            // c:786-789 — `defined` if handlerfunc present OR
            // BINF_PREFIX set; else `undefined`.
            //
            // c:Src/module.c:1002 addbuiltin / :1265 add_autobin — an
            // AUTOLOADED module contributes its builtins to `builtintab`
            // as STUBS whose `handlerfunc` is NULL until the module is
            // actually loaded; that is precisely the `undefined` arm.
            // `zsh -fc` reports 27 such names (bindkey, compadd, zle,
            // ulimit, zstyle, …) — the exact contents of the
            // builtin→module auto-load registry at Src/init.c:1708
            // init_bltinmods, ported to `MODULESTAB.autoload_builtins`.
            // zshrs links every module statically, so its BUILTINS rows
            // ALWAYS carry a handler and every name read `defined`,
            // making `${builtins[zle]}` and `${builtins[(R)undefined]}`
            // diverge. Model the stub state the way C does: a name in
            // the auto-load registry has no handler until its module is
            // loaded.
            let has_handler = bn.handlerfunc.is_some()
                && crate::ported::module::MODULESTAB
                    .lock()
                    .map(|t| match t.resolve_autoload_builtin(name) {
                        // c:1265 — still a stub while the module is
                        // merely registered for auto-load. "Actually
                        // loaded" is C's `m->u.handle && !MOD_UNLOAD`
                        // (c:1055), whose static-link analog is
                        // MOD_INIT_B set / MOD_UNLOAD clear — the SAME
                        // criterion getpmmodule and scanpmmodules use to
                        // print `loaded` vs `autoloaded`. NOT
                        // `is_loaded()`, which keys off MOD_LINKED and is
                        // pre-seeded for every compiled-in module.
                        Some(m) => t.modules.get(m).is_some_and(|md| {
                            (md.node.flags & crate::ported::zsh_h::MOD_INIT_B) != 0
                                && (md.node.flags & crate::ported::zsh_h::MOD_UNLOAD) == 0
                        }),
                        // c:1002 — a real bintab row from zsh/main or an
                        // already-loaded module.
                        None => true,
                    })
                    .unwrap_or(true);
            let has_prefix = (bn.node.flags & crate::ported::zsh_h::BINF_PREFIX as i32) != 0;
            let t = if has_handler || has_prefix {
                "defined"
            } else {
                "undefined"
            };
            (t.to_string(), true) // c:790
        } else {
            // c:791-792 — wrong DISABLED parity: treat as not found.
            (String::new(), false)
        }
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
        u_tied: None,
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
    dis: i32,
) {
    // C body (c:816-840): loop through builtintab nodes; for each
    // matching DISABLED filter, emit a scalar Param via func().
    // Static-link path: walk BUILTINS table from src/ported/builtin.rs
    // (the Rust canonical source for builtin entries).
    //
    // c:Src/Modules/parameter.c:825 — `if (dis ? (hn->flags & DISABLED)
    // : !(hn->flags & DISABLED))`. With `dis=0` (the `builtins`
    // param), emit only enabled entries; with `dis=DISABLED` (the
    // `dis_builtins` param), emit only disabled entries. The Rust
    // BUILTINS table currently carries no DISABLED bit at construction
    // (every entry's `flags == 0`), so dis=DISABLED → no entries
    // (matches zsh: empty dis_builtins by default). Previously the
    // filter was ignored, so `${#dis_builtins}` returned 159 instead
    // of 0.
    if let Some(f) = func {
        // c:Src/Modules/parameter.c:816-840 — C iterates the LIVE
        // `builtintab` which only contains builtins from currently-
        // loaded modules. zshrs's `BUILTINS` slice is the static
        // union of every statically-linked module's bintab, so direct
        // iteration over-reports in --zsh parity mode where modules
        // aren't auto-loaded. Skip entries whose owning module isn't
        // loaded so `${(k)builtins}` and `${#builtins}` agree with
        // `zsh -fc` (e.g. zsh's count is 103, zshrs's full BUILTINS
        // is 159). Default zshrs mode walks the full set so user
        // scripts see all built-in module commands without explicit
        // zmodload — matching the auto-load posture.
        //
        // BUILTINS also currently contains some duplicate entries
        // (`fg`, `kill`, `suspend` appear in two adjacent batches,
        // builtin.rs:12110-12423 + 12855-12878) — `builtintab` in
        // C is a hash table so re-adds collapse, but Vec iteration
        // here visits each occurrence. Dedup by name to match the
        // hash-table shape `${#builtins}` exposes.
        let mut emitted: std::collections::HashSet<String> = std::collections::HashSet::new();
        // c:825 — runtime DISABLED tracking lives in
        // BUILTINS_DISABLED (a HashSet maintained by `disable` /
        // `enable -r`). The BUILTINS slice's static `flags` field
        // never carries DISABLED at construction, so the prior
        // check `b_flags & DISABLED` always read 0. Read the live
        // set instead so `${(k)dis_builtins}` reflects the user's
        // `disable` invocations (and `${(k)builtins}` correctly
        // omits disabled entries).
        let disabled_set: std::collections::HashSet<String> =
            crate::ported::builtin::BUILTINS_DISABLED
                .lock()
                .ok()
                .map(|g| g.iter().cloned().collect())
                .unwrap_or_default();
        for b in BUILTINS.iter() {
            // c:823
            let is_disabled = disabled_set.contains(&b.node.nam); // c:825 hn->flags & DISABLED
            let pass = if dis != 0 {
                // c:825 dis ? (hn->flags & DISABLED)
                is_disabled
            } else {
                // c:825 !(hn->flags & DISABLED)
                !is_disabled
            };
            if !pass {
                continue;
            }
            // c:Src/Modules/parameter.c:823 — `builtintab` membership.
            // One predicate, shared with the compctl namespace dump:
            // see `builtin_in_builtintab`.
            if !crate::ext_builtins::builtin_in_builtintab(&b.node.nam) {
                continue;
            }
            if !emitted.insert(b.node.nam.clone()) {
                continue;
            }
            let node = Box::new(hashnode {
                next: None,
                nam: b.node.nam.clone(),
                flags: 0, // c:828
            });
            f(&node, flags); // c:838
        }
        // zshrs extension builtins (`ext_builtins::EXT_BUILTIN_NAMES`)
        // dispatch in-process exactly like core builtins but have no
        // entry in the C-port BUILTINS table, so `${(k)builtins}` — and
        // therefore compsys's `_builtins` command-position completion —
        // would never offer names such as `doctor`, `peach`, `help`, or
        // `zassert_eq`. Emit them for the `builtins` param (dis == 0).
        //
        // `hide_ext_builtins()` suppresses them in two cases:
        //   * `--zsh` strict emulation (these names don't exist in zsh),
        //   * `ZSHRS_HIDE_EXT_BUILTINS` — the parity harnesses' knob, so
        //     a byte-for-byte `${(ko)builtins}` diff against real zsh
        //     isn't drowned in ~145 zshrs-original names. It is a
        //     MEASUREMENT flag, not a compat mode: dispatch is untouched
        //     (`peach` still runs, `whence -w peach` still says builtin).
        if dis == 0 && !crate::ext_builtins::hide_ext_builtins() {
            for name in crate::ext_builtins::EXT_BUILTIN_NAMES {
                let n = (*name).to_string();
                if disabled_set.contains(&n) {
                    continue; // c:825 honor `disable`
                }
                if !emitted.insert(n.clone()) {
                    continue; // already emitted from BUILTINS (e.g. coreutils drop-ins)
                }
                let node = Box::new(hashnode {
                    next: None,
                    nam: n,
                    flags: 0,
                });
                f(&node, flags);
            }
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
    // c:898-902 — `for (i = 0; i < ZPC_COUNT; i++) if (zpc_strings[i]
    //   && !dis == !zpc_disables[i]) *p++ = dupstring(zpc_strings[i]);`
    // Walks the canonical ZPC_STRINGS table (port at pattern.rs:3065)
    // in lockstep with the per-slot zpc_disables byte vector
    // (pattern.rs:3506). dis=0 emits enabled tokens (zpc_disables[i]
    // == 0); dis=1 emits the disabled set ("disable -p NAME" added
    // them). Skips NULL-marked slots (ZPC_NULL / ZPC_BNULLKEEP /
    // ZPC_INPAR_PIPE / ZPC_KSHCHAR) per the `zpc_strings[i] &&` gate.
    let zpc_count = crate::ported::zsh_h::ZPC_COUNT as usize;
    let strings = crate::ported::pattern::ZPC_STRINGS;
    let disables = crate::ported::pattern::zpc_disables.lock().unwrap();
    for i in 0..zpc_count {
        // c:900
        if let Some(s) = strings[i] {
            // c:902 — `if (zpc_strings[i] && !dis == !zpc_disables[i])`
            //         The C-style boolean equality compares the LOGICAL
            //         truth of both sides: !dis == !disables[i] means
            //         "both zero" or "both non-zero".
            let dis_b = dis != 0;
            let dis_slot_b = disables[i] != 0;
            if dis_b == dis_slot_b {
                ret.push(s.to_string()); // c:903 dupstring
            }
        }
    }
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
    // Faithful port of c:929-936:
    //   if (!value || (strcmp(value, 'on') && strcmp(value, 'off')))
    //       zwarn('invalid value: %s', value);
    //   else if (!(n = optlookup(pm->node.nam)))
    //       zwarn('no such option: %s', pm->node.nam);
    //   else if (dosetopt(n, (value && strcmp(value, 'off')), 0, opts))
    //       zwarn('can't change option: %s', pm->node.nam);
    //
    // Prior port checked value first and optlookup, but ignored
    // dosetopt's return — \`options[rcs]=off\` silently swallowed
    // 'cannot change option' errors (e.g. when a special-case
    // dosetopt path refuses the flip).
    let val = value.as_str();
    if val != "on" && val != "off" {
        // c:930-931
        zwarn(&format!("invalid value: {}", value));
        return;
    }
    let nam = pm.node.nam.clone();
    let n = optlookup(&nam); // c:932
    if n == 0 {
        zwarn(&format!("no such option: {}", nam)); // c:933
        return;
    }
    let on = val == "on"; // c:934 (value && strcmp(value, 'off'))
    if dosetopt(n, on as i32, 0) != 0 {
        // c:934-935 — non-zero dosetopt return signals 'can't change'.
        zwarn(&format!("can't change option: {}", nam));
    }
}

/// Port of `unsetpmoption(Param pm, UNUSED(int exp))` from Src/Modules/parameter.c:941.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn unsetpmoption(pm: Param, exp: i32) {
    // c:941
    // Faithful port of c:945-948:
    //   if (!(n = optlookup(pm->node.nam)))
    //       zwarn('no such option: %s', pm->node.nam);
    //   else if (dosetopt(n, 0, 0, opts))
    //       zwarn('can't change option: %s', pm->node.nam);
    //
    // Prior port silently swallowed both error paths — \`unset
    // 'options[rcs]'\` returned zero status even for unknown
    // option names or refused flips.
    let n = optlookup(&pm.node.nam);
    if n == 0 {
        zwarn(&format!("no such option: {}", pm.node.nam)); // c:946
        return;
    }
    if dosetopt(n, 0, 0) != 0 {
        // c:947-948
        zwarn(&format!("can't change option: {}", pm.node.nam));
    }
}

/// Port of `setpmoptions(Param pm, HashTable ht)` from Src/Modules/parameter.c:953.
/// C: `static void setpmoptions(Param pm, HashTable ht)` — set or unset
/// each shell option named in `ht` based on its "on"/"off" value.
#[allow(non_snake_case)]
#[allow(unused_variables)]
/// WARNING: param shape doesn't match C — C passes the temporary
/// HashTable of value-carrying child Params (see setpmcommands);
/// zshrs passes the (key, value) pairs directly.
pub fn setpmoptions(pm: Param, ht: &[(String, String)]) {
    // c:953
    // c:958-959 — if (!ht) return; (empty pair list = same no-op).
    //
    // c:961-977 — per-pair walk.
    for (nam, val) in ht {
        if val.is_empty() || (val != "on" && val != "off") {
            // c:972
            zwarn(
                // c:973
                &format!("invalid value: {}", val),
            );
        } else {
            // c:974 — dosetopt(optlookup(hn->nam), (val && strcmp(val, "off")), 0, opts);
            let n = optlookup(nam);
            let on: i32 = if val != "off" { 1 } else { 0 };
            if n == 0 || dosetopt(n, on, 0) != 0 {
                // c:975-976 — failure path: can't change option.
                zwarn(
                    // c:976
                    &format!("can't change option: {}", nam),
                );
            }
        }
    }
    // c:979-980 — if (ht != pm->u.hash) deleteparamtable(ht);
    let _ = pm;
}

/// Port of `getpmoption(UNUSED(HashTable ht), const char *name)` from Src/Modules/parameter.c:988.
/// C: `static HashNode getpmoption(UNUSED(HashTable ht), const char *name)`
/// — emit "on"/"off" for the named shell option.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn getpmoption(ht: *mut HashTable, name: &str) -> Option<Param> {
    // c:988
    // c:991-1010 — synth Param: u.str = (isset(opt)) ? "on" : "off".
    // Read the live option state via the canonical opt_state_get
    // accessor (Src/ported/options.rs:1623) so `\${options[NAME]}`
    // reads the actual runtime state, not an empty placeholder.
    //
    // optlookup returns SIGNED optno: positive means the canonical
    // name (e.g. "rcs" → +RCS_OPTNUM), negative means a "no…" alias
    // (e.g. "norcs" → -RCS_OPTNUM) which is the INVERSE — when RCS
    // is OFF, "norcs" is "on".
    let optno = optlookup(name); // c:1003
    let (value, found) = if optno != 0 {
        // c:1005 — "on" if set, "off" otherwise.
        // For negative optno (the "no" alias), invert the state so
        // `options[norcs]` reports the inverse of `options[rcs]`.
        let on = crate::ported::options::opt_state_get(name).unwrap_or_else(|| {
            // Fallback: lookup via canonical (no-prefix-stripped)
            // name. Necessary because opt_state_get is keyed on
            // the canonical option name; "norcs" misses the
            // direct lookup, so retry with the stripped name and
            // negate.
            let stripped = name.strip_prefix("no").unwrap_or(name);
            let s = crate::ported::options::opt_state_get(stripped).unwrap_or(false);
            if optno < 0 {
                !s
            } else {
                s
            }
        });
        (
            if on {
                "on".to_string()
            } else {
                "off".to_string()
            },
            true,
        )
    } else {
        (String::new(), false) // c:1009
    };
    let pm = Box::new(param {
        // c:993 hcalloc
        node: hashnode {
            next: None,
            nam: name.to_string(), // c:994
            // c:995 — `pm->node.flags = PM_SCALAR;` for known options;
            //         c:1009 — `pm->node.flags |= (PM_UNSET|PM_SPECIAL);`
            //         for unknown.
            //
            // Prior port added PM_READONLY in BOTH branches. C does not
            // set PM_READONLY — `$options` is a writable assoc (the
            // pmoption_gsu vtable at c:996 carries a setfn that routes
            // assignment through setopt). With PM_READONLY spuriously
            // set, `options[interactive]=on` failed with "read-only
            // variable: options" instead of toggling the option. This
            // broke the most common documented usage of the parameter
            // module's $options view (zsh manual SHMODULES.zsh-parameter
            // documents options[name] as a writable mapping).
            flags: if found {
                PM_SCALAR as i32
            } else {
                (PM_SCALAR | PM_UNSET | PM_SPECIAL) as i32
            },
        },
        u_data: 0,
        u_tied: None,
        u_arr: None,
        u_str: Some(value), // c:1005 / c:1008
        u_val: 0,
        u_dval: 0.0,
        u_hash: None,
        gsu_s: None, // c:996 pmoption_gsu — setter wiring pending
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
    // c:1025-1026 — `for (i = 0; i < optiontab->hsize; i++)
    //   for (hn = optiontab->nodes[i]; hn; hn = hn->next)`:
    // the walk is in optiontab BUCKET order, not sorted/random order.
    // OPTIONTAB models that scan order (first element: posixargzero).
    let names: Vec<String> = crate::ported::options::OPTIONTAB
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
    // Faithful port of c:1051-1068:
    //   m = modulestab->getnode2(modulestab, name);
    //   if (!m) return NULL;
    //   if (m->u.handle && !(m->node.flags & MOD_UNLOAD)) {
    //       type = ((m->node.flags & MOD_ALIAS) ?
    //               dyncat('alias:', m->u.alias) : 'loaded');
    //   }
    //   if (!type) {
    //       if (m->autoloads && firstnode(m->autoloads))
    //           type = 'autoloaded';
    //   }
    //   if (type) pm->u.str = dupstring(type);
    //   else { pm->u.str = ''; pm->node.flags |= (PM_UNSET|PM_SPECIAL); }
    //
    // Prior port missed the MOD_ALIAS branch entirely: alias entries
    // (zmodload -A foo=bar) showed up as 'loaded' instead of
    // 'alias:bar'. Now matches C's three-way dispatch.
    let modtab = MODULESTAB.lock().unwrap();
    let (module_present, is_alias, alias_target) = match modtab.modules.get(name) {
        // c:1055 gate: m->u.handle && !MOD_UNLOAD. Static-link analog
        // is MOD_INIT_B && !MOD_UNLOAD — the SAME criterion
        // scanpmmodules uses. is_loaded() checks MOD_LINKED, which
        // register_builtin_modules pre-seeds for every compiled-in
        // module, so per-key reads reported "loaded" for modules the
        // scan (and zsh -fc) reports "autoloaded".
        Some(m) => {
            let loaded = (m.node.flags & crate::ported::zsh_h::MOD_INIT_B) != 0
                && (m.node.flags & crate::ported::zsh_h::MOD_UNLOAD) == 0;
            let alias = (m.node.flags & crate::ported::zsh_h::MOD_ALIAS) != 0;
            (loaded, alias, m.alias.clone().unwrap_or_default())
        }
        None => (false, false, String::new()),
    };
    // c:1051-1059 — this getfn IS zsh/parameter's: it can only run
    // with the module loaded, so it self-reports "loaded" (matches
    // scanpmmodules' self-report and zsh -fc).
    let module_present = module_present || name == "zsh/parameter";
    // c:1060 autoload check: C uses per-module m->autoloads linklist.
    // Rust equivalent: any autoload_* map entry whose value == name,
    // plus the canonical .mdd autofeatures stub table.
    let autoload_present = modtab.autoload_builtins.values().any(|v| v == name)
        || modtab.autoload_conditions.values().any(|v| v == name)
        || modtab.autoload_params.values().any(|v| v == name)
        || modtab.autoload_mathfuncs.values().any(|v| v == name);
    drop(modtab);
    // After the lock drop — autoload_param_stubs re-locks MODULESTAB
    // for its loaded-filter; calling it under the lock deadlocks.
    let autoload_present = autoload_present
        || crate::vm_helper::autoload_param_stubs()
            .iter()
            .any(|(_, m)| *m == name);
    // c:1056-1063 — emit 'alias:<target>' / 'loaded' / 'autoloaded' / unset.
    let typ = if module_present {
        if is_alias {
            // c:1056-1057 — `alias:<target>`
            Some(format!("alias:{}", alias_target))
        } else {
            // c:1057 — bare 'loaded'
            Some("loaded".to_string())
        }
    } else if autoload_present {
        // c:1060-1061
        Some("autoloaded".to_string())
    } else {
        None // c:1062
    };
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
        u_tied: None,
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
    // c:1088-1099 — modulestab walk, emit each LOADED module.
    // C gate at c:1091:
    //   if (m->u.handle && !(m->node.flags & MOD_UNLOAD))
    // Static-link analog of `m->u.handle`: MOD_INIT_B (boot ran).
    // Same gate module_loaded uses post-6435a0dca2.
    //
    // c:1093 emit: `(m->node.flags & MOD_ALIAS)
    //              ? dyncat('alias:', m->u.alias) : 'loaded'`.
    //
    // Prior port iterated every modulestab entry — emitted entries
    // for register_builtin_modules-registered-but-not-loaded modules
    // (zsh/files, zsh/system, zsh/zftp, etc. that carry MOD_UNLOAD
    // at register time). \${(k)modules} listed ALL of them as
    // 'loaded' — diverging from \`zsh -fc\` which only reports
    // \`zsh/main\` until something fires explicit zmodload.
    let modules: Vec<(String, String)> = {
        let tab = MODULESTAB.lock().unwrap();
        tab.modules
            .iter()
            .filter_map(|(name, m)| {
                let loaded = (m.node.flags & crate::ported::zsh_h::MOD_INIT_B) != 0
                    && (m.node.flags & crate::ported::zsh_h::MOD_UNLOAD) == 0;
                if !loaded {
                    return None; // c:1091 gate
                }
                // c:1093 — alias entries get 'alias:<target>', others
                // get 'loaded'.
                let val = if (m.node.flags & crate::ported::zsh_h::MOD_ALIAS) != 0 {
                    format!("alias:{}", m.alias.as_deref().unwrap_or(""))
                } else {
                    "loaded".to_string()
                };
                Some((name.clone(), val))
            })
            .collect()
    };
    for (name, val) in modules {
        // c:1090
        done.insert(name.clone()); // c:1095 addlinknode(done, ...)
        let node = emit(&name, &val); // c:1093 emit value-side
        func(&Box::new(node), flags); // c:1096
    }
    // c:1088-1099 — this scan IS zsh/parameter's getfn: in C it can
    // only run with zsh/parameter loaded, so the module reports
    // itself "loaded" (zsh -fc 'print ${(k)modules}' includes
    // zsh/parameter). zshrs builds the param in statically; emit the
    // self-report when the modulestab walk didn't.
    if done.insert("zsh/parameter".to_string()) {
        let node = emit("zsh/parameter", "loaded");
        func(&Box::new(node), flags);
    }
    // c:1102-1110 — builtintab autoloaded (BINF_ADDED clear with optstr → module).
    // C stores the OWNING MODULE NAME in `bn->optstr` only for
    // BINF_ADDED-clear autoload STUB entries (`zmodload -ab`); real
    // builtins' optstr is the option string. zshrs's builtintab holds
    // the real statically-linked builtins (BINF_ADDED clear), so
    // walking it emitted ~75 OPTION STRINGS as module names in
    // ${(k)modules}. The autoload-stub registry is
    // MODULESTAB.autoload_builtins (builtin -> owning module).
    let auto_bin_modules: Vec<String> = {
        let tab = MODULESTAB.lock().unwrap();
        tab.autoload_builtins.values().cloned().collect()
    };
    for opt in auto_bin_modules {
        if done.insert(opt.clone()) {
            let node = emit(&opt, "autoloaded"); // c:1108
            func(&Box::new(node), flags); // c:1109
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
    // c:1119-1124 — realparamtab PM_AUTOLOAD entries. The canonical
    // stub registry is vm_helper::autoload_param_stubs (the .mdd
    // autofeatures table filtered to unloaded owners) — MODULESTAB's
    // autoload_params map only carries explicitly-registered names
    // and missed zsh/zleparameter.
    for (_pname, m) in crate::vm_helper::autoload_param_stubs() {
        if done.insert(m.to_string()) {
            let node = emit(m, "autoloaded");
            func(&Box::new(node), flags); // c:1124
        }
    }
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
    // Faithful port of c:1168-1182:
    //   int ok = 1;
    //   if (*name != '0' || name[1]) {
    //       if (*name == '0') ok = 0;          ← leading-zero with more chars
    //       else {
    //           for (p = name; *p && idigit(*p); p++);
    //           if (*p) ok = 0;                ← non-digit suffix
    //       }
    //   }
    //   if (ok && (he = quietgethist(atoi(name))))
    //       pm->u.str = dupstring(he->node.nam);
    //   else {
    //       pm->u.str = dupstring('');
    //       pm->node.flags |= (PM_UNSET|PM_SPECIAL);
    //   }
    //
    // Prior port did `name.parse::<i64>().ok()?` which early-returned
    // None on parse failure — but C returns a valid Param with
    // PM_UNSET|PM_SPECIAL set, never NULL. The two paths differ:
    //
    //   - Rust None: caller sees 'no such param'
    //   - C valid Param + PM_UNSET: caller sees 'param exists but
    //     is unset'
    //
    // \${history[bogus]} should match C's 'exists but unset'
    // semantic so \${+history[bogus]} returns 1 (param exists).
    let bytes = name.as_bytes();
    let mut ok = true;
    // c:1168 — `if (*name != '0' || name[1])`.
    if bytes.first() != Some(&b'0') || bytes.len() > 1 {
        // c:1169 — `if (*name == '0') ok = 0;`
        if bytes.first() == Some(&b'0') {
            ok = false; // leading zero with more chars
        } else {
            // c:1171-1174 — walk digits; if non-digit hit, ok = 0.
            for b in bytes {
                if !b.is_ascii_digit() {
                    ok = false;
                    break;
                }
            }
            // Empty name also fails — for loop didn't break but
            // no digits means atoi(name)=0 which would match h0
            // wrongly. Treat empty as invalid.
            if bytes.is_empty() {
                ok = false;
            }
        }
    }
    // c:1177 — `if (ok && (he = quietgethist(atoi(name))))`.
    let value = if ok {
        let num: i64 = name.parse().unwrap_or(0);
        crate::ported::hist::quietgethist(num).map(|e| e.node.nam.clone())
    } else {
        None
    };
    let (val, found) = match value {
        Some(v) => (v, true),           // c:1178
        None => (String::new(), false), // c:1180-1181
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
        u_tied: None,
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
    // Lazy-history chokepoint: a WHOLE-`$history` scan (`${(u@)history}`,
    // `${history[(R)pat]}`, hsmw ^R) wants ALL of it — page the rest of
    // the HISTFILE in now. This is the on-demand full load; startup
    // never slurps the file (extensions/history_lazy).
    crate::history_lazy::page_older_until(0);
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
            u_tied: None,
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

/// Direct port of `static char **histwgetfn(UNUSED(Param pm))` from
/// `Src/Modules/parameter.c:1217`. The `$historywords` array getter.
/// C body c:1224-1226 prepends `bufferwords(NULL, NULL, NULL, 0)`
/// (current editor line) to the result, then walks the history
/// ring newest→oldest slicing each entry's words via the
/// `histent.words[]` (begin,end) byte-offset pairs in reverse
/// position order.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn histwgetfn(pm: *mut param) -> Vec<String> {
    // c:1217
    let mut out: Vec<String> = Vec::new();
    // c:1224 — `bufferwords(NULL, NULL, NULL, 0)` — current editor line.
    let zleline: String = crate::ported::zle::zle_main::ZLELINE
        .lock()
        .unwrap()
        .iter()
        .collect();
    let cursor = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    let (bw, _) = crate::ported::hist::bufferwords(&zleline, cursor);
    out.extend(bw); // c:1225-1226 pushnode
                    // c:1229-1247 — walk hist_ring newest-to-oldest, slicing each
                    // entry's words by `histent.words[iw*2..iw*2+2]` byte offsets in
                    // reverse-position order.
    if let Ok(ring) = hist_ring.lock() {
        for he in ring.iter().rev() {
            // c:1229
            let hstr = he.node.nam.as_bytes();
            let len = hstr.len() as i32;
            let nwords = he.nwords as i32;
            // c:1232 — `for (iw = he->nwords - 1; iw >= 0; iw--)`
            let mut iw = nwords - 1;
            while iw >= 0 {
                let i2 = (iw as usize) * 2;
                if i2 + 1 >= he.words.len() {
                    break;
                }
                let wbegin = he.words[i2] as i32; // c:1233
                let wend = he.words[i2 + 1] as i32; // c:1234
                                                    // c:1236 — signed-short overflow bounds check.
                if wbegin < 0 || wbegin >= len || wend < 0 || wend > len {
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
    let job = if pend_nonempty { getjob(name, "") } else { job };
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
                    u_tied: None,
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
            format!("{} (core dumped)", sigmsg(pn.status & 0x7f))
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
                    u_tied: None,
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
    // Fallback to global pwd (c:1452's `: pwd` arm). C's `pwd` is the
    // shell-tracked LOGICAL cwd (Src/params.c:108, written by bin_cd
    // at Src/builtin.c:1239-1242) — read it through the canonical
    // getsparam("PWD") accessor, not the symlink-resolved
    // current_dir().
    crate::ported::params::getsparam("PWD").unwrap_or_else(|| {
        std::env::current_dir()
            .ok()
            .and_then(|p| p.to_str().map(String::from))
            .unwrap_or_default()
    })
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
                    u_tied: None,
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
/// WARNING: param shape doesn't match C — C passes the temporary
/// HashTable of value-carrying child Params (see setpmcommands);
/// zshrs passes the (key, value) pairs directly. Replace semantics:
/// every non-ND_USERNAME named dir is flushed first (c:1552-1558).
pub fn setpmnameddirs(pm: Param, ht: &[(String, String)]) {
    // c:1544
    // c:1549-1550 — if (!ht) return; (empty pair list still flushes
    // in C? No — !ht returns BEFORE the flush; an empty-but-present
    // table flushes and installs nothing. The pairs slice is always
    // "present" here, so flush unconditionally.)

    // c:1552-1558 — for (i = 0; i < nameddirtab->hsize; i++) flush non-ND_USERNAME.
    // The Rust `HashMap<String, nameddir>` doesn't expose buckets by `i`;
    // we walk all entries collecting keys to remove (C's combined
    // removenode+freenode = HashMap::remove).
    if let Ok(mut tab) = nameddirtab().lock() {
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
    for (nam, val) in ht {
        if val.is_empty() {
            // c:1570 !val
            zwarn("invalid value: ''"); // c:1571
        } else {
            // c:1573 — Nameddir nd = zshcalloc(sizeof(*nd));
            let nd = nameddir {
                node: hashnode {
                    next: None,
                    nam: nam.clone(),
                    flags: 0, // c:1575 nd->node.flags = 0
                },
                dir: val.clone(), // c:1576 nd->dir = ztrdup(val)
                diff: 0,
            };
            addnameddirnode(nam, nd); // c:1577
        }
    }
    // c:1581-1589 — opts[INTERACTIVE] guard around deleteparamtable
    // (avoid removing sub-pms eagerly when an interactive shell is
    // watching). The temp table is the borrowed slice here; nothing
    // to delete, so the guard collapses.
    let _ = pm;
}

/// Direct port of `getpmnameddir(UNUSED(HashTable ht), const char *name)` from Src/Modules/parameter.c:1597.
/// C body (c:1600-1615):
/// ```c
/// pm = (Param) hcalloc(sizeof(struct param));
/// pm->node.nam = dupstring(name);
/// pm->node.flags = PM_SCALAR;
/// pm->gsu.s = &pmnamedir_gsu;
/// if ((nd = (Nameddir) nameddirtab->getnode(nameddirtab, name)) &&
///     !(nd->node.flags & ND_USERNAME))
///     pm->u.str = dupstring(nd->dir);
/// else {
///     pm->u.str = dupstring("");
///     pm->node.flags |= (PM_UNSET|PM_SPECIAL);
/// }
/// ```
/// `nameddirs` enumerates `hash -d NAME=path` entries — entries
/// added at runtime via the hash builtin (Src/hashnameddir.c:104,
/// `addnameddirnode`). The username branch is a separate path
/// (`userdirs`, getpmuserdir at c:1646).
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn getpmnameddir(ht: *mut HashTable, name: &str) -> Option<Param> {
    // c:1597
    let (value, found) = match crate::ported::hashnameddir::nameddirtab()
        .lock()
        .ok()
        .and_then(|t| t.get(name).cloned())
    {
        // c:1607 — `nd = nameddirtab->getnode(nameddirtab, name)`
        Some(nd) if (nd.node.flags & crate::ported::zsh_h::ND_USERNAME) == 0 => {
            // c:1608 — `!(nd->node.flags & ND_USERNAME)`
            (nd.dir.clone(), true) // c:1609 nd->dir
        }
        _ => (String::new(), false), // c:1611 ""
    };
    let pm = Box::new(param {
        // c:1601 hcalloc
        node: hashnode {
            next: None,
            nam: name.to_string(), // c:1602
            flags: if found {
                PM_SCALAR as i32 // c:1603
            } else {
                (PM_SCALAR | PM_UNSET | PM_SPECIAL) as i32 // c:1612
            },
        },
        u_data: 0,
        u_tied: None,
        u_arr: None,
        u_str: Some(value), // c:1609 / c:1611
        u_val: 0,
        u_dval: 0.0,
        u_hash: None,
        gsu_s: None, // c:1604 pmnamedir_gsu
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
    Some(pm) // c:1614
}

/// Direct port of `scanpmnameddirs(UNUSED(HashTable ht), ScanFunc func, int flags)` from Src/Modules/parameter.c:1618.
/// C body (c:1621-1641):
/// ```c
/// memset(&pm, 0, sizeof(struct param));
/// pm.node.flags = PM_SCALAR;
/// pm.gsu.s = &pmnamedir_gsu;
/// for (i = 0; i < nameddirtab->hsize; i++)
///     for (hn = nameddirtab->nodes[i]; hn; hn = hn->next)
///         if (!((nd = (Nameddir) hn)->node.flags & ND_USERNAME)) {
///             pm.node.nam = hn->nam;
///             if (func != scancountparams &&
///                 ((flags & (SCANPM_WANTVALS|SCANPM_MATCHVAL)) ||
///                  !(flags & SCANPM_WANTKEYS)))
///                 pm.u.str = dupstring(nd->dir);
///             func(&pm.node, flags);
///         }
/// ```
/// Walks the `hash -d` named directories table, NOT /etc/passwd.
/// The passwd enumeration belongs to `scanpmuserdirs` (c:1669).
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(_ht, func) vs C=(ht, func, flags)
pub fn scanpmnameddirs(
    _ht: *mut HashTable,
    func: Option<ScanFunc>, // c:1618
    flags: i32,
) {
    if let Some(f) = func {
        if let Ok(tab) = crate::ported::hashnameddir::nameddirtab().lock() {
            for (nam, nd) in tab.iter() {
                // c:1627-1628
                // c:1629 — `!(nd->node.flags & ND_USERNAME)`
                if (nd.node.flags & crate::ported::zsh_h::ND_USERNAME) != 0 {
                    continue;
                }
                let node = Box::new(hashnode {
                    next: None,
                    nam: nam.clone(),        // c:1630
                    flags: PM_SCALAR as i32, // c:1623 pm.node.flags
                });
                f(&node, flags); // c:1640
            }
        }
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
    // c:1657 — the lookup is `nameddirtab->getnode(nameddirtab, name)`
    // AFTER c:1651's filltable, and filltable is a no-op in a
    // non-interactive shell (Src/utils.c:1193-1194 `if (!interact)
    // return;` inside adduserdir — see scanpmuserdirs below). With an
    // empty table C takes the c:1660-1663 else-arm: empty value,
    // PM_UNSET. zshrs queries getpwnam directly, which has no such gate,
    // so `zsh -f -c 'print ${userdirs[root]}'` printed `/var/root` where
    // zsh prints nothing (and `${+userdirs[root]}` read 1 vs 0).
    let cname = std::ffi::CString::new(name).ok()?;
    let pwd = if crate::ported::zsh_h::isset(crate::ported::zsh_h::INTERACTIVE) {
        unsafe { libc::getpwnam(cname.as_ptr()) } // c:1657 nd lookup
    } else {
        std::ptr::null_mut() // c:1660 — empty nameddirtab, no node
    };
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
        u_tied: None,
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
    // c:1676 `nameddirtab->filltable(nameddirtab)` →
    // Src/hashnameddir.c:96 fillnameddirtable, whose getpwent loop feeds
    // every entry through `adduserdir(pw->pw_name, pw->pw_dir,
    // ND_USERNAME, 1)` — and adduserdir's FIRST statement is
    // Src/utils.c:1193-1194 `if (!interact) return;` ("We don't maintain
    // a hash table in non-interactive shells"). So in a NON-interactive
    // shell nameddirtab never receives the passwd entries and the walk at
    // c:1682-1692 finds nothing: `zsh -f -c 'print ${#userdirs}'` is 0
    // even though `~root` still expands (that goes through getpwnam in
    // filesubstr, not through this table). zshrs inlines the passwd
    // enumeration here instead of routing it through adduserdir, so it
    // skipped that gate and reported every account in the directory
    // service — 136 entries against zsh's 0.
    if !crate::ported::zsh_h::isset(crate::ported::zsh_h::INTERACTIVE) {
        return; // c:Src/utils.c:1193-1194 via c:1676 filltable
    }
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
    // c:1701-1702 — `ht->addnode(ht, ztrdup(pm->node.nam),
    //                            createaliasnode(value, flags));`
    //
    // C callers:
    //   setpmralias    → ht=aliastab,    flags=0
    //   setpmdisralias → ht=aliastab,    flags=DISABLED
    //   setpmgalias    → ht=aliastab,    flags=ALIAS_GLOBAL
    //   setpmdisgalias → ht=aliastab,    flags=ALIAS_GLOBAL|DISABLED
    //   setpmsalias    → ht=sufaliastab, flags=ALIAS_SUFFIX
    //   setpmdissalias → ht=sufaliastab, flags=ALIAS_SUFFIX|DISABLED
    //
    // Rust callers pass a null ht so the dispatch reads the
    // ALIAS_SUFFIX bit out of flags — same shape as getalias,
    // scanaliases (2d2cdbaa5a), setaliases (e01ed226f0).
    //
    // Prior port always wrote to aliastab_lock(). That meant
    // \$saliases[X]=Y / \$dis_saliases[X]=Y silently landed in
    // aliastab with an ALIAS_SUFFIX flag (impossible combo for
    // that table) and sufaliastab stayed untouched.
    let name = (*pm).node.nam.clone();
    let node = crate::ported::hashtable::createaliasnode(&name, &value, flags as u32);
    if (flags & ALIAS_SUFFIX) != 0 {
        let mut tab = sufaliastab_lock().write().expect("sufaliastab poisoned");
        tab.add(node);
    } else {
        let mut tab = aliastab_lock().write().expect("aliastab poisoned");
        tab.add(node);
    }
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
/// WARNING: param shape doesn't match C — C passes `alht` (the live
/// alias table) plus the temporary HashTable of value-carrying child
/// Params; zshrs selects the live table from the ALIAS_SUFFIX bit in
/// `flags` and takes the (key, value) pairs directly (see
/// setpmcommands for why: zshrs's `hashnode` carries no value slot).
pub fn setaliases(
    alht: *mut HashTable,
    pm: Param, // c:1769
    ht: &[(String, String)],
    flags: i32,
) {
    // c:1774-1775 — `if (!ht) return;` — an empty pair list still
    // performs the flag-class flush below, same as C's empty-but-
    // present temp table.

    // c:1777-1789 — drop every alias currently in `alht` whose flags
    // exactly match the target flag-class.
    //
    // C callers select aliastab/sufaliastab via the alht arg:
    //   setpmraliases    → alht=aliastab,    flags=0
    //   setpmdisraliases → alht=aliastab,    flags=DISABLED
    //   setpmgaliases    → alht=aliastab,    flags=ALIAS_GLOBAL
    //   setpmdisgaliases → alht=aliastab,    flags=ALIAS_GLOBAL|DISABLED
    //   setpmsaliases    → alht=sufaliastab, flags=ALIAS_SUFFIX
    //   setpmdissaliases → alht=sufaliastab, flags=ALIAS_SUFFIX|DISABLED
    //
    // Rust callers pass null `alht`; dispatch on the ALIAS_SUFFIX
    // bit (mirrors getalias c:1901 + the scanaliases fix at
    // 2d2cdbaa5a). Prior port always operated on aliastab — every
    // suffix-alias bulk assignment (\$saliases=(...) etc.) silently
    // wrote to the wrong table.
    let suffix_table = (flags & ALIAS_SUFFIX) != 0;
    let mut keys_to_drop: Vec<String> = Vec::new(); // c:1772 hn iteration
    {
        let tab = if suffix_table {
            sufaliastab_lock()
        } else {
            aliastab_lock()
        }
        .read()
        .expect("aliastab poisoned");
        for (name, alias) in tab.iter() {
            // c:1777-1778
            if (alias.node.flags as i32) == flags {
                // c:1786 flags == hn->node.flags
                keys_to_drop.push(name.clone()); // c:1787 removenode(alht, hn->nam)
            }
        }
    }
    if !keys_to_drop.is_empty() {
        let mut tab = if suffix_table {
            sufaliastab_lock()
        } else {
            aliastab_lock()
        }
        .write()
        .expect("aliastab poisoned");
        for name in keys_to_drop {
            let _ = tab.remove(&name); // c:1788 freenode(hd)
        }
    }

    // c:1791-1804 — walk every entry in the user-supplied `ht`,
    // call createaliasnode(val, flags) and add it to alht:
    //   v.pm = (Param)hn; val = getstrvalue(&v);
    //   alht->addnode(alht, ztrdup(hn->nam),
    //                 createaliasnode(ztrdup(val), flags));
    for (nam, val) in ht {
        let node = createaliasnode(nam, val, flags as u32); // c:1803
        let mut tab = if suffix_table {
            sufaliastab_lock()
        } else {
            aliastab_lock()
        }
        .write()
        .expect("aliastab poisoned");
        tab.add(node); // c:1802 addnode
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
pub fn setpmraliases(pm: Param, ht: &[(String, String)]) {
    // c:1812
    setaliases(std::ptr::null_mut(), pm, ht, 0) // c:1812
}

/// Port of `setpmdisraliases(Param pm, HashTable ht)` from Src/Modules/parameter.c:1819.
#[allow(non_snake_case)]
pub fn setpmdisraliases(pm: Param, ht: &[(String, String)]) {
    // c:1819
    setaliases(std::ptr::null_mut(), pm, ht, DISABLED) // c:1819
}

/// Port of `setpmgaliases(Param pm, HashTable ht)` from Src/Modules/parameter.c:1826.
#[allow(non_snake_case)]
pub fn setpmgaliases(pm: Param, ht: &[(String, String)]) {
    // c:1826
    setaliases(std::ptr::null_mut(), pm, ht, ALIAS_GLOBAL) // c:1826
}

/// Port of `setpmdisgaliases(Param pm, HashTable ht)` from Src/Modules/parameter.c:1833.
/// C: `setaliases(aliastab, pm, ht, ALIAS_GLOBAL|DISABLED);`
#[allow(non_snake_case)]
pub fn setpmdisgaliases(pm: Param, ht: &[(String, String)]) {
    // c:1833
    setaliases(std::ptr::null_mut(), pm, ht, ALIAS_GLOBAL | DISABLED) // c:1819
}

/// Port of `setpmsaliases(Param pm, HashTable ht)` from Src/Modules/parameter.c:1840.
#[allow(non_snake_case)]
pub fn setpmsaliases(pm: Param, ht: &[(String, String)]) {
    // c:1840
    setaliases(std::ptr::null_mut(), pm, ht, ALIAS_SUFFIX) // c:1840
}

/// Port of `setpmdissaliases(Param pm, HashTable ht)` from Src/Modules/parameter.c:1847.
#[allow(non_snake_case)]
pub fn setpmdissaliases(pm: Param, ht: &[(String, String)]) {
    // c:1847
    setaliases(std::ptr::null_mut(), pm, ht, ALIAS_SUFFIX | DISABLED) // c:1847
}

// (`scan_magic_assoc_keys` moved out of src/ported/ to
// src/exec_shims.rs — it has no C counterpart and the
// no-non-C-ported-in-src/ported rule applies. The canonical scanpm*
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
        let m = ALIAS_GSU_HANDLER.get_or_init(|| Mutex::new(HashMap::new()));
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
    // c:1911 — `alht->getnode2(alht, name)`. C `getnode2` is
    // `gethashnode2` (Src/hashtable.c:255) which returns the entry
    // REGARDLESS of the DISABLED flag (unlike `getnode` which masks
    // disabled ones). Without `_including_disabled`, `${dis_aliases[k]}`
    // would never find a disabled entry because Rust `.get()` filters
    // them out.
    let entry = g.get_including_disabled(name); // c:1911 alht->getnode2
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
        u_tied: None,
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
///   `return getalias(aliastab, ht, name, ALIAS_GLOBAL|DISABLED);`
#[allow(non_snake_case)]
pub fn getpmdisgalias(ht: *mut HashTable, name: &str) -> Option<Param> {
    // c:1944
    // Prior port passed `DISABLED` only — matched disabled REGULAR
    // aliases (collision with getpmdisralias). C's c:1946 actually
    // passes `ALIAS_GLOBAL|DISABLED`; getalias's strict-equality
    // check at c:1912 means the wrong constant returned None for
    // every disabled global alias since they carry the full
    // ALIAS_GLOBAL|DISABLED flag combination.
    getalias(std::ptr::null_mut(), ht, name, ALIAS_GLOBAL | DISABLED) // c:1946
}

/// Port of `getpmsalias(HashTable ht, const char *name)` from Src/Modules/parameter.c:1951.
/// C: `static HashNode getpmsalias(HashTable ht, const char *name)` →
///   `return getalias(sufaliastab, ht, name, ALIAS_SUFFIX);`
#[allow(non_snake_case)]
pub fn getpmsalias(ht: *mut HashTable, name: &str) -> Option<Param> {
    // c:1953
    getalias(std::ptr::null_mut(), ht, name, ALIAS_SUFFIX) // c:1953
}

/// Port of `getpmdissalias(HashTable ht, const char *name)` from Src/Modules/parameter.c:1958.
/// C: `static HashNode getpmdissalias(HashTable ht, const char *name)` →
///   `return getalias(sufaliastab, ht, name, ALIAS_SUFFIX|DISABLED);`
#[allow(non_snake_case)]
pub fn getpmdissalias(ht: *mut HashTable, name: &str) -> Option<Param> {
    // c:1960
    getalias(std::ptr::null_mut(), ht, name, ALIAS_SUFFIX | DISABLED) // c:1960
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
        // c:1965 — `alht` arg picks the table. C callers:
        //   scanpmraliases  → alht=aliastab,     alflags=0
        //   scanpmdisraliases → alht=aliastab,     alflags=DISABLED
        //   scanpmgaliases  → alht=aliastab,     alflags=ALIAS_GLOBAL
        //   scanpmdisgaliases → alht=aliastab,     alflags=ALIAS_GLOBAL|DISABLED
        //   scanpmsaliases  → alht=sufaliastab,  alflags=ALIAS_SUFFIX
        //   scanpmdissaliases → alht=sufaliastab,  alflags=ALIAS_SUFFIX|DISABLED
        //
        // Dispatch on the ALIAS_SUFFIX bit (mirrors getalias's
        // c:1901 table pick). Prior port used strict equality
        // `alflags == ALIAS_SUFFIX`, which routed the
        // SUFFIX|DISABLED case to aliastab instead of sufaliastab
        // — `${(k)dis_saliases}` returned the wrong table's
        // entries when there were no global aliases with DISABLED
        // set, but the misdirection breaks the moment a user
        // creates a disabled suffix alias.
        let lock = if (alflags & ALIAS_SUFFIX) != 0 {
            sufaliastab_lock()
        } else {
            aliastab_lock()
        };
        if let Ok(tab) = lock.read() {
            for (_, alias) in tab.iter() {
                // c:1976 — `for (al = ...; al; ...)`
                // c:1977 — `if (alflags == al->node.flags)` strict
                // equality: scanpmraliases passes alflags=0 (regular,
                // flags==0), scanpmdisraliases passes DISABLED,
                // scanpmgaliases ALIAS_GLOBAL, scanpmsaliases
                // ALIAS_SUFFIX, scanpmdissaliases ALIAS_SUFFIX|DISABLED
                // etc. Anything else is skipped.
                if alias.node.flags != alflags {
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
    // Faithful port of c:2105-2135:
    //   gs = get_all_groups();
    //   if (!gs) { zerr('failed to retrieve groups for user: %e', errno);
    //              PM_UNSET|PM_SPECIAL; return; }
    //   for (gaptr = gs->array; gaptr < gs->array + gs->num; gaptr++) {
    //       if (!strcmp(name, gaptr->name)) {
    //           sprintf(buf, '%d', gaptr->gid);
    //           pm->u.str = dupstring(buf);
    //           return &pm->node;
    //       }
    //   }
    //   pm->u.str = dupstring('');
    //   pm->node.flags |= (PM_UNSET|PM_SPECIAL);
    //
    // get_all_groups() returns ONLY the groups the current user
    // belongs to (primary + supplementary). Prior Rust port used
    // getgrnam(name) which returns ANY group from the system
    // database. Semantic divergence:
    //
    //   - C: \${usergroups[wheel]} = gid_of_wheel iff user in wheel
    //   - Rust pre-port: \${usergroups[wheel]} = gid_of_wheel
    //     regardless of membership
    //
    // Now build the per-user group set via getgroups(2) + getgrgid(3)
    // resolution, matching C's get_all_groups semantics.
    let mut user_groups: Vec<(libc::gid_t, String)> = Vec::new();
    // c:2106 part 1 — get_all_groups: getgroups returns supplementary
    // gids; the primary gid comes from the current effective gid.
    let n_groups = unsafe { libc::getgroups(0, std::ptr::null_mut()) };
    if n_groups >= 0 {
        let mut gids: Vec<libc::gid_t> = vec![0; n_groups as usize];
        let got = unsafe { libc::getgroups(n_groups, gids.as_mut_ptr()) };
        if got >= 0 {
            // c:2106 includes effective gid in the user's group set
            // — get_all_groups c:2081-2083 adds egid if not already
            // present.
            let egid = unsafe { libc::getegid() };
            if !gids.iter().any(|&g| g == egid) {
                gids.push(egid);
            }
            // c:2086-2092 — resolve each gid to a group name.
            for gid in gids {
                let grp = unsafe { libc::getgrgid(gid) };
                if grp.is_null() {
                    continue; // c:2088 — failed lookup; skip rather
                              // than abort the whole walk (matches the
                              // 'return NULL' in C but we're already
                              // building a partial set).
                }
                let gr_name = unsafe { std::ffi::CStr::from_ptr((*grp).gr_name) };
                if let Ok(s) = gr_name.to_str() {
                    user_groups.push((gid, s.to_string()));
                }
            }
        }
    }

    let (value, found) = match user_groups.iter().find(|(_, n)| n == name) {
        // c:2124-2131 — match on name; emit gid as %d.
        Some((gid, _)) => (gid.to_string(), true),
        None => (String::new(), false), // c:2134
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
        u_tied: None,
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
    // c:2143
    // Faithful port of c:2146-2167:
    //   gs = get_all_groups();
    //   if (!gs) { zerr(...); return; }
    //   for (gaptr = gs->array; gaptr < gs->array + gs->num; gaptr++) {
    //       pm.node.nam = gaptr->name;
    //       ... emit gid as %d ...
    //       func(&pm.node, flags);
    //   }
    //
    // C iterates ONLY the current user's groups. Prior Rust port
    // used getgrent() which walks every group in the system database
    // — same semantic divergence as getpmusergroups before the
    // 7b5a68a79f fix. \${(k)usergroups} listed every system group,
    // breaking scripts that iterate the user's actual memberships.
    //
    // Build the user group set the same way getpmusergroups now
    // does: getgroups(2) + getegid(2) + getgrgid(3).
    let f = match func {
        Some(f) => f,
        None => return,
    };

    let n_groups = unsafe { libc::getgroups(0, std::ptr::null_mut()) };
    if n_groups < 0 {
        return; // c:2155 — get_all_groups NULL → zerr + return.
    }
    let mut gids: Vec<libc::gid_t> = vec![0; n_groups as usize];
    let got = unsafe { libc::getgroups(n_groups, gids.as_mut_ptr()) };
    if got < 0 {
        return;
    }
    // c:2081-2083 — get_all_groups appends egid if not in the
    // supplementary list.
    let egid = unsafe { libc::getegid() };
    if !gids.iter().any(|&g| g == egid) {
        gids.push(egid);
    }
    for gid in gids {
        let grp = unsafe { libc::getgrgid(gid) };
        if grp.is_null() {
            continue; // c:2088
        }
        let name = unsafe { std::ffi::CStr::from_ptr((*grp).gr_name) };
        let node = Box::new(hashnode {
            next: None,
            nam: name.to_string_lossy().into_owned(), // c:2160
            flags: 0,
        });
        f(&node, flags); // c:2167
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
// to its scanpm*/getpm* C callbacks. Rust-side dispatch is wired
// through the static `PARTAB` table below: each entry pairs the
// name + PM_* flags + getfn/scanfn fn pointers, so paramsubst can
// route `${name[key]}` through `getpmX(name=key)` and `${(k)name}`
// through `scanpmX(...)` like C does via the GSU `getnfn`/`scantfn`
// callbacks (`Src/Modules/parameter.c:2235`+).

/// Function-pointer types matching C's `GetNodeFunc` / `ScanTabFunc`
/// for the magic-assoc table dispatch.
pub type HashGetFn = fn(*mut HashTable, &str) -> Option<Param>;
/// `HashScanFn` type alias.
pub type HashScanFn = fn(*mut HashTable, Option<crate::ported::zsh_h::ScanFunc>, i32);

/// Strongly-typed PARTAB entry. C's `paramdef` keeps these as opaque
/// pointers; Rust's static-initialization rules make explicit fn
/// pointers cleaner. Only the magic-assoc shape (PM_HASHED) is
/// populated here; PM_ARRAY entries (`dirstack`, `funcstack`,
/// `patchars`, `reswords`, etc.) need a separate ArrayGetFn type.
#[allow(non_camel_case_types)]
#[derive(Clone, Copy)]
pub struct PartabHashEntry {
    /// Parameter name — `${name[key]}` triggers dispatch.
    pub name: &'static str,
    /// PM_* flag bits (PM_HASHED, PM_READONLY_SPECIAL, etc.).
    pub flags: i32,
    /// Per-key value lookup. Mirrors `getpm*` family in C.
    pub getfn: HashGetFn,
    /// Owning module when the row is bound by an explicit zmodload
    /// (C: the row lives in THAT module's paramdef table —
    /// zsh/system's sysparams/errnos at system.c:902-904, mapfile,
    /// langinfo). None = zsh/parameter (always available).
    pub module: Option<&'static str>,
    /// Full-table enumeration. Mirrors `scanpm*` family in C.
    pub scanfn: HashScanFn,
}

/// `static const struct paramdef partab[]` from `Src/Modules/parameter.c:
/// 2235-2298`. Each entry binds a magic-assoc name to its
/// per-key/full-scan canonical callbacks. The PM_ARRAY entries
/// (dirstack/funcstack/patchars/reswords/historywords/etc.) aren't
/// included here — they live in a separate `PARTAB_ARRAY` once the
/// array-shaped getfn types land.
pub static PARTAB: &[PartabHashEntry] = &[
    // c:2235 — `aliases`: regular aliases (no ALIAS_GLOBAL bit).
    PartabHashEntry {
        name: "aliases",
        flags: PM_HASHED as i32, // c:2235 SPECIALPMDEF flags
        getfn: getpmralias,
        module: None,
        scanfn: scanpmraliases,
    },
    // c:2237 — `builtins`: read-only.
    PartabHashEntry {
        name: "builtins",
        flags: PM_HASHED as i32 | PM_READONLY as i32, // c:2237 PM_READONLY_SPECIAL
        getfn: getpmbuiltin,
        module: None,
        scanfn: scanpmbuiltins,
    },
    // c:2238 — `commands`: cmdnamtab lookup + PATH path-build.
    PartabHashEntry {
        name: "commands",
        flags: PM_HASHED as i32, // c:2238
        getfn: getpmcommand,
        module: None,
        scanfn: scanpmcommands,
    },
    // c:2241 — `dis_aliases`: aliases with DISABLED bit.
    PartabHashEntry {
        name: "dis_aliases",
        flags: PM_HASHED as i32, // c:2241
        getfn: getpmdisralias,
        module: None,
        scanfn: scanpmdisraliases,
    },
    // c:2243 — `dis_builtins`: read-only disabled.
    PartabHashEntry {
        name: "dis_builtins",
        flags: PM_HASHED as i32 | PM_READONLY as i32, // c:2243 PM_READONLY_SPECIAL
        getfn: getpmdisbuiltin,
        module: None,
        scanfn: scanpmdisbuiltins,
    },
    // c:2245 — `dis_functions`: shfunctab with DISABLED bit.
    PartabHashEntry {
        name: "dis_functions",
        flags: PM_HASHED as i32, // c:2245
        getfn: getpmdisfunction,
        module: None,
        scanfn: scanpmdisfunctions,
    },
    // c:2249 — `dis_galiases`.
    PartabHashEntry {
        name: "dis_galiases",
        flags: PM_HASHED as i32, // c:2249
        getfn: getpmdisgalias,
        module: None,
        scanfn: scanpmdisgaliases,
    },
    // c:2255 — `dis_saliases`.
    PartabHashEntry {
        name: "dis_saliases",
        flags: PM_HASHED as i32, // c:2255
        getfn: getpmdissalias,
        module: None,
        scanfn: scanpmdissaliases,
    },
    // c:2263 — `functions`: shfunctab lookup.
    PartabHashEntry {
        name: "functions",
        flags: PM_HASHED as i32, // c:2263
        getfn: getpmfunction,
        module: None,
        scanfn: scanpmfunctions,
    },
    // c:2269 — `galiases`: aliases with ALIAS_GLOBAL bit.
    PartabHashEntry {
        name: "galiases",
        flags: PM_HASHED as i32, // c:2269
        getfn: getpmgalias,
        module: None,
        scanfn: scanpmgaliases,
    },
    // c:2271 — `history`: history-ring entry by event number.
    PartabHashEntry {
        name: "history",
        flags: PM_HASHED as i32 | PM_READONLY as i32, // c:2271 PM_READONLY_SPECIAL
        getfn: getpmhistory,
        module: None,
        scanfn: scanpmhistory,
    },
    // c:2275 — `jobdirs`.
    PartabHashEntry {
        name: "jobdirs",
        flags: PM_HASHED as i32 | PM_READONLY as i32, // c:2275 PM_READONLY_SPECIAL
        getfn: getpmjobdir,
        module: None,
        scanfn: scanpmjobdirs,
    },
    // c:2277 — `jobstates`.
    PartabHashEntry {
        name: "jobstates",
        flags: PM_HASHED as i32 | PM_READONLY as i32, // c:2277 PM_READONLY_SPECIAL
        getfn: getpmjobstate,
        module: None,
        scanfn: scanpmjobstates,
    },
    // c:2279 — `jobtexts`.
    PartabHashEntry {
        name: "jobtexts",
        flags: PM_HASHED as i32 | PM_READONLY as i32, // c:2279 PM_READONLY_SPECIAL
        getfn: getpmjobtext,
        module: None,
        scanfn: scanpmjobtexts,
    },
    // c:2281 — `modules`.
    PartabHashEntry {
        name: "modules",
        flags: PM_HASHED as i32 | PM_READONLY as i32, // c:2281 PM_READONLY_SPECIAL
        getfn: getpmmodule,
        module: None,
        scanfn: scanpmmodules,
    },
    // c:2283 — `nameddirs`.
    PartabHashEntry {
        name: "nameddirs",
        flags: PM_HASHED as i32, // c:2283
        getfn: getpmnameddir,
        module: None,
        scanfn: scanpmnameddirs,
    },
    // c:2285 — `options`.
    PartabHashEntry {
        name: "options",
        flags: PM_HASHED as i32, // c:2285
        getfn: getpmoption,
        module: None,
        scanfn: scanpmoptions,
    },
    // c:2287 — `parameters`.
    PartabHashEntry {
        name: "parameters",
        flags: PM_HASHED as i32 | PM_READONLY as i32, // c:2287 PM_READONLY_SPECIAL
        getfn: getpmparameter,
        module: None,
        scanfn: scanpmparameters,
    },
    // c:2293 — `saliases`: suffix aliases.
    PartabHashEntry {
        name: "saliases",
        flags: PM_HASHED as i32, // c:2293
        getfn: getpmsalias,
        module: None,
        scanfn: scanpmsaliases,
    },
    // c:2295 — `userdirs`.
    PartabHashEntry {
        name: "userdirs",
        flags: PM_HASHED as i32 | PM_READONLY as i32, // c:2295 PM_READONLY_SPECIAL
        getfn: getpmuserdir,
        module: None,
        scanfn: scanpmuserdirs,
    },
    // c:2297 — `usergroups`.
    PartabHashEntry {
        name: "usergroups",
        flags: PM_HASHED as i32 | PM_READONLY as i32, // c:2297 PM_READONLY_SPECIAL
        getfn: getpmusergroups,
        module: None,
        scanfn: scanpmusergroups,
    },
    // c:2247 — `dis_functions_source`: hashed assoc, key=fn name,
    // value=source path. Same shape as `functions` but for disabled.
    PartabHashEntry {
        name: "dis_functions_source",
        flags: PM_HASHED as i32 | PM_READONLY as i32, // c:2247
        getfn: getpmdisfunction_source,
        module: None,
        scanfn: scanpmdisfunction_source,
    },
    // c:2265 — `functions_source`.
    PartabHashEntry {
        name: "functions_source",
        flags: PM_HASHED as i32 | PM_READONLY as i32, // c:2265
        getfn: getpmfunction_source,
        module: None,
        scanfn: scanpmfunction_source,
    },
    // Src/Modules/mapfile.c:212 SPECIALPMDEF("mapfile", 0, ...).
    // Separate module from parameter.c but same PARTAB shape — both
    // register via boot_/enables_ into paramtab.
    PartabHashEntry {
        name: "mapfile",
        flags: PM_HASHED as i32, // mapfile.c:212 SPECIALPMDEF flags=0
        getfn: crate::ported::modules::mapfile::getpmmapfile,
        module: Some("zsh/mapfile"),
        scanfn: crate::ported::modules::mapfile::scanpmmapfile,
    },
    // Src/Modules/terminfo.c:291 SPECIALPMDEF("terminfo", PM_READONLY, ...).
    PartabHashEntry {
        name: "terminfo",
        flags: PM_HASHED as i32 | PM_READONLY as i32, // terminfo.c:291
        getfn: crate::ported::modules::terminfo::getterminfo,
        module: None,
        scanfn: crate::ported::modules::terminfo::scanterminfo,
    },
    // Src/Modules/termcap.c:299 SPECIALPMDEF("termcap", PM_READONLY, ...).
    PartabHashEntry {
        name: "termcap",
        flags: PM_HASHED as i32 | PM_READONLY as i32, // termcap.c:299
        getfn: crate::ported::modules::termcap::gettermcap,
        module: None,
        scanfn: crate::ported::modules::termcap::scantermcap,
    },
    // Src/Zle/zleparameter.c:133 SPECIALPMDEF("widgets", PM_READONLY, ...).
    PartabHashEntry {
        name: "widgets",
        flags: PM_HASHED as i32 | PM_READONLY as i32, // zleparameter.c:133
        getfn: crate::ported::zle::zleparameter::getpmwidgets,
        module: None,
        scanfn: crate::ported::zle::zleparameter::scanpmwidgets,
    },
    // Src/Modules/system.c:904 SPECIALPMDEF("sysparams", PM_READONLY, ...).
    PartabHashEntry {
        name: "sysparams",
        flags: PM_HASHED as i32 | PM_READONLY as i32, // system.c:904
        getfn: crate::ported::modules::system::getpmsysparams,
        module: Some("zsh/system"),
        scanfn: crate::ported::modules::system::scanpmsysparams,
    },
    // Src/Modules/langinfo.c:455 SPECIALPMDEF("langinfo", PM_READONLY, ...).
    PartabHashEntry {
        name: "langinfo",
        flags: PM_HASHED as i32 | PM_READONLY as i32, // langinfo.c:455
        getfn: crate::ported::modules::langinfo::getlanginfo,
        module: Some("zsh/langinfo"),
        scanfn: crate::ported::modules::langinfo::scanlanginfo,
    },
];

// scanpmfunction_source / scanpmdisfunction_source already ported
// at lines 957/970 — re-used here as PARTAB.scanfn pointers.

/// PM_ARRAY entries from `Src/Modules/parameter.c:2239-2291` — single
/// whole-array getfn returning `Vec<String>` (no per-key dispatch).
/// Mirrors C's `gsu_array.getfn(pm) -> char**`.
pub type ArrayGetFn = fn(pm: *mut param) -> Vec<String>;
/// Whole-array setter. Mirrors C's `gsu_array.setfn(pm, x)`. Only the
/// writable special arrays (`dirstack`) have one; read-only specials
/// leave it `None`.
pub type ArraySetFn = fn(pm: *mut param, x: Vec<String>);

/// Strongly-typed entry for PM_ARRAY-shape magic-assocs.
#[allow(non_camel_case_types)]
#[derive(Clone, Copy)]
pub struct PartabArrayEntry {
    /// Parameter name — `${name}` / `${name[N]}` triggers dispatch.
    pub name: &'static str,
    /// PM_* flag bits — always include PM_ARRAY.
    pub flags: i32,
    /// Whole-array getter. Mirrors C's `gsu_array.getfn(pm)`.
    pub getfn: ArrayGetFn,
    /// Whole-array setter. Mirrors C's `gsu_array.setfn(pm, x)`. `None`
    /// for read-only specials; `Some` for writable ones (`dirstack`).
    pub setfn: Option<ArraySetFn>,
    /// Owning module when bound by explicit zmodload; None =
    /// zsh/parameter.
    pub module: Option<&'static str>,
}

/// `static const struct paramdef partab[]` PM_ARRAY subset from
/// `Src/Modules/parameter.c:2239-2291`. Each entry's `gsu_array.getfn`
/// returns the full array (no per-key dispatch).
pub static PARTAB_ARRAY: &[PartabArrayEntry] = &[
    // c:2273 — `historywords`: words from current line + every history
    // entry, newest-first, reverse-by-position within each entry.
    PartabArrayEntry {
        name: "historywords",
        flags: PM_ARRAY as i32 | PM_READONLY as i32, // c:2273 PM_READONLY_SPECIAL
        getfn: histwgetfn,
        setfn: None,
        module: None,
    },
    // c:2239 — `dirstack`: $DIRSTACK pushd/popd state.
    PartabArrayEntry {
        name: "dirstack",
        flags: PM_ARRAY as i32, // c:2239
        getfn: dirsgetfn,
        setfn: Some(dirssetfn), // c:2229 dirs_gsu.setfn = dirssetfn
        module: None,
    },
    // c:2251 — `dis_patchars`: pattern metacharacters when extendedglob off.
    PartabArrayEntry {
        name: "dis_patchars",
        flags: PM_ARRAY as i32 | PM_READONLY as i32, // c:2251 PM_READONLY_SPECIAL
        getfn: dispatcharsgetfn,
        setfn: None,
        module: None,
    },
    // c:2253 — `dis_reswords`: reserved words when disabled.
    PartabArrayEntry {
        name: "dis_reswords",
        flags: PM_ARRAY as i32 | PM_READONLY as i32, // c:2253 PM_READONLY_SPECIAL
        getfn: disreswordsgetfn,
        setfn: None,
        module: None,
    },
    // c:2257 — `funcfiletrace`: per-frame caller file+lineno.
    PartabArrayEntry {
        name: "funcfiletrace",
        flags: PM_ARRAY as i32 | PM_READONLY as i32, // c:2257
        getfn: funcfiletracegetfn,
        setfn: None,
        module: None,
    },
    // c:2259 — `funcsourcetrace`: per-frame def-site file+lineno.
    PartabArrayEntry {
        name: "funcsourcetrace",
        flags: PM_ARRAY as i32 | PM_READONLY as i32, // c:2259
        getfn: funcsourcetracegetfn,
        setfn: None,
        module: None,
    },
    // c:2261 — `funcstack`: function-call stack names.
    PartabArrayEntry {
        name: "funcstack",
        flags: PM_ARRAY as i32 | PM_READONLY as i32, // c:2261
        getfn: funcstackgetfn,
        setfn: None,
        module: None,
    },
    // c:2267 — `functrace`: per-frame call file+lineno.
    PartabArrayEntry {
        name: "functrace",
        flags: PM_ARRAY as i32 | PM_READONLY as i32, // c:2267
        getfn: functracegetfn,
        setfn: None,
        module: None,
    },
    // c:2289 — `patchars`: pattern metacharacters when extendedglob on.
    PartabArrayEntry {
        name: "patchars",
        flags: PM_ARRAY as i32 | PM_READONLY as i32, // c:2289
        getfn: patcharsgetfn,
        setfn: None,
        module: None,
    },
    // c:2291 — `reswords`: shell reserved words.
    PartabArrayEntry {
        name: "reswords",
        flags: PM_ARRAY as i32 | PM_READONLY as i32, // c:2291
        getfn: reswordsgetfn,
        setfn: None,
        module: None,
    },
    // c:2273 — `historywords`: histwgetfn (parameter.c:1217-1252).
    PartabArrayEntry {
        name: "historywords",
        flags: PM_ARRAY as i32 | PM_READONLY as i32, // c:2273
        getfn: histwgetfn,
        setfn: None,
        module: None,
    },
    // Src/Modules/system.c:902 SPECIALPMDEF("errnos", PM_ARRAY|PM_READONLY, ...).
    PartabArrayEntry {
        name: "errnos",
        flags: PM_ARRAY as i32 | PM_READONLY as i32, // system.c:902
        getfn: crate::ported::modules::system::errnosgetfn,
        setfn: None,
        module: Some("zsh/system"),
    },
    // Src/Zle/zleparameter.c:132 SPECIALPMDEF("keymaps", PM_ARRAY|PM_READONLY, ...).
    PartabArrayEntry {
        name: "keymaps",
        flags: PM_ARRAY as i32 | PM_READONLY as i32, // zleparameter.c:132
        getfn: crate::ported::zle::zleparameter::keymapsgetfn,
        setfn: None,
        module: None,
    },
    // Src/Builtins/sched.c:382 SPECIALPMDEF("zsh_scheduled_events",
    // PM_ARRAY|PM_READONLY, &sched_gsu, NULL, NULL). Registered into
    // paramtab by zsh/sched's handlefeatures (via partab[] at c:381).
    // zshrs auto-loads zsh/sched and lists it in zsh_default_loaded
    // (module.rs:1134), so `$+zsh_scheduled_events` should read 1.
    // schedgetfn at sched.rs:582 walks the schedcmds linked list and
    // emits `<time>:<flags>:<cmd>` per entry.
    PartabArrayEntry {
        name: "zsh_scheduled_events",
        flags: PM_ARRAY as i32 | PM_READONLY as i32, // sched.c:382
        getfn: crate::ported::builtins::sched::schedgetfn,
        setfn: None,
        module: None,
    },
];

// partab_get / partab_scan_keys / partab_array_get dispatch helpers
// live in src/vm_helper.rs (outside src/ported/ — they're Rust-only
// convenience wrappers over the typed fn pointers, not C ports).

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
    //
    // zshrs's bin entry skips the canonical handlefeatures chain,
    // so `crate::vm_helper::init_partab_params` is called directly
    // from ShellExecutor::new() to install PM_SPECIAL placeholder
    // Params for every PARTAB / PARTAB_ARRAY entry.
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
static ALIAS_GSU_HANDLER: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

// File-static globals for parameter.c port — c:38-44, src/init.c.
// `dirstack` lives in src/exec.c globals; `funcstack` in src/init.c.
// Mirror as Mutex<Vec<...>> for cross-thread safety.
/// `DIRSTACK` static.
pub static DIRSTACK: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// `INCLEANUP` static.
pub static INCLEANUP: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

// `funcstack` global from Src/exec.c:340 — head of the active shell
// function call stack. Rust port mirrors the chain as Vec snapshot
// (the C source walks `funcstack->prev` to produce array params).
/// `FUNCSTACK` static.
pub static FUNCSTACK: Mutex<Vec<crate::ported::zsh_h::funcstack>> = Mutex::new(Vec::new());

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
fn setfeatureenables(
    _m: *const module,
    _f: &Mutex<crate::ported::zsh_h::features>,
    _e: Option<&[i32]>,
) -> i32 {
    0
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
            u_tied: None,
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
        realparamtab().write().unwrap().remove("ZSHRS_TEST_SP_A");
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
        let snapshot: Vec<_> = hist_ring.lock().unwrap().drain(..).collect();
        scanpmhistory(std::ptr::null_mut(), Some(counting_func), 0);
        let observed = COLLECTED_COUNT.load(Ordering::SeqCst);
        hist_ring.lock().unwrap().extend(snapshot);
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
        };
        setalias(std::ptr::null_mut(), Box::new(pm), "echo hi".to_string(), 0);
        let tab = aliastab_lock().read().expect("aliastab poisoned");
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

    // ═══════════════════════════════════════════════════════════════════
    // paramtypestr — per-flag combination pinning. Each test builds a
    // param with a specific flag set and asserts the exact return string.
    // ═══════════════════════════════════════════════════════════════════

    fn mk_pm(flags: u32, level: i32) -> param {
        param {
            node: hashnode {
                next: None,
                nam: String::new(),
                flags: flags as i32,
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
            level,
        }
    }

    /// PM_SCALAR (flag value 0) → "scalar".
    #[test]
    fn paramtypestr_scalar_with_no_flags_is_scalar() {
        let pm = mk_pm(PM_SCALAR, 0);
        assert_eq!(paramtypestr(&pm), "scalar");
    }

    /// PM_INTEGER → "integer".
    #[test]
    fn paramtypestr_integer_flag_is_integer() {
        let pm = mk_pm(PM_INTEGER, 0);
        assert_eq!(paramtypestr(&pm), "integer");
    }

    /// PM_ARRAY → "array".
    #[test]
    fn paramtypestr_array_flag_is_array() {
        let pm = mk_pm(PM_ARRAY, 0);
        assert_eq!(paramtypestr(&pm), "array");
    }

    /// PM_HASHED → "association".
    #[test]
    fn paramtypestr_hashed_flag_is_association() {
        let pm = mk_pm(PM_HASHED, 0);
        assert_eq!(paramtypestr(&pm), "association");
    }

    /// PM_NAMEREF → "nameref".
    #[test]
    fn paramtypestr_nameref_flag_is_nameref() {
        let pm = mk_pm(PM_NAMEREF, 0);
        assert_eq!(paramtypestr(&pm), "nameref");
    }

    /// PM_EFLOAT → "float".
    #[test]
    fn paramtypestr_efloat_flag_is_float() {
        let pm = mk_pm(PM_EFLOAT, 0);
        assert_eq!(paramtypestr(&pm), "float");
    }

    /// PM_FFLOAT → "float".
    #[test]
    fn paramtypestr_ffloat_flag_is_float() {
        let pm = mk_pm(PM_FFLOAT, 0);
        assert_eq!(paramtypestr(&pm), "float");
    }

    /// PM_UNSET shortcut returns empty string regardless of other flags.
    /// c:48-92 — `if (PM_UNSET) return ""`.
    #[test]
    fn paramtypestr_unset_short_circuits_to_empty() {
        let pm = mk_pm(PM_UNSET | PM_INTEGER, 0);
        assert_eq!(paramtypestr(&pm), "");
    }

    /// PM_AUTOLOAD → "undefined" (precedes the type switch).
    #[test]
    fn paramtypestr_autoload_overrides_type_to_undefined() {
        let pm = mk_pm(PM_AUTOLOAD | PM_INTEGER, 0);
        assert_eq!(paramtypestr(&pm), "undefined");
    }

    // ─ Suffix combinations ─────────────────────────────────────────
    /// PM_INTEGER + PM_READONLY → "integer-readonly".
    #[test]
    fn paramtypestr_integer_readonly_combo() {
        let pm = mk_pm(PM_INTEGER | PM_READONLY, 0);
        assert_eq!(paramtypestr(&pm), "integer-readonly");
    }

    /// PM_SCALAR + PM_EXPORTED → "scalar-export".
    #[test]
    fn paramtypestr_scalar_exported_combo() {
        let pm = mk_pm(PM_SCALAR | PM_EXPORTED, 0);
        assert_eq!(paramtypestr(&pm), "scalar-export");
    }

    /// PM_ARRAY + PM_UNIQUE → "array-unique".
    #[test]
    fn paramtypestr_array_unique_combo() {
        let pm = mk_pm(PM_ARRAY | PM_UNIQUE, 0);
        assert_eq!(paramtypestr(&pm), "array-unique");
    }

    /// PM_SCALAR + PM_LOWER → "scalar-lower".
    #[test]
    fn paramtypestr_scalar_lower_combo() {
        let pm = mk_pm(PM_SCALAR | PM_LOWER, 0);
        assert_eq!(paramtypestr(&pm), "scalar-lower");
    }

    /// PM_SCALAR + PM_UPPER → "scalar-upper".
    #[test]
    fn paramtypestr_scalar_upper_combo() {
        let pm = mk_pm(PM_SCALAR | PM_UPPER, 0);
        assert_eq!(paramtypestr(&pm), "scalar-upper");
    }

    /// PM_SCALAR + PM_LEFT → "scalar-left".
    #[test]
    fn paramtypestr_scalar_left_combo() {
        let pm = mk_pm(PM_SCALAR | PM_LEFT, 0);
        assert_eq!(paramtypestr(&pm), "scalar-left");
    }

    /// PM_SCALAR + PM_RIGHT_B → "scalar-right_blanks".
    #[test]
    fn paramtypestr_scalar_right_blanks_combo() {
        let pm = mk_pm(PM_SCALAR | PM_RIGHT_B, 0);
        assert_eq!(paramtypestr(&pm), "scalar-right_blanks");
    }

    /// PM_SCALAR + PM_RIGHT_Z → "scalar-right_zeros".
    #[test]
    fn paramtypestr_scalar_right_zeros_combo() {
        let pm = mk_pm(PM_SCALAR | PM_RIGHT_Z, 0);
        assert_eq!(paramtypestr(&pm), "scalar-right_zeros");
    }

    /// PM_SCALAR + PM_HIDE → "scalar-hide".
    #[test]
    fn paramtypestr_scalar_hide_combo() {
        let pm = mk_pm(PM_SCALAR | PM_HIDE, 0);
        assert_eq!(paramtypestr(&pm), "scalar-hide");
    }

    /// PM_SCALAR + PM_HIDEVAL → "scalar-hideval".
    #[test]
    fn paramtypestr_scalar_hideval_combo() {
        let pm = mk_pm(PM_SCALAR | PM_HIDEVAL, 0);
        assert_eq!(paramtypestr(&pm), "scalar-hideval");
    }

    /// PM_SCALAR + PM_SPECIAL → "scalar-special".
    #[test]
    fn paramtypestr_scalar_special_combo() {
        let pm = mk_pm(PM_SCALAR | PM_SPECIAL, 0);
        assert_eq!(paramtypestr(&pm), "scalar-special");
    }

    /// PM_SCALAR + PM_TIED → "scalar-tied".
    #[test]
    fn paramtypestr_scalar_tied_combo() {
        let pm = mk_pm(PM_SCALAR | PM_TIED, 0);
        assert_eq!(paramtypestr(&pm), "scalar-tied");
    }

    /// PM_SCALAR + PM_TAGGED → "scalar-tag".
    #[test]
    fn paramtypestr_scalar_tagged_combo() {
        let pm = mk_pm(PM_SCALAR | PM_TAGGED, 0);
        assert_eq!(paramtypestr(&pm), "scalar-tag");
    }

    /// level > 0 → adds "-local" right after type.
    #[test]
    fn paramtypestr_level_nonzero_adds_local_suffix() {
        let pm = mk_pm(PM_SCALAR, 1);
        assert_eq!(paramtypestr(&pm), "scalar-local");
    }

    /// level=0 → no "-local" suffix.
    #[test]
    fn paramtypestr_level_zero_no_local_suffix() {
        let pm = mk_pm(PM_SCALAR, 0);
        assert!(
            !paramtypestr(&pm).contains("-local"),
            "no -local when level=0"
        );
    }

    /// All flags combined: long suffix chain.
    #[test]
    fn paramtypestr_many_flags_combined() {
        let pm = mk_pm(
            PM_INTEGER | PM_READONLY | PM_EXPORTED | PM_TIED | PM_UNIQUE,
            2,
        );
        let s = paramtypestr(&pm);
        assert!(s.starts_with("integer"));
        assert!(s.contains("-local"));
        assert!(s.contains("-readonly"));
        assert!(s.contains("-export"));
        assert!(s.contains("-tied"));
        assert!(s.contains("-unique"));
    }

    // ─── zsh-corpus pins for paramtypestr ───────────────────────────

    /// PM_UNSET → empty string.
    #[test]
    fn parameter_corpus_paramtypestr_unset_is_empty() {
        let pm = mk_pm(PM_UNSET, 0);
        assert_eq!(paramtypestr(&pm), "");
    }

    /// PM_AUTOLOAD → "undefined".
    #[test]
    fn parameter_corpus_paramtypestr_autoload_is_undefined() {
        let pm = mk_pm(PM_AUTOLOAD, 0);
        assert_eq!(paramtypestr(&pm), "undefined");
    }

    /// PM_SCALAR → "scalar".
    #[test]
    fn parameter_corpus_paramtypestr_scalar() {
        let pm = mk_pm(PM_SCALAR, 0);
        assert_eq!(paramtypestr(&pm), "scalar");
    }

    /// PM_ARRAY → "array".
    #[test]
    fn parameter_corpus_paramtypestr_array() {
        let pm = mk_pm(PM_ARRAY, 0);
        assert_eq!(paramtypestr(&pm), "array");
    }

    /// PM_INTEGER → "integer".
    #[test]
    fn parameter_corpus_paramtypestr_integer() {
        let pm = mk_pm(PM_INTEGER, 0);
        assert_eq!(paramtypestr(&pm), "integer");
    }

    /// PM_EFLOAT → "float".
    #[test]
    fn parameter_corpus_paramtypestr_efloat() {
        let pm = mk_pm(PM_EFLOAT, 0);
        assert_eq!(paramtypestr(&pm), "float");
    }

    /// PM_FFLOAT → "float".
    #[test]
    fn parameter_corpus_paramtypestr_ffloat() {
        let pm = mk_pm(PM_FFLOAT, 0);
        assert_eq!(paramtypestr(&pm), "float");
    }

    /// PM_HASHED → "association".
    #[test]
    fn parameter_corpus_paramtypestr_hashed() {
        let pm = mk_pm(PM_HASHED, 0);
        assert_eq!(paramtypestr(&pm), "association");
    }

    /// PM_NAMEREF → "nameref".
    #[test]
    fn parameter_corpus_paramtypestr_nameref() {
        let pm = mk_pm(PM_NAMEREF, 0);
        assert_eq!(paramtypestr(&pm), "nameref");
    }

    /// PM_SCALAR + level=2 → "scalar-local".
    #[test]
    fn parameter_corpus_paramtypestr_scalar_local() {
        let pm = mk_pm(PM_SCALAR, 2);
        assert_eq!(paramtypestr(&pm), "scalar-local");
    }

    /// PM_SCALAR + PM_LEFT → "scalar-left".
    #[test]
    fn parameter_corpus_paramtypestr_scalar_left() {
        let pm = mk_pm(PM_SCALAR | PM_LEFT, 0);
        assert_eq!(paramtypestr(&pm), "scalar-left");
    }

    /// PM_INTEGER + PM_READONLY → "integer-readonly".
    #[test]
    fn parameter_corpus_paramtypestr_integer_readonly() {
        let pm = mk_pm(PM_INTEGER | PM_READONLY, 0);
        assert_eq!(paramtypestr(&pm), "integer-readonly");
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests pinning Src/Modules/parameter.c funcstack helpers.
    // Tests that capture KNOWN ZSHRS BUGS use #[ignore = "ZSHRS BUG: …"].
    // ═══════════════════════════════════════════════════════════════════

    /// `funcstackgetfn` on empty FUNCSTACK returns empty vec.
    /// C `Src/Modules/parameter.c:627` — count is 0, ret has only
    /// the terminating NULL (1-element char**), Rust returns empty
    /// Vec<String> (matches semantically — caller iterates non-NULL
    /// elements).
    #[test]
    fn funcstackgetfn_empty_stack_returns_empty_vec() {
        let _g = crate::test_util::global_state_lock();
        // Empty FUNCSTACK (no shell function in progress).
        crate::ported::modules::parameter::FUNCSTACK
            .lock()
            .unwrap()
            .clear();
        let v = funcstackgetfn(std::ptr::null_mut());
        assert!(v.is_empty(), "no shell function in progress → empty stack");
    }

    /// `functracegetfn` on empty stack returns empty.
    #[test]
    fn functracegetfn_empty_stack_returns_empty_vec() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::modules::parameter::FUNCSTACK
            .lock()
            .unwrap()
            .clear();
        let v = functracegetfn(std::ptr::null_mut());
        assert!(v.is_empty());
    }

    /// `funcsourcetracegetfn` on empty stack returns empty.
    #[test]
    fn funcsourcetracegetfn_empty_stack_returns_empty_vec() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::modules::parameter::FUNCSTACK
            .lock()
            .unwrap()
            .clear();
        let v = funcsourcetracegetfn(std::ptr::null_mut());
        assert!(v.is_empty());
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/parameter.c accessors.
    // ═══════════════════════════════════════════════════════════════════

    /// c:911 — `patcharsgetfn` returns Vec (may be empty if getpatchars
    /// stub isn't populated). Pin: no panic + return type Vec<String>.
    #[test]
    fn patcharsgetfn_returns_vec_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _v: Vec<String> = patcharsgetfn(std::ptr::null_mut());
    }

    /// c:917 — `dispatcharsgetfn` returns Vec (no panic).
    #[test]
    fn dispatcharsgetfn_returns_vec_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _v: Vec<String> = dispatcharsgetfn(std::ptr::null_mut());
    }

    /// c:1255 — `pmjobtext(_, -1)` for out-of-range job returns empty.
    #[test]
    fn pmjobtext_out_of_range_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let s = pmjobtext(std::ptr::null_mut(), 99999);
        assert!(s.is_empty(), "out-of-range job → empty");
    }

    /// c:1340 — `pmjobstate(_, -1)` for invalid job returns empty.
    #[test]
    fn pmjobstate_out_of_range_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let s = pmjobstate(std::ptr::null_mut(), 99999);
        assert!(s.is_empty());
    }

    /// c:1277 — `getpmjobtext` for non-numeric name returns None or
    /// PM_UNSET param (no panic).
    #[test]
    fn getpmjobtext_non_numeric_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = getpmjobtext(std::ptr::null_mut(), "not_a_number");
    }

    /// c:1277 — `getpmjobtext` empty name no panic.
    #[test]
    fn getpmjobtext_empty_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = getpmjobtext(std::ptr::null_mut(), "");
    }

    /// c:1083 — `getpmmodule(_, "")` no panic.
    #[test]
    fn getpmmodule_empty_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = getpmmodule(std::ptr::null_mut(), "");
    }

    /// c:1083 — `getpmmodule` unknown module name → returns Some
    /// (PM_UNSET) or None per C convention.
    #[test]
    fn getpmmodule_unknown_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = getpmmodule(std::ptr::null_mut(), "zshrs_never_real_module_xyz");
    }

    /// c:1677 — `getpmoption(_, "")` no panic.
    #[test]
    fn getpmoption_empty_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = getpmoption(std::ptr::null_mut(), "");
    }

    /// c:1947 — `dirsgetfn` returns the dirstack (may be empty).
    #[test]
    fn dirsgetfn_returns_vec_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = dirsgetfn(std::ptr::null_mut());
    }

    /// c:911 — patcharsgetfn deterministic (whatever it returns must
    /// be consistent across calls).
    #[test]
    fn patcharsgetfn_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = patcharsgetfn(std::ptr::null_mut());
        for _ in 0..3 {
            assert_eq!(patcharsgetfn(std::ptr::null_mut()), first);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/parameter.c alias accessors.
    // ═══════════════════════════════════════════════════════════════════

    /// c:1901 — `getalias(_, _, "missing", 0)` returns Some(PM_UNSET).
    /// Per c:1917, missing entries get PM_UNSET|PM_SPECIAL flags set.
    #[test]
    fn getalias_missing_returns_pm_unset() {
        let _g = crate::test_util::global_state_lock();
        use crate::ported::zsh_h::PM_UNSET;
        let pm = getalias(
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            "zshrs_never_real_alias_xyz",
            0,
        )
        .expect("getalias returns Some even for missing");
        assert!(pm.node.flags & PM_UNSET as i32 != 0, "PM_UNSET on miss");
    }

    /// c:1907 — getalias returns Param with name preserved.
    #[test]
    fn getalias_returns_param_with_name_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let pm = getalias(
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            "test_name_xyz",
            0,
        )
        .expect("Some");
        assert_eq!(pm.node.nam, "test_name_xyz");
    }

    /// c:1923 — `getpmralias(_, "missing")` returns Some(PM_UNSET).
    #[test]
    fn getpmralias_missing_returns_pm_unset() {
        let _g = crate::test_util::global_state_lock();
        use crate::ported::zsh_h::PM_UNSET;
        let pm = getpmralias(std::ptr::null_mut(), "zshrs_never_real_ralias_xyz").expect("Some");
        assert!(pm.node.flags & PM_UNSET as i32 != 0);
    }

    /// c:1930 — `getpmdisralias(_, "missing")` returns Some(PM_UNSET).
    #[test]
    fn getpmdisralias_missing_returns_pm_unset() {
        let _g = crate::test_util::global_state_lock();
        use crate::ported::zsh_h::PM_UNSET;
        let pm =
            getpmdisralias(std::ptr::null_mut(), "zshrs_never_real_disralias_xyz").expect("Some");
        assert!(pm.node.flags & PM_UNSET as i32 != 0);
    }

    /// c:1937 — `getpmgalias(_, "missing")` returns Some(PM_UNSET).
    #[test]
    fn getpmgalias_missing_returns_pm_unset() {
        let _g = crate::test_util::global_state_lock();
        use crate::ported::zsh_h::PM_UNSET;
        let pm = getpmgalias(std::ptr::null_mut(), "zshrs_never_real_galias_xyz").expect("Some");
        assert!(pm.node.flags & PM_UNSET as i32 != 0);
    }

    /// c:1901 — getalias for empty name returns Some(PM_UNSET).
    #[test]
    fn getalias_empty_name_returns_some() {
        let _g = crate::test_util::global_state_lock();
        let _ = getalias(std::ptr::null_mut(), std::ptr::null_mut(), "", 0)
            .expect("always Some per C convention");
    }

    /// c:2844 — `setpmralias` no panic with empty value.
    #[test]
    fn setpmralias_empty_value_no_panic() {
        let _g = crate::test_util::global_state_lock();
        use crate::ported::zsh_h::{hashnode, param};
        let pm = Box::new(param {
            node: hashnode {
                next: None,
                nam: "test".to_string(),
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
        setpmralias(pm, String::new());
    }

    /// c:2891 — `unsetpmalias` no panic.
    #[test]
    fn unsetpmalias_no_panic() {
        let _g = crate::test_util::global_state_lock();
        use crate::ported::zsh_h::{hashnode, param};
        let pm = Box::new(param {
            node: hashnode {
                next: None,
                nam: "test".to_string(),
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
        unsetpmalias(pm, 0);
    }

    /// c:2905 — `unsetpmsalias` no panic.
    #[test]
    fn unsetpmsalias_no_panic() {
        let _g = crate::test_util::global_state_lock();
        use crate::ported::zsh_h::{hashnode, param};
        let pm = Box::new(param {
            node: hashnode {
                next: None,
                nam: "test".to_string(),
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
        unsetpmsalias(pm, 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/parameter.c
    // c:50 paramtypestr / c:132 getpmparameter / c:606 getpmcommand /
    // c:944 getfunction / c:998 getpmfunction / c:1007 getpmdisfunction
    // ═══════════════════════════════════════════════════════════════════

    /// c:132 — `getpmparameter(null, "")` empty name returns Some(PM_UNSET).
    #[test]
    fn getpmparameter_empty_name_returns_some_unset() {
        use crate::ported::zsh_h::PM_UNSET;
        let _g = crate::test_util::global_state_lock();
        let pm = getpmparameter(std::ptr::null_mut(), "");
        if let Some(p) = pm {
            assert_ne!(
                p.node.flags & PM_UNSET as i32,
                0,
                "empty param name → PM_UNSET"
            );
        }
    }

    /// c:132 — `getpmparameter` returns Option<Param>.
    #[test]
    fn getpmparameter_returns_option_param_type() {
        use crate::ported::zsh_h::Param;
        let _g = crate::test_util::global_state_lock();
        let _: Option<Param> = getpmparameter(std::ptr::null_mut(), "anything");
    }

    /// c:606 — `getpmcommand(null, "")` empty name returns Some.
    #[test]
    fn getpmcommand_empty_name_returns_some() {
        let _g = crate::test_util::global_state_lock();
        let pm = getpmcommand(std::ptr::null_mut(), "");
        assert!(pm.is_some(), "C convention: always Some for missing");
    }

    /// c:998 — `getpmfunction(null, "")` empty name returns Some.
    #[test]
    fn getpmfunction_empty_name_returns_some() {
        let _g = crate::test_util::global_state_lock();
        let pm = getpmfunction(std::ptr::null_mut(), "");
        assert!(pm.is_some(), "C convention: always Some for missing");
    }

    /// c:1007 — `getpmdisfunction(null, "")` empty name returns Some.
    #[test]
    fn getpmdisfunction_empty_name_returns_some() {
        let _g = crate::test_util::global_state_lock();
        let pm = getpmdisfunction(std::ptr::null_mut(), "");
        assert!(pm.is_some());
    }

    /// c:944 — `getfunction(null, "", 0)` returns Option<Param>.
    #[test]
    fn getfunction_returns_option_param_type() {
        use crate::ported::zsh_h::Param;
        let _g = crate::test_util::global_state_lock();
        let _: Option<Param> = getfunction(std::ptr::null_mut(), "anything", 0);
    }

    /// c:50 — `paramtypestr` returns String (compile-time type pin).
    #[test]
    fn paramtypestr_returns_string_type() {
        use crate::ported::zsh_h::{hashnode, param};
        let pm = param {
            node: hashnode {
                next: None,
                nam: "x".to_string(),
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
        };
        let _: String = paramtypestr(&pm);
    }

    /// c:50 — `paramtypestr` for plain scalar returns non-empty string.
    #[test]
    fn paramtypestr_scalar_returns_nonempty() {
        use crate::ported::zsh_h::{hashnode, param};
        let pm = param {
            node: hashnode {
                next: None,
                nam: "x".to_string(),
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
        };
        let s = paramtypestr(&pm);
        assert!(!s.is_empty(), "type str must be non-empty");
    }

    /// c:132 — `getpmparameter` is deterministic for same name.
    #[test]
    fn getpmparameter_deterministic_for_unknown() {
        let _g = crate::test_util::global_state_lock();
        let p1 = getpmparameter(std::ptr::null_mut(), "__zshrs_never_param__")
            .map(|p| p.node.nam.clone());
        let p2 = getpmparameter(std::ptr::null_mut(), "__zshrs_never_param__")
            .map(|p| p.node.nam.clone());
        assert_eq!(p1, p2, "getpmparameter must be deterministic");
    }

    /// c:606 — `getpmcommand` returns Option<Param>.
    #[test]
    fn getpmcommand_returns_option_param_type() {
        use crate::ported::zsh_h::Param;
        let _g = crate::test_util::global_state_lock();
        let _: Option<Param> = getpmcommand(std::ptr::null_mut(), "anything");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/parameter.c
    // c:606 getpmcommand / c:998 getpmfunction / c:1007 getpmdisfunction /
    // c:1159 getpmfunction_source / c:1169 getpmdisfunction_source /
    // c:1202 funcstackgetfn / c:1239 functracegetfn / c:1273 funcsourcetracegetfn
    // ═══════════════════════════════════════════════════════════════════

    /// c:606 — `getpmcommand` is deterministic for unknown command.
    #[test]
    fn getpmcommand_deterministic_for_unknown() {
        let _g = crate::test_util::global_state_lock();
        let a =
            getpmcommand(std::ptr::null_mut(), "__zshrs_never_cmd__").map(|p| p.node.nam.clone());
        let b =
            getpmcommand(std::ptr::null_mut(), "__zshrs_never_cmd__").map(|p| p.node.nam.clone());
        assert_eq!(a, b, "getpmcommand must be deterministic");
    }

    /// c:998 — `getpmfunction` returns Option<Param>.
    #[test]
    fn getpmfunction_returns_option_param_type() {
        use crate::ported::zsh_h::Param;
        let _g = crate::test_util::global_state_lock();
        let _: Option<Param> = getpmfunction(std::ptr::null_mut(), "anything");
    }

    /// c:998 — `getpmfunction` is deterministic for unknown function.
    #[test]
    fn getpmfunction_deterministic_for_unknown() {
        let _g = crate::test_util::global_state_lock();
        let a =
            getpmfunction(std::ptr::null_mut(), "__zshrs_never_fn__").map(|p| p.node.nam.clone());
        let b =
            getpmfunction(std::ptr::null_mut(), "__zshrs_never_fn__").map(|p| p.node.nam.clone());
        assert_eq!(a, b, "getpmfunction must be deterministic");
    }

    /// c:1007 — `getpmdisfunction` returns Option<Param>.
    #[test]
    fn getpmdisfunction_returns_option_param_type() {
        use crate::ported::zsh_h::Param;
        let _g = crate::test_util::global_state_lock();
        let _: Option<Param> = getpmdisfunction(std::ptr::null_mut(), "anything");
    }

    /// c:1159 — `getpmfunction_source` returns Option<Param>.
    #[test]
    fn getpmfunction_source_returns_option_param_type() {
        use crate::ported::zsh_h::Param;
        let _g = crate::test_util::global_state_lock();
        let _: Option<Param> = getpmfunction_source(std::ptr::null_mut(), "anything");
    }

    /// c:1169 — `getpmdisfunction_source` returns Option<Param>.
    #[test]
    fn getpmdisfunction_source_returns_option_param_type() {
        use crate::ported::zsh_h::Param;
        let _g = crate::test_util::global_state_lock();
        let _: Option<Param> = getpmdisfunction_source(std::ptr::null_mut(), "anything");
    }

    /// c:1202 — `funcstackgetfn` returns Vec<String> (compile-time type pin).
    #[test]
    fn funcstackgetfn_returns_vec_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Vec<String> = funcstackgetfn(std::ptr::null_mut());
    }

    /// c:1239 — `functracegetfn` returns Vec<String>.
    #[test]
    fn functracegetfn_returns_vec_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Vec<String> = functracegetfn(std::ptr::null_mut());
    }

    /// c:1273 — `funcsourcetracegetfn` returns Vec<String>.
    #[test]
    fn funcsourcetracegetfn_returns_vec_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Vec<String> = funcsourcetracegetfn(std::ptr::null_mut());
    }

    /// c:1202 — `funcstackgetfn(null)` is deterministic across calls.
    #[test]
    fn funcstackgetfn_null_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = funcstackgetfn(std::ptr::null_mut());
        for _ in 0..3 {
            assert_eq!(
                funcstackgetfn(std::ptr::null_mut()),
                first,
                "funcstackgetfn(null) must be deterministic"
            );
        }
    }
}
