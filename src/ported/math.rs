//! Mathematical expression evaluation for zshrs
//!
//! Direct port from zsh/Src/math.c
//!
//! Supports:
//! - Integer and floating point arithmetic
//! - All C operators (+, -, *, /, %, <<, >>, &, |, ^, etc.)
//! - Zsh ** power operator
//! - Comparison operators (<, >, <=, >=, ==, !=)
//! - Logical operators (&&, ||, !)
//! - Ternary operator (? :)
//! - Assignment operators (=, +=, -=, *=, /=, etc.)
//! - Pre/post increment/decrement (++, --)
//! - Base conversion (`16#FF`, `2#1010`, `[16]FF`)
//! - Special values (Inf, NaN)
//! - Variable references and assignment

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use crate::ported::options::opt_state_set;
use crate::ported::params::{convbase, getsparam, unsetparam};
use crate::ported::utils::zerr;
/// Re-export of `mnumber` (defined in zsh_h.rs as the Src/zsh.h:95 port).
pub use crate::ported::zsh_h::{mnumber, Nularg, MN_FLOAT, MN_INTEGER, MN_UNSET};
use crate::zsh_h::{PM_ARRAY, PM_EFLOAT, PM_FFLOAT, PM_HASHED, PM_INTEGER, PM_TYPE};

/// Re-export of `MN_FLOAT` (defined in zsh_h.rs as the Src/zsh.h:104 port).
/// Re-export of `MN_INTEGER` (defined in zsh_h.rs as the Src/zsh.h:103 port).
/// Re-export of `MN_UNSET` (defined in zsh_h.rs as the Src/zsh.h:105 port).

/// Port of `struct mathvalue` from `Src/math.c`:
///
/// ```c
/// struct mathvalue {
///     char *lval;     /* lvalue string for variable write-back  */
///     Value pval;     /* resolved variable handle (or NULL)     */
///     mnumber val;    /* current numeric value                  */
/// };
/// ```
#[derive(Clone)]
pub(crate) struct mathvalue {
    pub val: mnumber,
    pub lval: Option<String>,
    /// `Value pval` slot from the C source. zsh uses it to cache the
    /// resolved parameter handle so write-back doesn't re-parse the
    /// `lval` string. Rust port leaves this as `()` for now — the
    /// resolved variable lives in `crate::ported::exec::ShellExecutor`'s
    /// `variables` map, looked up by `lval` on each access.
    pub pval: (),
}

/// Operator associativity and type flags
const LR: u16 = 0x0000; // left-to-right
const RL: u16 = 0x0001; // right-to-left
const BOOL: u16 = 0x0002; // short-circuit boolean

const OP_A2: u16 = 0x0004; // 2 arguments
const OP_A2IR: u16 = 0x0008; // 2 args, return int
const OP_A2IO: u16 = 0x0010; // 2 args, must be int
const OP_E2: u16 = 0x0020; // 2 args with assignment
const OP_E2IO: u16 = 0x0040; // 2 args assign, must be int
const OP_OP: u16 = 0x0080; // expecting operator position
const OP_OPF: u16 = 0x0100; // followed by operator (after this, next is operator)

/// Math tokens — direct port of Src/math.c:109-162. C uses bare
/// `#define`s; the Rust port mirrors as `pub const` ints so
/// `static int mtok` (math.c:305) can be a plain `i32` and the
/// C precedence/type tables index by the literal numbers.
pub const M_INPAR: i32 = 0; // c:109  '('
/// `M_OUTPAR` constant.
pub const M_OUTPAR: i32 = 1; // c:110  ')'
/// `NOT` constant.
pub const NOT: i32 = 2; // c:111  '!'
/// `COMP` constant.
pub const COMP: i32 = 3; // c:112  '~'
/// `POSTPLUS` constant.
pub const POSTPLUS: i32 = 4; // c:113  x++
/// `POSTMINUS` constant.
pub const POSTMINUS: i32 = 5; // c:114  x--
/// `UPLUS` constant.
pub const UPLUS: i32 = 6; // c:115  +x
/// `UMINUS` constant.
pub const UMINUS: i32 = 7; // c:116  -x
/// `AND` constant.
pub const AND: i32 = 8; // c:117  &
/// `XOR` constant.
pub const XOR: i32 = 9; // c:118  ^
/// `OR` constant.
pub const OR: i32 = 10; // c:119  |
/// `MUL` constant.
pub const MUL: i32 = 11; // c:120  *
/// `DIV` constant.
pub const DIV: i32 = 12; // c:121  /
/// `MOD` constant.
pub const MOD: i32 = 13; // c:122  %
/// `PLUS` constant.
pub const PLUS: i32 = 14; // c:123  +
/// `MINUS` constant.
pub const MINUS: i32 = 15; // c:124  -
/// `SHLEFT` constant.
pub const SHLEFT: i32 = 16; // c:125  <<
/// `SHRIGHT` constant.
pub const SHRIGHT: i32 = 17; // c:126  >>
/// `LES` constant.
pub const LES: i32 = 18; // c:127  <
/// `LEQ` constant.
pub const LEQ: i32 = 19; // c:128  <=
/// `GRE` constant.
pub const GRE: i32 = 20; // c:129  >
/// `GEQ` constant.
pub const GEQ: i32 = 21; // c:130  >=
/// `DEQ` constant.
pub const DEQ: i32 = 22; // c:131  ==
/// `NEQ` constant.
pub const NEQ: i32 = 23; // c:132  !=
/// `DAND` constant.
pub const DAND: i32 = 24; // c:133  &&
/// `DOR` constant.
pub const DOR: i32 = 25; // c:134  ||
/// `DXOR` constant.
pub const DXOR: i32 = 26; // c:135  ^^
/// `QUEST` constant.
pub const QUEST: i32 = 27; // c:136  ? (ternary)
/// `COLON` constant.
pub const COLON: i32 = 28; // c:137  :
/// `EQ` constant.
pub const EQ: i32 = 29; // c:138  =
/// `PLUSEQ` constant.
pub const PLUSEQ: i32 = 30; // c:139  +=
/// `MINUSEQ` constant.
pub const MINUSEQ: i32 = 31; // c:140  -=
/// `MULEQ` constant.
pub const MULEQ: i32 = 32; // c:141  *=
/// `DIVEQ` constant.
pub const DIVEQ: i32 = 33; // c:142  /=
/// `MODEQ` constant.
pub const MODEQ: i32 = 34; // c:143  %=
/// `ANDEQ` constant.
pub const ANDEQ: i32 = 35; // c:144  &=
/// `XOREQ` constant.
pub const XOREQ: i32 = 36; // c:145  ^=
/// `OREQ` constant.
pub const OREQ: i32 = 37; // c:146  |=
/// `SHLEFTEQ` constant.
pub const SHLEFTEQ: i32 = 38; // c:147  <<=
/// `SHRIGHTEQ` constant.
pub const SHRIGHTEQ: i32 = 39; // c:148  >>=
/// `DANDEQ` constant.
pub const DANDEQ: i32 = 40; // c:149  &&=
/// `DOREQ` constant.
pub const DOREQ: i32 = 41; // c:150  ||=
/// `DXOREQ` constant.
pub const DXOREQ: i32 = 42; // c:151  ^^=
/// `COMMA` constant.
pub const COMMA: i32 = 43; // c:152  ,
/// `EOI` constant.
pub const EOI: i32 = 44; // c:153  end of input
/// `PREPLUS` constant.
pub const PREPLUS: i32 = 45; // c:154  ++x
/// `PREMINUS` constant.
pub const PREMINUS: i32 = 46; // c:155  --x
/// `NUM` constant.
pub const NUM: i32 = 47; // c:156  number literal
/// `ID` constant.
pub const ID: i32 = 48; // c:157  identifier
/// `POWER` constant.
pub const POWER: i32 = 49; // c:158  **
/// `CID` constant.
pub const CID: i32 = 50; // c:159  #identifier (char value)
/// `POWEREQ` constant.
pub const POWEREQ: i32 = 51; // c:160  **=
/// `FUNC` constant.
pub const FUNC: i32 = 52; // c:161  function call
/// Total token count — Src/math.c:162 `#define TOKCOUNT 53`. The
/// `c_prec`/`z_prec`/`type` arrays are sized by this.
pub const TOKCOUNT: usize = 53;

/// Port of `enum prec_type` from `Src/math.c`. `mathevall()` (line
/// 367) uses this to differentiate top-level expression evaluation
/// (`(())`, `$(())`) from function-argument evaluation
/// (`func(arg, arg, …)`) — argument-mode terminates parsing on
/// the first comma encountered at the top level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum prec_type {
    MPREC_TOP,
    MPREC_ARG,
}

/// Port of `getmathparam(struct mathvalue *mptr)` from `Src/math.c:337`.
///
/// Look up a parameter by name from inside math context. zsh
/// auto-typesets a missing-but-referenced name (its mathparam
/// flag), but the Rust port keeps the variables map separate from
/// the param table so a miss returns `Integer(0)` and skips the
/// type-coercion. Indirect-string mode (`a="3+2"; $((a))`) is
/// handled by recursively evaluating the string value.
/// WARNING: param names don't match C — Rust=() vs C=(mptr)
pub(crate) fn getmathparam(name: &str) -> mnumber {
    // c:Src/math.c:358-362 — after reading a parameter's value, FORCEFLOAT
    // coerces an INTEGER result to float, so `integer a=3 b=4; setopt
    // force_float; $((a/b))` does float division (0.75) not integer (0).
    // The literal/operator paths already honor force_float; only this
    // param-read path dropped it. The read logic is wrapped in an
    // immediately-invoked closure (NOT a named inner fn — the name-parity
    // build gate requires every `fn` to have a C counterpart, and there is
    // only one C `getmathparam`).
    let __raw = (|| -> mnumber {
        // Strip array subscript if present
        let base_name = if let Some(bracket) = name.find('[') {
            &name[..bracket]
        } else {
            name
        };
        if let Some(v) = m_variables_get(base_name) {
            return v;
        }
        // c:Src/math.c:337 getmathparam — falls back to `getvalue(s)`
        // which parses the full subscript syntax (params.c:2180).
        // The Rust port previously required callers to seed
        // `with_string_variables` (a pre-populate pattern that
        // diverged from C). Read paramtab + array subscripts here
        // so matheval works without seeding.
        if let Some(bracket) = name.find('[') {
            let close = name.rfind(']').unwrap_or(name.len());
            let arr_name = &name[..bracket];
            let idx_str = &name[bracket + 1..close];

            // c:Src/params.c::getarg — subscript-flag form `(i)pat` /
            // `(I)pat` inside an arith subscript: search the array for
            // `pat` and return the 1-based index (or len+1 / 0 for
            // miss). Bug #341. The other flag arms (`r`/`R` returning
            // strings, `n`/`b`/`e`/`w`/`s` etc.) don't yield arith
            // values, so we only handle `i`/`I` here.
            // Flag block `(flags)pat`: `i`/`I` make it an index search
            // (forward / reverse), an optional `e` modifier forces EXACT
            // (literal) compare instead of glob match. Accepts `(i)`,`(I)`,
            // `(ie)`,`(Ie)`,`(ei)`,… so membership tests like
            // `(( arr[(Ie)$x] ))` resolve. Other flag letters (r/R/n/b/w/s)
            // don't yield an arith value and fall through.
            if idx_str.starts_with('(') {
                if let Some(close) = idx_str.find(')') {
                    let flags = &idx_str[1..close];
                    let pat = &idx_str[close + 1..];
                    let is_index = flags.contains('i') || flags.contains('I');
                    if is_index && flags.chars().all(|c| matches!(c, 'i' | 'I' | 'e' | 'n')) {
                        let reverse = flags.contains('I');
                        let exact = flags.contains('e');
                        let matches_elem = |e: &str| -> bool {
                            if exact {
                                e == pat
                            } else {
                                crate::ported::pattern::patcompile(
                                    &{
                                        let mut t = pat.to_string();
                                        crate::ported::glob::tokenize(&mut t);
                                        t
                                    },
                                    crate::ported::zsh_h::PAT_HEAPDUP as i32,
                                    None,
                                )
                                .map_or(e == pat, |p| crate::ported::pattern::pattry(&p, e))
                            }
                        };
                        if let Ok(tab) = crate::ported::params::paramtab().read() {
                            if let Some(pm) = tab.get(arr_name) {
                                if let Some(arr) = &pm.u_arr {
                                    let len = arr.len() as i64;
                                    let mut found: i64 = if reverse { 0 } else { len + 1 };
                                    let it: Vec<(usize, &String)> = if reverse {
                                        arr.iter().enumerate().rev().collect()
                                    } else {
                                        arr.iter().enumerate().collect()
                                    };
                                    for (i, e) in it {
                                        if matches_elem(e) {
                                            found = (i + 1) as i64;
                                            break;
                                        }
                                    }
                                    return mnumber {
                                        l: found,
                                        d: 0.0,
                                        type_: MN_INTEGER,
                                    };
                                }
                            }
                        }
                        return mnumber {
                            l: 0,
                            d: 0.0,
                            type_: MN_INTEGER,
                        };
                    }
                }
            }

            // Recursively eval the index (so a[i+1], h[$k], etc work).
            // CRITICAL: save/restore evaluator state around the recursive
            // matheval — without this, the inner call's `push(idx_value)`
            // contaminates the OUTER expression's operand stack. Bug
            // manifested as `$((1 + arr[1]))` returning 10 (just arr[1])
            // because the outer NUM(1) got popped by the inner eval's
            // op() during `op(PLUS)` (which sees [NUM(1), ID(arr[1]),
            // NUM(1_from_idx_eval)] instead of [NUM(1), ID(arr[1])]).
            // C mathevall at math.c:367 does the same xyy* save/restore
            // around recursive entry.
            let saved = save_state();
            let idx_val = matheval(idx_str)
                .map(|n| if n.type_ == MN_FLOAT { n.d as i64 } else { n.l })
                .unwrap_or(0);
            restore_state(saved);
            // Read paramtab directly: PM_ARRAY → u_arr indexed by 1-based pos.
            if let Ok(tab) = crate::ported::params::paramtab().read() {
                if let Some(pm) = tab.get(arr_name) {
                    if let Some(arr) = &pm.u_arr {
                        let len = arr.len() as i64;
                        // !!! BASH-MODE GATE (no C counterpart) !!! bash
                        // indexed arrays are 0-based in arithmetic too
                        // (`$(( a[1] ))` is the SECOND element), so skip the
                        // zsh 1-based `-1`. Negative indices count from the
                        // end identically in both, so only the non-negative
                        // arm differs. Mirrors the param-expansion 0-based
                        // subscript already applied in --bash.
                        let pos = if idx_val < 0 {
                            len + idx_val
                        } else if crate::dash_mode::bash_mode() {
                            idx_val
                        } else {
                            idx_val - 1
                        };
                        if pos >= 0 && (pos as usize) < arr.len() {
                            let raw = &arr[pos as usize];
                            if let Ok(n) = raw.parse::<i64>() {
                                return mnumber {
                                    l: n,
                                    d: 0.0,
                                    type_: MN_INTEGER,
                                };
                            }
                            if let Ok(f) = raw.parse::<f64>() {
                                return mnumber {
                                    l: 0,
                                    d: f,
                                    type_: MN_FLOAT,
                                };
                            }
                        }
                    }
                }
            }
            // c:Src/math.c:337 getmathparam → `getvalue(&vbuf, &s, 1)`
            // (params.c:2180). C resolves the subscript through the
            // PARAMETER'S OWN gsu, so a hash whose individual keys are
            // live gsu cells returns the COMPUTED value in arithmetic
            // context exactly as it does in string context. The single
            // case that matters is `compstate[nmatches]`, whose getter
            // is `get_nmatches` (Src/Zle/complete.c:1411 —
            // `permmatches(0) ? 0 : nmatches`) and which is never
            // stored data.
            //
            // The port read `paramtab_hashed_storage()` directly here,
            // which never holds that key, so arithmetic always saw 0
            // while `${compstate[nmatches]}` (subst.rs:6814) saw the
            // live count. The compsys idiom
            //   nm="$compstate[nmatches]" … [[ nm -ne compstate[nmatches] ]]
            // (`_alternative` sh:63, `_arguments`, `_describe`,
            // `_parameters`) therefore ALWAYS concluded "this completer
            // added nothing": `_alternative` returned 1 after adding
            // 52k matches, so `_complete` returned 1, `_megacomplete`
            // returned 1, and `_main_complete` re-ran the whole
            // completer for the next matcher-list entry — doubling the
            // work on every `_alternative`-based completion.
            //
            // `assoc_key_hit` (vm_helper.rs:113) is the existing shared
            // accessor that already applies C's gsu semantics for this
            // hash; route through it rather than adding a fourth copy
            // of the special-case. It falls back to the same store read
            // below when it declines (name shadowed by a `local`).
            if let Some((_, Some(v))) = crate::vm_helper::assoc_key_hit(arr_name, idx_str) {
                if let Ok(n) = v.parse::<i64>() {
                    return mnumber {
                        l: n,
                        d: 0.0,
                        type_: MN_INTEGER,
                    };
                }
                if let Ok(f) = v.parse::<f64>() {
                    return mnumber {
                        l: 0,
                        d: f,
                        type_: MN_FLOAT,
                    };
                }
            }
            // PM_HASHED via paramtab_hashed_storage.
            if let Ok(m) = crate::ported::params::paramtab_hashed_storage().lock() {
                if let Some(map) = m.get(arr_name) {
                    if let Some(v) = map.get(idx_str) {
                        if let Ok(n) = v.parse::<i64>() {
                            return mnumber {
                                l: n,
                                d: 0.0,
                                type_: MN_INTEGER,
                            };
                        }
                        if let Ok(f) = v.parse::<f64>() {
                            return mnumber {
                                l: 0,
                                d: f,
                                type_: MN_FLOAT,
                            };
                        }
                    }
                }
            }
            // c:Src/math.c:337 getmathparam → getvalue: magic-assoc special
            // parameters (`sysparams`, `errnos`, `commands`, …) don't live in
            // paramtab — their per-key value comes from a module getfn, the
            // same PARTAB dispatch string context uses (subst.rs:8102). Route
            // through it so `(( sysparams[pid] ))` yields the pid instead of 0.
            // gitstatus_start relies on this (plugin line 649 / 593), so the
            // whole zsh/system-backed init path (p10k's git prompt) was dead.
            if let Some(e_) = crate::ported::modules::parameter::PARTAB
                .iter()
                .find(|e_| e_.name == arr_name)
            {
                let module_ok = match e_.module {
                    Some(m_) => crate::ported::module::MODULESTAB
                        .lock()
                        .map(|t| t.is_loaded(m_))
                        .unwrap_or(false),
                    None => true,
                };
                if module_ok && !crate::vm_helper::magic_special_shadowed(arr_name) {
                    // c:Src/params.c:589-594 getparamnode → c:563-585 loadparamnode —
                    // an arithmetic read of `special[key]` resolves the CONTAINER
                    // name, clearing its PM_AUTOLOAD; the subscript is irrelevant.
                    crate::vm_helper::mark_module_param_used(arr_name);
                    if let Some(v) =
                        (e_.getfn)(std::ptr::null_mut(), idx_str).and_then(|p_| p_.u_str)
                    {
                        if let Ok(n) = v.parse::<i64>() {
                            return mnumber {
                                l: n,
                                d: 0.0,
                                type_: MN_INTEGER,
                            };
                        }
                        if let Ok(f) = v.parse::<f64>() {
                            return mnumber {
                                l: 0,
                                d: f,
                                type_: MN_FLOAT,
                            };
                        }
                    }
                }
            }
            return mnumber {
                l: 0,
                d: 0.0,
                type_: MN_INTEGER,
            };
        }
        if let Some(raw) = getsparam(base_name) {
            if let Ok(n) = raw.parse::<i64>() {
                return mnumber {
                    l: n,
                    d: 0.0,
                    type_: MN_INTEGER,
                };
            }
            if let Ok(f) = raw.parse::<f64>() {
                return mnumber {
                    l: 0,
                    d: f,
                    type_: MN_FLOAT,
                };
            }
            // c:Src/math.c:337 getmathparam — falls back to recursively
            // evaluating the raw string as an arith expression. zsh: a
            // scalar holding `0xff` / `0b101` / `3+2` / `1e3` all evaluate
            // when used in arith context. Direct path through `matheval`
            // gives lexconstant + parser its full integer-base + float
            // handling.
            let saved = save_state();
            let inherited_strs = saved.string_variables.clone();
            new(&raw);
            m_variables_set(saved.variables.clone());
            let mut strs = inherited_strs;
            strs.remove(base_name);
            m_string_variables_set(strs);
            m_prec_set(saved.prec);
            m_c_precedences_set(saved.c_precedences);
            let result = mathevall();
            // c:Src/math.c::matheval — when the recursive eval errors
            // (e.g. raw is "42xyz" with trailing junk), preserve the error
            // message so it propagates to the outer arith caller instead
            // of being clobbered by restore_state. zsh: `a="42xyz";
            // $((a+1))` → "bad math expression: operator expected at
            // `xyz'" rc=1. zshrs previously swallowed the error and
            // returned 0 (then +1 = 1) silently. Bug #494.
            let err_to_propagate = match &result {
                Err(msg) => Some(msg.clone()),
                Ok(_) => None,
            };
            restore_state(saved);
            if let Ok(r) = result {
                return r;
            }
            if let Some(msg) = err_to_propagate {
                m_error_set(msg);
            }
            // Non-numeric and non-evaluable string: fall through.
        }
        // Recursive eval: if the var holds a non-numeric string, evaluate
        // it AS an arith expression. zsh: `a="3+2"; $((a))` → 5. Bound
        // to one level of indirection — fresh evaluator each call so we
        // don't accidentally pollute s.variables.
        if let Some(raw) = m_string_variables_get(base_name) {
            // Save parent's eval state — `new(&raw)` resets thread_locals
            // for the sub-eval, which would otherwise clobber the parent.
            // Mirrors C `mathevall()` xyy* save/restore pattern (math.c:367).
            let saved = save_state();
            // Inherit caller's variables/string_variables/prec into the
            // sub-eval, with `base_name` removed from the indirect map to
            // prevent infinite recursion on `a="$a"`-style cycles.
            let inherited_vars = saved.variables.clone();
            let mut inherited_strs = saved.string_variables.clone();
            inherited_strs.remove(base_name);
            let inherited_prec = saved.prec;
            let inherited_c_prec = saved.c_precedences;

            new(&raw);
            m_variables_set(inherited_vars);
            m_string_variables_set(inherited_strs);
            m_prec_set(inherited_prec);
            m_c_precedences_set(inherited_c_prec);

            let result = mathevall();
            restore_state(saved);
            if let Ok(r) = result {
                return r;
            }
        }
        // c:Src/math.c:345-346 — `if (unset(UNSET)) zerr("%s: parameter
        // not set", mptr->lval);`. When `nounset` is set (i.e., the
        // canonical `UNSET` option is OFF), referring to an unset
        // parameter in arith context is an error. Bug #88 in
        // docs/BUGS.md: zshrs silently used 0, masking typos and
        // breaking defensive `set -u` scripts.
        if !crate::ported::zsh_h::isset(crate::ported::zsh_h::UNSET) {
            crate::ported::utils::zerr(&format!("{}: parameter not set", name));
        }
        mnumber {
            l: 0,
            d: 0.0,
            type_: MN_INTEGER,
        }
    })();
    if m_force_float() && __raw.type_ == MN_INTEGER {
        // c:359-362 — coerce integer → float under FORCEFLOAT.
        return mnumber {
            l: 0,
            d: __raw.l as f64,
            type_: MN_FLOAT,
        };
    }
    __raw
}

/// Evaluate the expression
/// Port of `mathevall(char *s, enum prec_type prec_tp, char **ep)` from `Src/math.c:367`.
/// WARNING: param names don't match C — Rust=() vs C=(s, prec_tp, ep)
pub(crate) fn mathevall() -> Result<mnumber, String> {
    // c:Src/math.c — matheval reads `isset(CPRECEDENCES)` / `isset(FORCEFLOAT)`
    // / `isset(OCTALZEROES)` live at its use sites (e.g. c:348, 359, 482). The
    // zshrs port caches them in per-eval thread-locals for speed but never
    // populated them, so `setopt forcefloat` / `cprecedences` / `octalzeroes`
    // had no effect inside arithmetic. Sync the caches from the live options at
    // each eval entry (the option can't change mid-expression).
    m_c_precedences_set(crate::ported::zsh_h::isset(
        crate::ported::zsh_h::CPRECEDENCES,
    ));
    m_force_float_set(crate::ported::zsh_h::isset(
        crate::ported::zsh_h::FORCEFLOAT,
    ));
    m_octal_zeroes_set(crate::ported::zsh_h::isset(
        crate::ported::zsh_h::OCTALZEROES,
    ));
    m_prec_set(if m_c_precedences() { &C_PREC } else { &Z_PREC });

    // c:386/446 — `if (mlevel++)` … `if (--mlevel)` bracket the evaluator.
    // The output radix is deliberately NOT reset here: C clears it only in
    // `matheval` and only when `mlevel` is 0 (c:1486), i.e. exactly once per
    // TOP-LEVEL expression. `mathevall` is re-entered for every nested
    // evaluation — including the recursive re-eval of a scalar parameter whose
    // value is itself a math expression (`j=8#62; $(( [#36] j ))`, getmathparam
    // below) — so resetting here wiped the outer `[#36]` and printed base 10.
    // That is what the C comment at c:1485 means by "maintain outputradix and
    // outputunderscore across levels of evaluation".
    let _mlevel = MathLevel::enter();

    // c:Src/math.c — bound recursive re-evaluation. A scalar whose
    // value references itself in arithmetic — `x=x`, `x="1+x"`, or a
    // cycle `x=y; y=x` — makes getmathparam re-enter mathevall on the
    // same value without end (getsparam returns the self-referential
    // string, mathevall re-parses it, getmathparam resolves the var
    // again …). Unbounded, that recursion overruns the stack and the
    // whole shell dies with SIGBUS/SIGABRT rather than erroring. zsh
    // caps `mlevel` and bails with a diagnostic; match it so the eval
    // fails cleanly (0 result) instead of crashing. thefuck's config
    // tripped this the moment the cmd-subst deadlock that used to mask
    // it was fixed.
    const MAX_MLEVEL: i32 = 256; // c:Src/math.c MAX_MLEVEL
    if M_LEVEL.with(|c| c.get()) > MAX_MLEVEL {
        let expr = m_input_clone();
        return Err(format!("math recursion limit exceeded: {}", expr.trim()));
    }

    // Skip leading whitespace and Nularg
    while let Some(c) = peek() {
        if c.is_whitespace() || c == '\u{a1}' {
            advance();
        } else {
            break;
        }
    }

    if m_pos() >= m_input_len() {
        return Ok(mnumber {
            l: 0,
            d: 0.0,
            type_: MN_INTEGER,
        });
    }

    mathparse(top_prec());

    if let Some(err) = m_error_take() {
        return Err(err);
    }

    // Check for trailing characters
    while let Some(c) = peek() {
        if c.is_whitespace() {
            advance();
        } else if c == ')' {
            // zsh's specific wording for the unmatched-close
            // case: `bad math expression: unexpected ')'`.
            return Err("bad math expression: unexpected ')'".to_string());
        } else {
            // c:1498-1499 — `if (*junk) zerr("bad math expression:
            // illegal character: %c", *junk);`
            return Err(format!("bad math expression: illegal character: {}", c));
        }
    }

    if m_stack_is_empty() {
        return Ok(mnumber {
            l: 0,
            d: 0.0,
            type_: MN_INTEGER,
        });
    }

    let mv = m_stack_pop().unwrap();
    let result = if (mv.val.type_ == MN_UNSET) {
        if let Some(ref name) = mv.lval {
            getmathparam(name)
        } else {
            mnumber {
                l: 0,
                d: 0.0,
                type_: MN_INTEGER,
            }
        }
    } else {
        mv.val
    };

    // c:Src/math.c:425-444 — `if (errflag) { ret = 0; }` and the
    // caller checks `errflag` externally to detect failure. The Rust
    // port carries the error in the Result instead of a side-channel
    // errflag, so any m_error_set inside getmathparam (e.g. the
    // recursive-eval arm at math.rs:384 for `a="/usr/bin"; (( a ))`)
    // must surface as Err here rather than being swallowed by the
    // unconditional Ok(result) below. Without this check, scalar
    // params whose values fail recursive math parsing silently
    // resolved to 0, breaking `${(t)assoc[NAME]}` parity where C's
    // bracket-eval relies on the substituted name's value to
    // trigger "bad math expression".
    if let Some(err) = m_error_take() {
        return Err(err);
    }
    Ok(result)
}

/// Port of `lexconstant()` from `Src/math.c:462`.
///
/// Lex a numeric constant — decimal/hex/binary/octal integer or
/// floating-point literal. Sets `m_yyval()` and returns
/// `NUM`. Recognises `0x`/`0b` prefixes, base-prefix
/// (`16#FF`), trailing-dot float, scientific notation, and zsh's
/// !!! WARNING: RUST-ONLY HELPER — NO DIRECT C COUNTERPART !!!
/// C math.c:856 calls `getkeystring(ptr, NULL, GETKEYS_MATH, &v)` — the
/// shared 200-line key-string decoder run in GETKEY_SINGLE_CHAR mode
/// (decode exactly ONE char, report bytes consumed). zshrs's
/// `getkeystring_with` loops the whole string and has no single-char
/// mode, and the math lexer advances a char cursor (not a byte ptr), so
/// this small adapter decodes just the one escaped char at the cursor
/// and returns (code, chars-consumed). It mirrors the GETKEYS_MATH flag
/// set (OCTAL_ESC | EMACS | CTRL). Allowlisted in fake_fn_allowlist.txt.
fn decode_math_keychar(s: &str) -> Option<(i64, usize)> {
    let cs: Vec<char> = s.chars().collect();
    if cs.is_empty() {
        return None;
    }
    if cs[0] != '\\' {
        // c:Src/utils.c:7198-7207 — the wide-character arm is reached ONLY
        // when `isset(MULTIBYTE)`; a byte above 127 otherwise falls past it.
        // c:7209-7210 — `else if (*s == Meta) *t++ = *++s ^ 32;` decodes the
        // escaped byte, and c:7211 takes any remaining byte as itself. So
        // `##` answers a CHARACTER code in multibyte mode and a BYTE value in
        // byte mode: `unsetopt multibyte; $(( ##${gr[1]} ))` is 206 (0xce),
        // not 945 (`α`).
        if cs[0] == char::from(crate::ported::zsh_h::Meta) {
            if let Some(&n) = cs.get(1) {
                return Some((((n as u32) ^ 32) as i64, 2));
            }
        }
        let mb = crate::ported::options::opt_state_get("multibyte").unwrap_or(true);
        if !mb && (cs[0] as u32) > 127 {
            // c:7211 — one raw byte, which for text held as UTF-8 is the
            // lead byte of the character at the cursor.
            let mut buf = [0u8; 4];
            return Some((cs[0].encode_utf8(&mut buf).as_bytes()[0] as i64, 1));
        }
        return Some((cs[0] as i64, 1));
    }
    // `\X` escape — `\` plus at least one more char.
    let e = match cs.get(1) {
        Some(c) => *c,
        None => return Some(('\\' as i64, 1)),
    };
    let simple = |code: i64| Some((code, 2));
    match e {
        'n' => simple(10),
        't' => simple(9),
        'r' => simple(13),
        'e' | 'E' => simple(27),
        'a' => simple(7),
        'b' => simple(8),
        'f' => simple(12),
        'v' => simple(11),
        '\\' => simple(92),
        '0'..='7' => {
            // \NNN octal (GETKEY_OCTAL_ESC), up to 3 digits.
            let mut val: i64 = 0;
            let mut n = 0;
            while n < 3 {
                match cs.get(1 + n) {
                    Some(c @ '0'..='7') => {
                        val = val * 8 + (*c as i64 - '0' as i64);
                        n += 1;
                    }
                    _ => break,
                }
            }
            Some((val, 1 + n))
        }
        'x' => {
            // \xNN hex, up to 2 digits.
            let mut val: i64 = 0;
            let mut n = 0;
            while n < 2 {
                match cs.get(2 + n).and_then(|c| c.to_digit(16)) {
                    Some(d) => {
                        val = val * 16 + d as i64;
                        n += 1;
                    }
                    None => break,
                }
            }
            if n == 0 {
                Some(('x' as i64, 2))
            } else {
                Some((val, 2 + n))
            }
        }
        'c' => {
            // \cX control char (GETKEY_CTRL): code = X & 0x1f.
            match cs.get(2) {
                Some(c) => Some(((*c as i64) & 0x1f, 3)),
                None => Some(('c' as i64, 2)),
            }
        }
        'C' => {
            // c:Src/utils.c:7041-7046 — `case 'C': if (how & GETKEY_EMACS) {
            // if (s[1]=='-') s++; control=1; }`. `\C-X` / `\CX` → control
            // char `X & 0x1f` (e.g. `##\C-a` → 1). GETKEYS_MATH sets
            // GETKEY_EMACS, so the dash is optional and consumed when present.
            let dash = cs.get(2) == Some(&'-');
            let tidx = if dash { 3 } else { 2 };
            match cs.get(tidx) {
                Some(c) => Some(((*c as i64) & 0x1f, tidx + 1)),
                None => Some(('C' as i64, 2)),
            }
        }
        'M' => {
            // c:Src/utils.c — GETKEY_EMACS meta: `\M-X` / `\MX` → `X | 0x80`.
            let dash = cs.get(2) == Some(&'-');
            let tidx = if dash { 3 } else { 2 };
            match cs.get(tidx) {
                Some(c) => Some(((*c as i64) | 0x80, tidx + 1)),
                None => Some(('M' as i64, 2)),
            }
        }
        other => simple(other as i64),
    }
}

/// underscore digit-grouping. Mirrors C's `zstrtol_underscore()`
/// for greedy base parsing (consume valid digits only, leave the
/// rest as the next token).
pub(crate) fn lexconstant() -> i32 {
    let _start = m_pos();
    let mut is_neg = false;

    // Handle leading minus for unary context
    if peek() == Some('-') {
        is_neg = true;
        advance();
    }

    // Check for hex/binary/octal
    if peek() == Some('0') {
        advance();
        match peek().map(|c| c.to_ascii_lowercase()) {
            Some('x') => {
                // Hex: 0xFF
                advance();
                let hex_start = m_pos();
                while let Some(c) = peek() {
                    if c.is_ascii_hexdigit() || c == '_' {
                        advance();
                    } else {
                        break;
                    }
                }
                let hex_str: String = m_input_clone()[hex_start..m_pos()]
                    .chars()
                    .filter(|&c| c != '_')
                    .collect();
                // c:Src/math.c lexconstant — zsh parses every integer base via
                // zstrtol, which truncates on overflow with a
                // "number truncated after N digits" warning (utils.c:2511).
                // i64::from_str_radix just errored to 0 on a >63-bit hex
                // literal (0xFFFFFFFFFFFFFFFF). Route through the port.
                let val = crate::ported::utils::zstrtol(&hex_str, 16).0;
                m_lastbase_set(16);
                m_yyval_set(if m_force_float() {
                    mnumber {
                        l: 0,
                        d: if is_neg { -(val as f64) } else { val as f64 },
                        type_: MN_FLOAT,
                    }
                } else {
                    mnumber {
                        l: if is_neg { -val } else { val },
                        d: 0.0,
                        type_: MN_INTEGER,
                    }
                });
                return NUM;
            }
            // !!! DASH-STRICT GATE !!! `0b` binary literals are a zsh/bash(4+)
            // extension; real dash/ash reject them (POSIX has only decimal /
            // `0` octal / `0x` hex). Under --dash/--ash the `b` is left
            // unconsumed so `0b101` errors on the stray `b`, matching dash.
            Some('b') if !crate::dash_mode::dash_strict() => {
                // Binary: 0b1010
                advance();
                let bin_start = m_pos();
                while let Some(c) = peek() {
                    if c == '0' || c == '1' || c == '_' {
                        advance();
                    } else {
                        break;
                    }
                }
                let bin_str: String = m_input_clone()[bin_start..m_pos()]
                    .chars()
                    .filter(|&c| c != '_')
                    .collect();
                let val = crate::ported::utils::zstrtol(&bin_str, 2).0; // c:zstrtol base 2
                m_lastbase_set(2);
                m_yyval_set(if m_force_float() {
                    mnumber {
                        l: 0,
                        d: if is_neg { -(val as f64) } else { val as f64 },
                        type_: MN_FLOAT,
                    }
                } else {
                    mnumber {
                        l: if is_neg { -val } else { val },
                        d: 0.0,
                        type_: MN_INTEGER,
                    }
                });
                return NUM;
            }
            Some('o') | Some('O') => {
                // zsh rejects `0o…` octal-prefix (Rust/Python form).
                // Only `0x` (hex), `0b` (binary), and bare-leading-0
                // (with `setopt octalzeroes`) are recognized. Emit
                // the same diagnostic zsh produces — set s.error
                // and return a stub Num so the caller's
                // error-propagation path picks up the failure.
                m_error_set(format!(
                    "bad math expression: operator expected at `{}'",
                    &m_input_clone()[m_pos()..]
                ));
                m_yyval_set(mnumber {
                    l: 0,
                    d: 0.0,
                    type_: MN_INTEGER,
                });
                return NUM;
            }
            _ => {
                // Could be octal or just 0
                if m_octal_zeroes() {
                    // c:Src/math.c:489-512 — OCTALZEROES enabled.
                    // C scans all digits then calls
                    // `zstrtol_underscore(ptr, &ptr, 0, 1)` with base 0;
                    // strtol's base-0 octal mode stops at the first
                    // invalid octal digit (8 or 9), so the leftover
                    // digit is seen by the outer parser and produces
                    // "operator expected at `N'".
                    //
                    // To match: scan VALID octal digits (0-7) +
                    // underscores, STOP at 8/9, then emit NUM. Do NOT
                    // roll back over the 8/9 — it stays in the input
                    // for the outer parser.
                    //
                    // `.`/`e`/`E`/`#` (c:501) disqualify the whole
                    // number — fall through to decimal/float by
                    // rewinding to before the leading 0.
                    let oct_start = m_pos();
                    let mut is_float_or_base = false;
                    let mut hit_invalid_octal = false;
                    // First peek-ahead: scan all digits to detect the
                    // terminator type. This matches C's `for (ptr2 = nptr;
                    // idigit(*ptr2) || *ptr2 == '_'; ptr2++)` peek.
                    let mut probe = oct_start;
                    let input = m_input_clone();
                    while let Some(&b) = input.as_bytes().get(probe) {
                        if (b as char).is_ascii_digit() || b == b'_' {
                            if b == b'8' || b == b'9' {
                                hit_invalid_octal = true;
                            }
                            probe += 1;
                        } else {
                            if b == b'.' || b == b'e' || b == b'E' || b == b'#' {
                                is_float_or_base = true;
                            }
                            break;
                        }
                    }
                    if is_float_or_base || probe == oct_start {
                        // c:Src/math.c:489 — the octal branch is gated on
                        // `ptr2 > nptr && *ptr2 != '.'/'e'/'E'/'#'`. `nptr`
                        // is the char AFTER the leading `0`, so `ptr2 > nptr`
                        // requires at least one further digit: a bare single
                        // `0` (probe == oct_start) is NOT octal notation and
                        // must NOT set lastbase=8. Without this, `integer c=0`
                        // under `setopt octalzeroes` (emulate sh) displayed
                        // `8#0` instead of `0`. `.`/`e`/`E`/`#` after digits
                        // (is_float_or_base) likewise falls through to
                        // decimal/float.
                        m_pos_sub(1); // rewind over leading 0
                    } else {
                        // Octal path. Advance over valid octal digits
                        // only (0-7) + underscores; stop at 8/9.
                        while let Some(c) = peek() {
                            if ('0'..='7').contains(&c) || c == '_' {
                                advance();
                            } else {
                                break;
                            }
                        }
                        let oct_str: String = m_input_clone()[oct_start..m_pos()]
                            .chars()
                            .filter(|&c| c != '_')
                            .collect();
                        let val = if oct_str.is_empty() {
                            0 // c:zstrtol leading-0-only → value 0
                        } else {
                            crate::ported::utils::zstrtol(&oct_str, 8).0 // c:zstrtol base 8
                        };
                        let _ = hit_invalid_octal; // implicit via leftover digit
                        m_lastbase_set(8);
                        m_yyval_set(if m_force_float() {
                            mnumber {
                                l: 0,
                                d: if is_neg { -(val as f64) } else { val as f64 },
                                type_: MN_FLOAT,
                            }
                        } else {
                            mnumber {
                                l: if is_neg { -val } else { val },
                                d: 0.0,
                                type_: MN_INTEGER,
                            }
                        });
                        return NUM;
                    }
                } else {
                    // Put back the 0 — fall through to decimal parser.
                    m_pos_sub(1);
                }
            }
        }
    }

    // Parse decimal integer or float
    let num_start = m_pos();
    while let Some(c) = peek() {
        if is_digit(c) || c == '_' {
            advance();
        } else {
            break;
        }
    }

    // Check for float
    if peek() == Some('.') || peek() == Some('e') || peek() == Some('E') {
        // Float
        if peek() == Some('.') {
            advance();
            while let Some(c) = peek() {
                if is_digit(c) || c == '_' {
                    advance();
                } else {
                    break;
                }
            }
        }
        if peek() == Some('e') || peek() == Some('E') {
            advance();
            if peek() == Some('+') || peek() == Some('-') {
                advance();
            }
            while let Some(c) = peek() {
                if is_digit(c) || c == '_' {
                    advance();
                } else {
                    break;
                }
            }
        }
        let float_str: String = m_input_clone()[num_start..m_pos()]
            .chars()
            .filter(|&c| c != '_')
            .collect();
        // c:552-559 — right after strtod:
        //     yyval.u.d = strtod(ptr, &nptr);
        //     if (ptr == nptr || *nptr == '.') {
        //         zerr("bad floating point constant");
        //         return EOI;
        //     }
        // The `*nptr == '.'` half is the interesting one: a SECOND dot
        // immediately after the constant strtod just consumed is fatal at LEX
        // time. Without it `1.2.3` lexed as the float 1.2 followed by `.3` and
        // the failure surfaced from the PARSER as "bad math expression:
        // operator expected at `.3 '" — a different diagnostic for what zsh
        // calls a malformed constant. (`ptr == nptr`, strtod consuming nothing,
        // cannot happen here: this branch is only entered having already seen a
        // digit or a dot.)
        if peek() == Some('.') {
            m_error_set("bad floating point constant".to_string()); // c:557
            return EOI; // c:558
        }
        let val: f64 = float_str.parse().unwrap_or(0.0);
        m_yyval_set(mnumber {
            l: 0,
            d: if is_neg { -val } else { val },
            type_: MN_FLOAT,
        });
        return NUM;
    }

    // Check for base#value syntax (e.g., 16#FF)
    // !!! DASH-STRICT GATE (no C counterpart) !!! real dash/ash have POSIX
    // arithmetic only (decimal / `0` octal / `0x` hex); `base#num` is a
    // zsh/bash/ksh extension they reject. Under `zshrs --dash`/`--ash` do NOT
    // consume `#` as a base separator — the parser then hits the stray `#` and
    // errors like the real shell. --sh (bash-family) and --ksh keep accepting.
    if peek() == Some('#') && !crate::dash_mode::dash_strict() {
        advance();
        let base_str: String = m_input_clone()[num_start..m_pos() - 1]
            .chars()
            .filter(|&c| c != '_')
            .collect();
        let base: u32 = base_str.parse().unwrap_or(10);
        // zsh: `1#X` errors with "invalid base (must be 2 to 36 inclusive)".
        // i64::from_str_radix panics on out-of-range base; reject early.
        if !(2..=36).contains(&base) {
            m_error_set(format!(
                "invalid base (must be 2 to 36 inclusive): {}",
                base
            ));
            m_yyval_set(mnumber {
                l: 0,
                d: 0.0,
                type_: MN_INTEGER,
            });
            return NUM;
        }
        m_lastbase_set(base as i32);

        // Mirror zsh's `zstrtol_underscore(ptr, &ptr, base, 1)`
        // semantics: consume ONLY chars valid for the base
        // (greedy), stopping at the first invalid digit.
        // Underscore-as-thousands-separator is allowed
        // mid-number. The remaining input becomes the next
        // token, which the parser will then trip on as
        // "operator expected at `<rest>'" via the regular
        // checkunary/parser path.
        //
        // Earlier version used Rust's `from_str_radix` which
        // is all-or-nothing — a single bad digit nuked the
        // entire literal. For `2#1011x` zsh consumes the
        // valid `1011` (= 11) and errors on the trailing `x`;
        // ours errored on the whole `1011x` as one chunk.
        // Same for `2#10112` (zsh: at `2`, ours: at `10112`).
        //
        // Empty-digit-sequence case (`10#`, `2#`) silently
        // yields 0, matching zsh's `zstrtol` returning 0 when
        // no valid digits follow.
        let mut val: i64 = 0;
        let base_i64 = base as i64;
        while let Some(c) = peek() {
            if c == '_' {
                advance();
                continue;
            }
            let digit_val: Option<u32> = if c.is_ascii_digit() {
                Some(c as u32 - '0' as u32)
            } else if c.is_ascii_alphabetic() {
                Some(c.to_ascii_lowercase() as u32 - 'a' as u32 + 10)
            } else {
                None
            };
            let Some(d) = digit_val else {
                break;
            };
            if d >= base {
                break;
            }
            val = val.saturating_mul(base_i64).saturating_add(d as i64);
            advance();
        }
        m_yyval_set(if m_force_float() {
            mnumber {
                l: 0,
                d: if is_neg { -(val as f64) } else { val as f64 },
                type_: MN_FLOAT,
            }
        } else {
            mnumber {
                l: if is_neg { -val } else { val },
                d: 0.0,
                type_: MN_INTEGER,
            }
        });
        return NUM;
    }

    // Plain integer
    let int_str: String = m_input_clone()[num_start..m_pos()]
        .chars()
        .filter(|&c| c != '_')
        .collect();
    // c:Src/utils.c:2466-2515 zstrtol — accept overflow with truncation and a
    // `"number truncated after N digits"` warning rather than silently
    // producing 0. The fast i64 path covers everything up to i64::MAX; past
    // it, DELEGATE to the faithful zstrtol port (same as the hex branch above
    // at c:785) instead of a hardcoded 18-digit cut. zstrtol accumulates the
    // magnitude in a u64 and truncates ONLY when the unsigned multiply
    // overflows (19 digits for a 20+-digit run), then reinterprets the retained
    // u64 as signed — so `99999999999999999999` wraps to `-8446744073709551617`
    // (19 digits), while the fit-in-u64-but-not-i64 band (`9999999999999999999`)
    // hits the signed-overflow special case at 18 digits. The old `[..18]` slice
    // was one digit short on both counts. Bug #350; mirrors builtin.rs
    // parse_int_arg for #258.
    let val: i64 = match int_str.parse::<i64>() {
        Ok(n) => n,
        Err(_) if !int_str.is_empty() && int_str.chars().all(|c| c.is_ascii_digit()) => {
            // zstrtol emits the "number truncated after N digits" warning itself.
            crate::ported::utils::zstrtol_underscore(&int_str, 10, false).0
        }
        Err(_) => 0,
    };
    m_yyval_set(if m_force_float() {
        mnumber {
            l: 0,
            d: if is_neg { -(val as f64) } else { val as f64 },
            type_: MN_FLOAT,
        }
    } else {
        mnumber {
            l: if is_neg { -val } else { val },
            d: 0.0,
            type_: MN_INTEGER,
        }
    });
    NUM
}

// ===========================================================
// Remaining stubs from Src/math.c that don't yet have a faithful
// implementation in the migrated free-fn evaluator. The
// in-place implementations (mathevall, getmathparam, lexconstant,
// setmathvar, callmathfunc, checkunary) replaced their stubs;
// the names below correspond to C helpers the evaluator uses
// internally below — bodies wire to existing Rust idioms while
// preserving the C name + citation.
// ===========================================================

/// Port of `isinf(double x)` from Src/math.c:588 — IEEE +/-Infinity test.
/// Wraps Rust's `f64::is_infinite`.
/// WARNING: param names don't match C — Rust=() vs C=(x)
pub(crate) fn isinf(x: f64) -> bool {
    x.is_infinite()
}

/// Port of `isnan(double x)` from Src/math.c:608 — IEEE NaN test. C
/// implements it as `store(&x) != store(&x)` to defeat compiler
/// folding of the canonical `x != x` NaN test; we route through
/// `store` for parity, but Rust's `f64::is_nan` is the
/// correctness path.
/// WARNING: param names don't match C — Rust=() vs C=(x)
pub(crate) fn isnan(x: f64) -> bool {
    store(x) != store(x) || x.is_nan()
}

/// Port of `notzero(mnumber a)` from Src/math.c:1142 — error-on-zero check
/// used by `/` and `%` operators. Returns true when `a` is non-
/// zero (caller continues), false when zero (caller raises
/// "division by zero"). Float zero is treated as non-zero per
/// IEEE 754 (1/0.0 → Inf, not an error) — only integer zero
/// trips the check, matching math.c's `if (!a.u.l) zerr(…)`.
/// WARNING: param names don't match C — Rust=() vs C=(a)
pub(crate) fn notzero(a: mnumber) -> bool {
    if (a.type_ == MN_UNSET) {
        return false;
    }
    if (a.type_ == MN_INTEGER) {
        return a.l != 0;
    }
    true
}

// ============================================================
// Module-level math statics — direct port of Src/math.c globals.
//
// math.c declares each of these at file scope:
//   int noeval;                         // line 40
//   mnumber zero_mnumber;               // line 45
//   mnumber lastmathval;                // line 53
//   int lastbase;                       // line 58
//   static char *ptr;                   // line 60
//   static mnumber yyval;               // line 62
//   static char *yylval;                // line 63
//   static int mlevel = 0;              // line 67
//   static int unary = 1;               // line 71
//   static struct mathvalue *stack;     // (math.c body)
//   ... and a few derived from option flags (force_float, etc.).
//
// Rust port: thread_local!<Cell|RefCell<T>> per global. `mathevall`
// (math.c:367) saves these to its own locals (`xyyval`, `xyylval`,
// `xunary`, etc.) on entry and restores on exit so recursive math
// calls (function-arg eval, indirect string eval) don't clobber
// the outer evaluator's state.
//
// Cell for Copy types (i64/i32/usize/bool/mnumber/i32/&'static
// slice). RefCell for owned/non-Copy (String, Vec, HashMap, Option).
// ============================================================

thread_local! {
    /// `mnumber lastmathval` (math.c:53) — result of the most recent
    /// top-level `matheval`. Read by callmathfunc's MFF_USERFUNC branch
    /// (math.c:1115 `return lastmathval`): a `functions -M` math function
    /// communicates its result via the last `(( ))` in its body.
    static M_LASTMATHVAL: Cell<mnumber> = const { Cell::new(mnumber { l: 0, d: 0.0, type_: MN_INTEGER }) };
    /// `static char *ptr` — current input cursor. Owned String in Rust
    /// (vs C's caller-owned char*) so the thread_local isn't a borrow.
    static M_INPUT: RefCell<String> = const { RefCell::new(String::new()) };
    /// Byte offset into `M_INPUT` of the next char to lex.
    static M_POS: Cell<usize> = const { Cell::new(0) };
    /// Byte offset where the current token began (post-whitespace).
    /// Used to format zsh-style "at `<remaining>'" error pointers.
    static M_TOK_START: Cell<usize> = const { Cell::new(0) };
    /// `static mnumber yyval` (math.c:62) — value lexed by zzlex.
    static M_YYVAL: Cell<mnumber> = const { Cell::new(mnumber { l: 0, d: 0.0, type_: MN_INTEGER }) };
    /// `static char *yylval` (math.c:63) — identifier or function-call
    /// text lexed by zzlex (caller side reads via `M_YYLVAL.with(...)`).
    static M_YYLVAL: RefCell<String> = const { RefCell::new(String::new()) };
    /// `static struct mathvalue *stack` — operand stack for the
    /// shunting-yard evaluator. Mirrors C's heap-grown array.
    static M_STACK: RefCell<Vec<mathvalue >> = const { RefCell::new(Vec::new()) };
    /// `int mtok` — current token tag set by zzlex.
    static M_MTOK: Cell<i32> = const { Cell::new(EOI) };
    /// `static int unary` (math.c:71) — 1 when the parser is expecting
    /// an operand (so `+`/`-` mean unary plus/minus).
    static M_UNARY: Cell<bool> = const { Cell::new(true) };
    // nonzero means we are not evaluating, just parsing                    // c:37
    /// `int noeval` (math.c:40) — non-zero when in the parse-only side
    /// of `&&`/`||`/ternary; suppresses side-effects.
    static M_NOEVAL: Cell<i32> = const { Cell::new(0) };                    // c:40
    // last input base we used                                              // c:55
    /// `int lastbase` (math.c:58) — base of the last numeric literal
    /// (set by lexconstant, used by `$((…))` formatting).
    static M_LASTBASE: Cell<i32> = const { Cell::new(-1) };                 // c:58
    /// `int *prec` — active precedence table (Z_PREC or C_PREC).
    static M_PREC: Cell<&'static [u8; TOKCOUNT]> = const { Cell::new(&Z_PREC) };
    /// `setopt CPRECEDENCES` mirror.
    static M_C_PRECEDENCES: Cell<bool> = const { Cell::new(false) };
    /// `setopt FORCEFLOAT` mirror.
    static M_FORCE_FLOAT: Cell<bool> = const { Cell::new(false) };
    /// `setopt OCTALZEROES` mirror.
    static M_OCTAL_ZEROES: Cell<bool> = const { Cell::new(false) };
    /// In-memory params table (zshrs uses this instead of the C param
    /// table). Carries float/integer mnumber results.
    static M_VARIABLES: RefCell<HashMap<String, mnumber>> = RefCell::new(HashMap::new());
    /// Raw string values for variables whose contents aren't a plain
    /// number — recursively re-eval'd by `getmathparam` for
    /// `a="3+2"; $((a))` semantics.
    static M_STRING_VARIABLES: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    /// `$?` — last command exit status, used by the `?` token in
    /// unary position.
    static M_LASTVAL: Cell<i32> = const { Cell::new(0) };
    /// `$$` — current process ID, lexed for the `$` token.
    static M_PID: Cell<i64> = const { Cell::new(0) };
    /// Error message accumulator. zsh C uses `setjmp`/`longjmp`; the
    /// Rust port returns errors via this Option then `mathevall`
    /// surfaces it as `Result::Err`.
    static M_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
    /// `int outputradix` (math.c:580) — output base for the result.
    /// Set by `[#N]` (positive N, with `N#` prefix) or `[##N]`
    /// (negative N, bare digits). Read by subst.rs's `$((…))`
    /// formatter at math.c:4493-4498.
    static M_OUTPUTRADIX: Cell<i32> = const { Cell::new(0) };               // c:580
    /// `int outputunderscore` (math.c:583) — group every N digits with
    /// `_` for readable hex/decimal output. Set by `[#N_M]` /
    /// `[##N_M]` / `[#_M]`. Read alongside outputradix.
    static M_OUTPUTUNDERSCORE: Cell<i32> = const { Cell::new(0) };          // c:583
    /// `static int mlevel` (math.c:67) — count of `mathevall` frames
    /// currently on the stack. C brackets the evaluator body with
    /// `if (mlevel++)` (c:386) / `if (--mlevel)` (c:446), so mlevel is 0
    /// exactly when no evaluation is in progress. `matheval` reads it to
    /// decide whether the output radix is a fresh one (c:1486).
    static M_LEVEL: Cell<i32> = const { Cell::new(0) };                     // c:67
}

/// RAII bracket for C's `mlevel++` (math.c:386) / `--mlevel` (math.c:446).
///
/// C can pair the increment and decrement by hand because `mathevall` has a
/// single exit. The Rust evaluator returns `Result` from many points inside the
/// parse loop, so the decrement rides on `Drop` — otherwise an `Err` path would
/// leave `mlevel` permanently raised and every later `matheval` would skip its
/// reset, treating an unrelated expression as a nested one.
struct MathLevel;

impl MathLevel {
    /// c:386 — `if (mlevel++)`.
    fn enter() -> Self {
        M_LEVEL.with(|c| c.set(c.get() + 1));
        MathLevel
    }
}

impl Drop for MathLevel {
    /// c:446 — `if (--mlevel)`.
    fn drop(&mut self) {
        M_LEVEL.with(|c| c.set(c.get() - 1));
    }
}

/// `outputradix` accessor for subst.rs's `$((…))` formatter.
/// Returns 0 if no `[#…]` directive was seen during the most
/// recent matheval. Caller is responsible for clearing via
/// `set_output_format(0, 0)` if it wants per-call state.
pub fn outputradix() -> i32 {
    M_OUTPUTRADIX.with(|c| c.get())
}

/// `outputunderscore` accessor — see [`outputradix`].
pub fn outputunderscore() -> i32 {
    M_OUTPUTUNDERSCORE.with(|c| c.get())
}

/// Reset the output-format state. Called by `mathevall` before
/// each evaluation so `[#16]` from a prior `$((…))` doesn't leak
/// into the next call.
pub fn reset_output_format() {
    M_OUTPUTRADIX.with(|c| c.set(0));
    M_OUTPUTUNDERSCORE.with(|c| c.set(0));
}

fn m_outputradix_set(v: i32) {
    M_OUTPUTRADIX.with(|c| c.set(v));
}

fn m_outputunderscore_set(v: i32) {
    M_OUTPUTUNDERSCORE.with(|c| c.set(v));
}

// ============================================================
// WARNING: NOT IN MATH.C — every `m_*` fn below is a Rust-only
// thread_local accessor. C dereferences the corresponding module
// global directly (`yyval.u.l`, `*ptr++`, etc.) without an
// fn-shaped wrapper. The wrappers exist solely because Rust's
// `thread_local!` requires `.with(|c| ...)` for any access, and
// scattering 600 such closures throughout the evaluator would be
// unreadable. Allowlisted in tests/data/fake_fn_allowlist.txt.
// ============================================================
// Accessor helpers — each thread_local reads/writes via these so the
// migration from `s.X` → free-fn-only access is mechanical.

#[inline]
fn m_input_clone() -> String {
    M_INPUT.with(|c| c.borrow().clone())
}
#[inline]
fn m_input_set(v: String) {
    M_INPUT.with(|c| *c.borrow_mut() = v)
}
#[inline]
fn m_input_len() -> usize {
    M_INPUT.with(|c| c.borrow().len())
}
#[inline]
fn m_input_byte(i: usize) -> u8 {
    M_INPUT.with(|c| c.borrow().as_bytes().get(i).copied().unwrap_or(0))
}
#[inline]
fn m_input_slice_from(start: usize) -> String {
    M_INPUT.with(|c| c.borrow()[start..].to_string())
}
#[inline]
fn m_input_slice(start: usize, end: usize) -> String {
    M_INPUT.with(|c| c.borrow()[start..end].to_string())
}

#[inline]
fn m_pos() -> usize {
    M_POS.with(|c| c.get())
}
#[inline]
fn m_pos_set(v: usize) {
    M_POS.with(|c| c.set(v))
}
#[inline]
fn m_pos_sub(n: usize) {
    M_POS.with(|c| c.set(c.get() - n))
}
#[inline]
fn m_pos_add(n: usize) {
    M_POS.with(|c| c.set(c.get() + n))
}

#[inline]
fn m_tok_start() -> usize {
    M_TOK_START.with(|c| c.get())
}
#[inline]
fn m_tok_start_set(v: usize) {
    M_TOK_START.with(|c| c.set(v))
}

#[inline]
fn m_yyval() -> mnumber {
    M_YYVAL.with(|c| c.get())
}
#[inline]
fn m_yyval_set(v: mnumber) {
    M_YYVAL.with(|c| c.set(v))
}

#[inline]
fn m_yylval_clone() -> String {
    M_YYLVAL.with(|c| c.borrow().clone())
}
#[inline]
fn m_yylval_set(v: String) {
    M_YYLVAL.with(|c| *c.borrow_mut() = v)
}

#[inline]
fn m_mtok() -> i32 {
    M_MTOK.with(|c| c.get())
}
#[inline]
fn m_mtok_set(t: i32) {
    M_MTOK.with(|c| c.set(t))
}

#[inline]
fn m_unary() -> bool {
    M_UNARY.with(|c| c.get())
}
#[inline]
fn m_unary_set(v: bool) {
    M_UNARY.with(|c| c.set(v))
}

/// Accessor for the math-evaluator `noeval` counter (`Src/math.c:40`
/// `int noeval`). C reads/writes the global directly — Rust ports
/// the global as a thread-local `M_NOEVAL` (so nested evaluators
/// stay isolated) and exposes the read via this `pub` accessor.
/// Used by exec.c's `execsave` / `execrestore` save/restore frame
/// (`Src/exec.c:6450,6486`) — the math state must round-trip across
/// nested sublist evaluation so a ternary-arm `noeval++/--` inside
/// one expression doesn't leak into outer evaluations.
#[inline]
pub fn m_noeval() -> i32 {
    M_NOEVAL.with(|c| c.get())
}
/// Setter paired with `m_noeval` — assigns the math-evaluator
/// `noeval` counter. C does plain `noeval = en->noeval;`; this is
/// the Rust thread-local equivalent.
#[inline]
pub fn m_noeval_set(v: i32) {
    M_NOEVAL.with(|c| c.set(v))
}
#[inline]
fn m_noeval_inc() {
    M_NOEVAL.with(|c| c.set(c.get() + 1))
}
#[inline]
fn m_noeval_dec() {
    M_NOEVAL.with(|c| c.set(c.get() - 1))
}

#[inline]
fn m_lastbase_set(v: i32) {
    M_LASTBASE.with(|c| c.set(v))
}

/// Public getter for `lastbase` — used by `assignstrvalue` in
/// params.rs to inherit the input numeric base when a freshly
/// assigned integer parameter has none of its own.
pub fn lastbase() -> i32 {
    M_LASTBASE.with(|c| c.get())
}

/// Public setter for `lastbase` — used by the bytecode arith
/// compiler (extensions/arith_compiler.rs) to communicate the
/// source numeric base when a `N#NNN` or `0x..` literal is
/// consumed inside `(( … ))`. The canonical math.c port at
/// `Src/math.c::lexconstant` sets this internally; bypassing
/// the canonical lexer (as `arith_compiler` does) requires
/// poking the TLS slot directly so assignsparam's `pm.base ==
/// 0 ? lastbase()` inheritance path fires.
pub fn set_lastbase(base: i32) {
    m_lastbase_set(base)
}

#[inline]
fn m_prec() -> &'static [u8; TOKCOUNT] {
    M_PREC.with(|c| c.get())
}
#[inline]
fn m_prec_set(p: &'static [u8; TOKCOUNT]) {
    M_PREC.with(|c| c.set(p))
}

#[inline]
fn m_c_precedences() -> bool {
    M_C_PRECEDENCES.with(|c| c.get())
}
#[inline]
fn m_c_precedences_set(v: bool) {
    M_C_PRECEDENCES.with(|c| c.set(v))
}
#[inline]
fn m_force_float() -> bool {
    M_FORCE_FLOAT.with(|c| c.get())
}
#[inline]
fn m_force_float_set(v: bool) {
    M_FORCE_FLOAT.with(|c| c.set(v))
}
#[inline]
fn m_octal_zeroes() -> bool {
    // c:Src/math.c:489 — `isset(OCTALZEROES)` is read directly at
    // each integer-literal parse site, not snapshotted at math-eval
    // entry. The thread-local cache here only honored a snapshot
    // pushed by arith_compile (line 1205); freshly toggled
    // `setopt octalzeroes` inside the same script never reached it.
    // Mirror C by reading the option live.
    if crate::ported::zsh_h::isset(crate::ported::zsh_h::OCTALZEROES) {
        return true;
    }
    M_OCTAL_ZEROES.with(|c| c.get())
}
#[inline]
fn m_octal_zeroes_set(v: bool) {
    M_OCTAL_ZEROES.with(|c| c.set(v))
}

#[inline]
fn m_lastval_set(v: i32) {
    M_LASTVAL.with(|c| c.set(v))
}
#[inline]
fn m_lastval() -> i32 {
    M_LASTVAL.with(|c| c.get())
}
#[inline]
fn m_pid() -> i64 {
    M_PID.with(|c| c.get())
}
#[inline]
fn m_pid_set(v: i64) {
    M_PID.with(|c| c.set(v))
}

#[inline]
fn m_error_take() -> Option<String> {
    M_ERROR.with(|c| c.borrow_mut().take())
}
#[inline]
fn m_error_some() -> bool {
    M_ERROR.with(|c| c.borrow().is_some())
}
#[inline]
fn m_error_set(msg: String) {
    M_ERROR.with(|c| {
        if c.borrow().is_none() {
            *c.borrow_mut() = Some(msg);
        }
    })
}
#[inline]
fn m_error_set_force(msg: String) {
    M_ERROR.with(|c| *c.borrow_mut() = Some(msg))
}
#[inline]
fn m_error_clear() {
    M_ERROR.with(|c| *c.borrow_mut() = None)
}

// Stack helpers — mathvalue stack operations.
#[inline]
fn m_stack_push(v: mathvalue) {
    M_STACK.with(|c| c.borrow_mut().push(v))
}
#[inline]
fn m_stack_pop() -> Option<mathvalue> {
    M_STACK.with(|c| c.borrow_mut().pop())
}
#[inline]
fn m_stack_len() -> usize {
    M_STACK.with(|c| c.borrow().len())
}
#[inline]
fn m_stack_is_empty() -> bool {
    M_STACK.with(|c| c.borrow().is_empty())
}
#[inline]
fn m_stack_top_clone() -> Option<mathvalue> {
    M_STACK.with(|c| c.borrow().last().cloned())
}

// Variable map helpers.
#[inline]
fn m_variables_get(name: &str) -> Option<mnumber> {
    M_VARIABLES.with(|c| c.borrow().get(name).copied())
}
#[inline]
fn m_variables_insert(k: String, v: mnumber) {
    M_VARIABLES.with(|c| {
        c.borrow_mut().insert(k, v);
    })
}
#[inline]
fn m_variables_clone() -> HashMap<String, mnumber> {
    M_VARIABLES.with(|c| c.borrow().clone())
}
#[inline]
fn m_variables_set(map: HashMap<String, mnumber>) {
    M_VARIABLES.with(|c| *c.borrow_mut() = map)
}

#[inline]
fn m_string_variables_get(name: &str) -> Option<String> {
    M_STRING_VARIABLES.with(|c| c.borrow().get(name).cloned())
}
#[inline]
fn m_string_variables_remove(name: &str) {
    M_STRING_VARIABLES.with(|c| {
        c.borrow_mut().remove(name);
    })
}
#[inline]
fn m_string_variables_clone() -> HashMap<String, String> {
    M_STRING_VARIABLES.with(|c| c.borrow().clone())
}
#[inline]
fn m_string_variables_set(map: HashMap<String, String>) {
    M_STRING_VARIABLES.with(|c| *c.borrow_mut() = map)
}
#[inline]
fn m_string_variables_insert(k: String, v: String) {
    M_STRING_VARIABLES.with(|c| {
        c.borrow_mut().insert(k, v);
    })
}

/// Save/restore container — mirrors C `mathevall()` (Src/math.c:367)'s
/// stack locals (`xyyval`, `xyylval`, `xunary`, `xnoeval`, `xptr`,
/// etc.). Wrap recursive math eval (`callmathfunc` arg parsing,
/// `getmathparam` indirect-string eval) with `save_state()` /
/// `restore_state()` so the parent's evaluator state survives the
/// inner call's thread_local mutations.
#[allow(non_camel_case_types)]
struct xyy_locals {
    input: String,
    pos: usize,
    tok_start: usize,
    yyval: mnumber,
    yylval: String,
    stack: Vec<mathvalue>,
    mtok: i32,
    unary: bool,
    noeval: i32,
    error: Option<String>,
    variables: HashMap<String, mnumber>,
    string_variables: HashMap<String, String>,
    prec: &'static [u8; TOKCOUNT],
    c_precedences: bool,
    force_float: bool,
    octal_zeroes: bool,
    lastbase: i32,
}

// WARNING: NOT IN MATH.C — Rust-only helper. C inlines the
// xyy* save/restore directly inside `mathevall()`'s body
// (math.c:367 onward); the Rust port factors it out because two
// callsites (callmathfunc arg parsing, getmathparam indirect-string
// eval) would each duplicate ~17 lines of save/restore code.
fn save_state() -> xyy_locals {
    xyy_locals {
        input: m_input_clone(),
        pos: m_pos(),
        tok_start: m_tok_start(),
        yyval: m_yyval(),
        yylval: m_yylval_clone(),
        stack: M_STACK.with(|c| c.borrow().clone()),
        mtok: m_mtok(),
        unary: m_unary(),
        noeval: m_noeval(),
        error: M_ERROR.with(|c| c.borrow().clone()),
        variables: m_variables_clone(),
        string_variables: m_string_variables_clone(),
        prec: m_prec(),
        c_precedences: m_c_precedences(),
        force_float: m_force_float(),
        octal_zeroes: m_octal_zeroes(),
        lastbase: M_LASTBASE.with(|c| c.get()),
    }
}

/// Port of `store(double *x)` from Src/math.c:601 — load/store a double
/// via a pointer to defeat compilers that mis-optimize the
/// canonical `x != x` NaN test. zsh only compiles this path when
/// `HAVE_ISNAN` is undefined; we keep it as a name-parity shim
/// so `isnan()` can route through it (matching the C source's
/// `store(&x) != store(&x)` idiom).
/// WARNING: param names don't match C — Rust=() vs C=(x)
pub(crate) fn store(x: f64) -> f64 {
    x
}

/// Port of `getcvar(char *s)` from Src/math.c:943 — character-constant
/// lookup. Reads the named shell variable and returns the
/// codepoint of its first character. Used for `#varname` token
/// (CId): `x="hello"; (( y = #x ))` puts 104 (`'h'`) into y.
/// On miss or empty value, returns 0 (matches zsh's `*s ? *s : 0`).
/// WARNING: param names don't match C — Rust=() vs C=(s)
pub(crate) fn getcvar(name: &str) -> mnumber {
    if let Some(raw) = m_string_variables_get(name) {
        return mnumber {
            l: raw.chars().next().map(|c| c as i64).unwrap_or(0),
            d: 0.0,
            type_: MN_INTEGER,
        };
    }
    // c:Src/math.c:943 — `getcvar` falls back to `getsparam` for
    // scalar params not already cached in math-local tables. Without
    // this, `a=A; (( #a ))` returned 0 instead of 65 — `m_string_
    // variables_get` only sees variables explicitly seeded into the
    // math frame.
    if let Some(raw) = getsparam(name) {
        return mnumber {
            l: raw.chars().next().map(|c| c as i64).unwrap_or(0),
            d: 0.0,
            type_: MN_INTEGER,
        };
    }
    if let Some(v) = m_variables_get(name) {
        let s = match v.type_ {
            MN_INTEGER => v.l.to_string(),
            MN_FLOAT => {
                let f = v.d;
                if isnan(f) {
                    "NaN".to_string()
                } else if isinf(f) {
                    if f > 0.0 {
                        "Inf".to_string()
                    } else {
                        "-Inf".to_string()
                    }
                } else {
                    format!("{:.10}", f)
                }
            }
            _ => "0".to_string(),
        };
        return mnumber {
            l: s.chars().next().map(|c| c as i64).unwrap_or(0),
            d: 0.0,
            type_: MN_INTEGER,
        };
    }
    mnumber {
        l: 0,
        d: 0.0,
        type_: MN_INTEGER,
    }
}

/// Port of `zzlex()` from `Src/math.c:617`.
///
/// Main math-expression lexer — returns the next token, advancing
/// `m_pos()` and updating `m_yyval()` / `m_yylval_clone()` as side-effects.
/// Handles all operators, ident lookahead for `Func` vs `Id`,
/// `[base]value` / `[#base]EXPR` output-radix prefixes, char
/// constants (`#x`, `##varname`), and dispatches numeric literals
/// to `lexconstant()`.
pub(crate) fn zzlex() -> i32 {
    m_yyval_set(mnumber {
        l: 0,
        d: 0.0,
        type_: MN_INTEGER,
    });

    loop {
        let pre_pos = m_pos();
        let c = match advance() {
            Some(c) => c,
            None => {
                m_tok_start_set(pre_pos);
                return EOI;
            }
        };

        if matches!(c, ' ' | '\t' | '\n' | '"') {
            continue;
        }
        // Record where this token began (post-whitespace) so error
        // formatters can produce zsh-style "at `<remaining>`" messages.
        m_tok_start_set(pre_pos);

        match c {
            '+' => {
                if peek() == Some('+') {
                    advance();
                    return if m_unary() { PREPLUS } else { POSTPLUS };
                }
                if peek() == Some('=') {
                    advance();
                    return PLUSEQ;
                }
                return if m_unary() { UPLUS } else { PLUS };
            }

            '-' => {
                if peek() == Some('-') {
                    advance();
                    return if m_unary() { PREMINUS } else { POSTMINUS };
                }
                if peek() == Some('=') {
                    advance();
                    return MINUSEQ;
                }
                if m_unary() {
                    // Check if followed by digit for negative number
                    if let Some(next) = peek() {
                        if is_digit(next) || next == '.' {
                            m_pos_sub(1); // Put back the -
                            return lexconstant();
                        }
                    }
                    return UMINUS;
                }
                return MINUS;
            }

            '(' => return M_INPAR,
            ')' => return M_OUTPAR,

            '!' => {
                if peek() == Some('=') {
                    advance();
                    return NEQ;
                }
                return NOT;
            }

            '~' => return COMP,

            '&' => {
                if peek() == Some('&') {
                    advance();
                    if peek() == Some('=') {
                        advance();
                        return DANDEQ;
                    }
                    return DAND;
                }
                if peek() == Some('=') {
                    advance();
                    return ANDEQ;
                }
                return AND;
            }

            '|' => {
                if peek() == Some('|') {
                    advance();
                    if peek() == Some('=') {
                        advance();
                        return DOREQ;
                    }
                    return DOR;
                }
                if peek() == Some('=') {
                    advance();
                    return OREQ;
                }
                return OR;
            }

            '^' => {
                if peek() == Some('^') {
                    advance();
                    if peek() == Some('=') {
                        advance();
                        return DXOREQ;
                    }
                    return DXOR;
                }
                if peek() == Some('=') {
                    advance();
                    return XOREQ;
                }
                return XOR;
            }

            '*' => {
                // !!! DASH-STRICT GATE (no C counterpart) !!!
                // dash arithmetic has no `**` exponentiation operator.
                // Skip the POWER branch under dash_strict so `2**10` lexes
                // as `2 * * 10` (MUL MUL) and the parser errors with
                // "expecting primary", matching /bin/dash.
                if peek() == Some('*') && !crate::dash_mode::dash_strict() {
                    advance();
                    if peek() == Some('=') {
                        advance();
                        return POWEREQ;
                    }
                    return POWER;
                }
                if peek() == Some('=') {
                    advance();
                    return MULEQ;
                }
                return MUL;
            }

            '/' => {
                if peek() == Some('=') {
                    advance();
                    return DIVEQ;
                }
                return DIV;
            }

            '%' => {
                if peek() == Some('=') {
                    advance();
                    return MODEQ;
                }
                return MOD;
            }

            '<' => {
                if peek() == Some('<') {
                    advance();
                    if peek() == Some('=') {
                        advance();
                        return SHLEFTEQ;
                    }
                    return SHLEFT;
                }
                if peek() == Some('=') {
                    advance();
                    return LEQ;
                }
                return LES;
            }

            '>' => {
                if peek() == Some('>') {
                    advance();
                    if peek() == Some('=') {
                        advance();
                        return SHRIGHTEQ;
                    }
                    return SHRIGHT;
                }
                if peek() == Some('=') {
                    advance();
                    return GEQ;
                }
                return GRE;
            }

            '=' => {
                if peek() == Some('=') {
                    advance();
                    return DEQ;
                }
                return EQ;
            }

            '$' => {
                // $$ = pid
                m_yyval_set(mnumber {
                    l: m_pid(),
                    d: 0.0,
                    type_: MN_INTEGER,
                });
                return NUM;
            }

            '?' => {
                if m_unary() {
                    // c:Src/math.c:772-776 — `case '?': if (unary) { yyval.u.l
                    // = lastval; return NUM; } return QUEST;`. Read the live
                    // `LASTVAL` atomic (zsh C's `lastval` global). The local
                    // `m_lastval()` cache is unset by any matheval caller —
                    // it would always be 0. Bug #367.
                    let lv =
                        crate::ported::builtin::LASTVAL.load(std::sync::atomic::Ordering::Relaxed);
                    m_yyval_set(mnumber {
                        l: lv as i64,
                        d: 0.0,
                        type_: MN_INTEGER,
                    });
                    return NUM;
                }
                return QUEST;
            }

            ':' => return COLON,
            ',' => {
                // !!! DASH-STRICT GATE (no C counterpart) !!!
                // dash arithmetic has no comma operator; `$((1,2))` errors.
                // Flag an error and end input so the whole `$((...))` fails
                // with a non-zero status like /bin/dash (which reports
                // "expecting EOF").
                if crate::dash_mode::dash_strict() {
                    m_error_set("bad math expression: ',' operator not supported".to_string());
                    return EOI;
                }
                return COMMA;
            }

            '[' => {
                // [base]value or output format [#base]
                if is_digit(peek().unwrap_or('\0')) {
                    // [base]value
                    let base_start = m_pos();
                    while let Some(c) = peek() {
                        if is_digit(c) {
                            advance();
                        } else {
                            break;
                        }
                    }
                    if peek() != Some(']') {
                        m_error_set("bad base syntax".to_string());
                        return EOI;
                    }
                    let base_str: String = m_input_clone()[base_start..m_pos()].to_string();
                    let base: u32 = base_str.parse().unwrap_or(10);
                    advance(); // skip ]

                    if !is_digit(peek().unwrap_or('\0')) && !is_ident_start(peek().unwrap_or('\0'))
                    {
                        m_error_set("bad base syntax".to_string());
                        return EOI;
                    }
                    // Reject out-of-range bases; from_str_radix panics
                    // on bases outside [2, 36].
                    if !(2..=36).contains(&base) {
                        m_error_set(format!(
                            "invalid base (must be 2 to 36 inclusive): {}",
                            base
                        ));
                        m_yyval_set(mnumber {
                            l: 0,
                            d: 0.0,
                            type_: MN_INTEGER,
                        });
                        return NUM;
                    }

                    let val_start = m_pos();
                    while let Some(c) = peek() {
                        if c.is_ascii_alphanumeric() {
                            advance();
                        } else {
                            break;
                        }
                    }
                    let val_str = &m_input_clone()[val_start..m_pos()];
                    let val = crate::ported::utils::zstrtol(val_str, base as i32).0; // c:zstrtol base#N
                    m_lastbase_set(base as i32);
                    m_yyval_set(mnumber {
                        l: val,
                        d: 0.0,
                        type_: MN_INTEGER,
                    });
                    return NUM;
                }
                // c:Src/math.c:798-832 — `[#N]` / `[##N]` / `[#_M]`
                // output format specifier. Set outputradix to ±N and
                // outputunderscore to M (digit-grouping width).
                //
                //   `[#N]`        outputradix = +N    (emit `N#` prefix)
                //   `[##N]`       outputradix = -N    (bare digits, no prefix)
                //   `[#N_M]`      ... plus underscore every M digits
                //   `[#_M]`       outputradix unchanged, underscore = M
                //   `[#_]`        outputradix unchanged, underscore = 3 (default)
                //
                // Previous Rust port matched `[#…]` and SILENTLY DROPPED
                // the directive, so `$(([##16] 255))` returned `255` (decimal)
                // instead of `FF`. p10k uses `[##16]` in glyph-code emitters
                // and `[#16]` in icon-byte formatting; both were broken.
                if peek() == Some('#') {
                    advance(); // c:798 — skip first `#`
                    let mut n: i32 = 1; // c:799
                    if peek() == Some('#') {
                        // c:800 — second `#` flips sign for "no prefix"
                        n = -1; // c:801
                        advance(); // c:802
                    }
                    let p_now = peek().unwrap_or('\0');
                    if !is_digit(p_now) && p_now != '_' {
                        // c:804-805
                        m_error_set("bad output format specification".to_string());
                        return EOI;
                    }
                    let mut checkradix = false;
                    if is_digit(p_now) {
                        // c:806-809 — `outputradix = n * zstrtol(ptr, &ptr, 10);`
                        let rstart = m_pos();
                        while let Some(c) = peek() {
                            if is_digit(c) {
                                advance();
                            } else {
                                break;
                            }
                        }
                        let radix_str: String = m_input_clone()[rstart..m_pos()].to_string();
                        let radix: i32 = radix_str.parse().unwrap_or(10);
                        m_outputradix_set(n * radix); // c:807
                        checkradix = true; // c:808
                    }
                    if peek() == Some('_') {
                        // c:810-816 — `[…_M]` underscore digit-grouping width.
                        advance(); // c:811
                        let us_now = peek().unwrap_or('\0');
                        if is_digit(us_now) {
                            let ustart = m_pos();
                            while let Some(c) = peek() {
                                if is_digit(c) {
                                    advance();
                                } else {
                                    break;
                                }
                            }
                            let us_str: String = m_input_clone()[ustart..m_pos()].to_string();
                            m_outputunderscore_set(us_str.parse().unwrap_or(3));
                            // c:812-813
                        } else {
                            m_outputunderscore_set(3); // c:814-815 default
                        }
                    }
                    if peek() != Some(']') {
                        // c:822-823
                        m_error_set("bad output format specification".to_string());
                        return EOI;
                    }
                    advance(); // c:832 — skip `]`
                    if checkradix {
                        // c:824-831 — validate base ∈ [2, 36].
                        let abs_n = M_OUTPUTRADIX.with(|c| c.get()).abs();
                        if !(2..=36).contains(&abs_n) {
                            m_error_set(format!(
                                "invalid base (must be 2 to 36 inclusive): {}",
                                M_OUTPUTRADIX.with(|c| c.get())
                            ));
                            return EOI;
                        }
                    }
                    // c:833 — `break;` — fall through to the next token
                    // (the format directive doesn't yield a NUM itself).
                    continue;
                }
                m_error_set("bad output format specification".to_string());
                return EOI;
            }

            '#' => {
                // Character code: #\x or ##string
                if peek() == Some('\\') || peek() == Some('#') {
                    advance(); // consume the `\` / 2nd `#` marker
                               // c:852-854 — `ptr++; if (!*ptr) { zerr("bad math
                               // expression: character missing after ##"); return EOI; }`.
                               // `$((##))` with nothing after the marker is an error, not 0.
                    if peek().is_none() {
                        crate::ported::utils::zerr(
                            "bad math expression: character missing after ##",
                        );
                        return EOI;
                    }
                    // c:Src/math.c:856 — `getkeystring(ptr, NULL,
                    // GETKEYS_MATH, &v)` decodes the char AFTER the
                    // marker, honoring backslash escapes: `##\n` → 10,
                    // `##\e` → 27, `##A` → 65. The previous port read a
                    // single literal char, so `##\n` yielded 92 (`\`)
                    // and left `n` dangling ("operator expected").
                    let rest: String = m_input_clone()[m_pos()..].to_string();
                    if let Some((code, consumed)) = decode_math_keychar(&rest) {
                        for _ in 0..consumed {
                            advance();
                        }
                        m_yyval_set(mnumber {
                            l: code,
                            d: 0.0,
                            type_: MN_INTEGER,
                        });
                        return NUM;
                    }
                    if let Some(ch) = advance() {
                        m_yyval_set(mnumber {
                            l: ch as i64,
                            d: 0.0,
                            type_: MN_INTEGER,
                        });
                        return NUM;
                    }
                }
                // #varname - get first char value
                let id_start = m_pos();
                while let Some(c) = peek() {
                    if is_ident(c) {
                        advance();
                    } else {
                        break;
                    }
                }
                if m_pos() > id_start {
                    m_yylval_set(m_input_clone()[id_start..m_pos()].to_string());
                    return CID;
                }
                // c:Src/math.c:911-915 — bare `#` (followed by non-ident) is
                // `$#` (positional-parameter count): `yyval.u.l =
                // poundgetfn(NULL); return NUM;`. Bug #368.
                m_yyval_set(mnumber {
                    l: crate::ported::params::poundgetfn(),
                    d: 0.0,
                    type_: MN_INTEGER,
                });
                return NUM;
            }

            _ => {
                if is_digit(c) || (c == '.' && is_digit(peek().unwrap_or('\0'))) {
                    m_pos_sub(c.len_utf8());
                    return lexconstant();
                }

                if is_ident_start(c) {
                    let id_start = m_pos() - c.len_utf8();
                    while let Some(c) = peek() {
                        if is_ident(c) {
                            advance();
                        } else {
                            break;
                        }
                    }

                    let id = &m_input_clone()[id_start..m_pos()];

                    // Check for Inf/NaN
                    let id_lower = id.to_lowercase();
                    if id_lower == "nan" {
                        m_yyval_set(mnumber {
                            l: 0,
                            d: f64::NAN,
                            type_: MN_FLOAT,
                        });
                        return NUM;
                    }
                    if id_lower == "inf" {
                        m_yyval_set(mnumber {
                            l: 0,
                            d: f64::INFINITY,
                            type_: MN_FLOAT,
                        });
                        return NUM;
                    }

                    // Check for function call
                    if peek() == Some('(') {
                        // Skip to closing paren
                        let func_start = id_start;
                        advance(); // (
                        let mut depth = 1;
                        while let Some(c) = peek() {
                            advance();
                            if c == '(' {
                                depth += 1;
                            } else if c == ')' {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                        }
                        m_yylval_set(m_input_clone()[func_start..m_pos()].to_string());
                        return FUNC;
                    }

                    // Check for array subscript
                    if peek() == Some('[') {
                        advance(); // [
                        let mut depth = 1;
                        while let Some(c) = peek() {
                            advance();
                            if c == '[' {
                                depth += 1;
                            } else if c == ']' {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                        }
                    }

                    m_yylval_set(m_input_clone()[id_start..m_pos()].to_string());
                    return ID;
                }

                // c:842 — `default: if (idigit(*--ptr) ...` — the C
                // default case BACKS UP so an unrecognized char (e.g.
                // `'`) is left un-consumed; matheval's trailing-junk
                // check (c:1498-1499) then reports THAT char:
                // `$(( 'A' ))` → "illegal character: '" not ": A".
                m_pos_sub(c.len_utf8());
                return EOI;
            }
        }
    }
}

impl Default for mathvalue {
    fn default() -> Self {
        mathvalue {
            val: mnumber {
                l: 0,
                d: 0.0,
                type_: MN_INTEGER,
            },
            lval: None,
            pval: (),
        }
    }
}

/// Port of `push(mnumber val, char *lval, int getme)` from `Src/math.c:916`.
///
/// Push a value onto the evaluator's operand stack, with the
/// optional lvalue name (set when the value came from a variable
/// reference; needed for `++`/`--`/assignment-op write-back).
/// WARNING: param names don't match C — Rust=(lval) vs C=(val, lval, getme)
pub(crate) fn push(val: mnumber, lval: Option<String>) {
    m_stack_push(mathvalue {
        val,
        lval,
        pval: (),
    });
}

/// Port of `pop(int noget)` from `Src/math.c:931`.
///
/// Pop the top operand from the stack, resolving any deferred
/// variable read (`mnumber { l: 0, d: 0.0, type_: MN_UNSET }` + lval set). The C source
/// passes a `noget` flag to skip the resolution; the Rust port
/// always resolves since callers that want the raw lvalue use
/// `pop_with_lval` instead.
/// WARNING: param names don't match C — Rust=() vs C=(noget)
pub(crate) fn pop() -> mnumber {
    if let Some(mv) = m_stack_pop() {
        if (mv.val.type_ == MN_UNSET) {
            if let Some(ref name) = mv.lval {
                return getmathparam(name);
            }
        }
        mv.val
    } else {
        m_error_set("stack underflow".to_string());
        mnumber {
            l: 0,
            d: 0.0,
            type_: MN_INTEGER,
        }
    }
}

/// Port of `setmathvar(struct mathvalue *mvp, mnumber v)` from `Src/math.c:972`.
///
/// Write `val` to the named parameter from inside math context.
/// Port of `setmathvar(struct mathvalue *mvp, mnumber v)` from `Src/math.c:972`.
/// Calls `setnparam` (the canonical param-set) and returns the value
/// re-typed to match the parameter's type (C c:1014-1027).
pub(crate) fn setmathvar(name: &str, val: mnumber) -> mnumber {
    // c:972
    // c:996-1001 — bad-lvalue check (empty name).
    if name.is_empty() {
        zerr("bad math expression: lvalue required");
        return mnumber {
            l: 0,
            d: 0.0,
            type_: MN_INTEGER,
        };
    }
    // c:1002-1003 — `if (noeval) return v;`
    if M_NOEVAL.with(|n| n.get()) != 0 {
        return val;
    }
    // c:1004 — `setnparam(mvp->lval, v)`. C passes the FULL lval
    // (including any `[subscript]`) to setnparam → assignnparam,
    // which calls getvalue to resolve the subscript and routes the
    // write via setnumvalue on the resulting Value (whose v->pm for
    // a hash element is the hash-element scalar shim — see
    // params.c:640 `foundparam` set by scanparamvals at c:664). The
    // previous Rust port stripped the subscript here and called
    // setnparam("counts", val) for `counts[apple]++` — silently
    // wiping the assoc/array and replacing it with a scalar.
    //
    // Until the foundparam/PM_HASHELEM scalar-shim path lands in
    // assignnparam, route subscripted writes through `assignsparam`
    // (which already handles PM_HASHED + PM_ARRAY[idx] writes at
    // params.rs:4880-4914). For PM_ARRAY targets pre-evaluate the
    // subscript body via `matheval` so `arr[i + 1]` becomes
    // `arr[3]` before assignsparam parses the body as i64 — same
    // dispatch C's getarg (params.c:1367) performs internally via
    // mathevalarg.
    if let Some(bi) = name.find('[') {
        let close = name.rfind(']').unwrap_or(name.len());
        let base = &name[..bi];
        let body = if close > bi { &name[bi + 1..close] } else { "" };
        // PM_HASHED → literal-string subscript (no math eval).
        // PM_ARRAY / unset → math-eval the subscript body.
        let is_hashed = {
            let tab = crate::ported::params::paramtab().read();
            tab.ok()
                .and_then(|t| {
                    t.get(base)
                        .map(|pm| PM_TYPE(pm.node.flags as u32) == PM_HASHED)
                })
                .unwrap_or(false)
        };
        let canonical = if is_hashed {
            name.to_string()
        } else {
            // Save/restore evaluator state around the recursive
            // matheval — mirrors getmathparam at math.rs:230 and
            // C mathevall's xyy* save/restore pattern (math.c:367).
            let saved = save_state();
            let idx_val = matheval(body)
                .map(|n| if n.type_ == MN_FLOAT { n.d as i64 } else { n.l })
                .unwrap_or(0);
            restore_state(saved);
            format!("{}[{}]", base, idx_val)
        };
        // Render mnumber as decimal string for assignsparam's
        // numeric subscript-write path. PM_ARRAY/PM_HASHED store
        // strings; assignsparam writes them straight through.
        let val_str = if val.type_ == MN_FLOAT {
            crate::ported::params::convfloat_underscore(val.d, 0)
        } else {
            crate::ported::params::convbase_underscore(val.l, 10, 0)
        };
        let _ = crate::ported::params::assignsparam(&canonical, &val_str, 0);
        // Cache the resolved (canonical) name so a subsequent read
        // of the same subscript in the SAME math expression sees
        // the new value without a paramtab round-trip.
        m_variables_insert(canonical, val);
        return val;
    }

    // Unsubscripted path — cache by name and route through setnparam
    // as before. The canonical paramtab write inside setnparam is
    // what makes the value persist beyond the current $((…)).
    m_variables_insert(name.to_string(), val);
    // c:1005 — `pm = setnparam(mvp->lval, v);`
    let pm = crate::ported::params::setnparam(name, val);
    // c:1006-1027 — re-type the return per the param's type after setnparam.
    if let Some(pm) = pm {
        let flags = pm.node.flags as u32;
        if flags & PM_INTEGER != 0 {
            let l = if val.type_ == MN_FLOAT {
                val.d as i64
            } else {
                val.l
            };
            return mnumber {
                l,
                d: 0.0,
                type_: MN_INTEGER,
            };
        }
        if flags & (PM_EFLOAT | PM_FFLOAT) != 0 {
            let d = if val.type_ == MN_INTEGER {
                val.l as f64
            } else {
                val.d
            };
            return mnumber {
                l: 0,
                d,
                type_: MN_FLOAT,
            };
        }
    }
    val
}

/// Call a math function
/// Port of `callmathfunc(char *o)` from `Src/math.c:1037`.
/// WARNING: param names don't match C — Rust=() vs C=(o)
pub(crate) fn callmathfunc(call: &str) -> mnumber {
    // Parse function name and args
    let paren = call.find('(').unwrap_or(call.len());
    let name = &call[..paren];
    // c:Src/math.c:1037 — `callmathfunc` looks up `name` in the
    // global `mathfuncs` table. The table is empty until
    // `zmodload zsh/mathfunc` (Src/Modules/mathfunc.c mtab[]) is
    // loaded. Without it, every named call fails with "unknown
    // function: NAME" (Src/math.c:1066). The previous Rust port
    // unconditionally dispatched against the built-in match arms,
    // auto-loading the module's contents — `zsh -fc 'echo
    // $((sqrt(4)))'` should exit 1, not silently return `2.`.
    let is_module_func = matches!(
        name,
        "abs"
            | "acos"
            | "acosh"
            | "asin"
            | "asinh"
            | "atan"
            | "atanh"
            | "cbrt"
            | "ceil"
            | "copysign"
            | "cos"
            | "cosh"
            | "erf"
            | "erfc"
            | "exp"
            | "expm1"
            | "fabs"
            | "float"
            | "floor"
            | "fmod"
            | "gamma"
            | "hypot"
            | "ilogb"
            | "int"
            | "j0"
            | "j1"
            | "jn"
            | "ldexp"
            | "lgamma"
            | "log"
            | "log10"
            | "log1p"
            | "log2"
            | "logb"
            | "nextafter"
            | "rand48"
            | "rint"
            | "scalb"
            | "sin"
            | "sinh"
            | "sqrt"
            | "tan"
            | "tanh"
            | "y0"
            | "y1"
            | "yn"
    );
    // c:Src/module.c:2206-2322 `load_module` — the post-init flag
    // signaling "this module's setup/boot ran" is MOD_INIT_B (set
    // at c:2322 after do_boot_module). MOD_LINKED alone is just
    // "statically linkable" and is pre-set for every builtin
    // module at registration time in modulestab::init_builtin
    // (zsh_h.rs:758) — so it's true even before any `zmodload`.
    // Gate on MOD_INIT_B to mirror C's "the module's mtab[] is
    // currently in the global mathfuncs table".
    let module_loaded = crate::ported::module::MODULESTAB
        .lock()
        .ok()
        .and_then(|tab| {
            tab.modules.get("zsh/mathfunc").map(|m| {
                let flags = m.node.flags;
                (flags & crate::ported::zsh_h::MOD_INIT_B) != 0
                    && (flags & crate::ported::zsh_h::MOD_UNLOAD) == 0
            })
        })
        .unwrap_or(false);
    // c:Src/math.c:1108-1116 — MFF_USERFUNC branch: when the named
    // math function points at a user shfunc (registered via
    // `functions -M`), dispatch via doshfunc instead of looking it
    // up in mathfuncs. The body sets `lastmathval` and returns;
    // callmathfunc reads it back. Routed here BEFORE the module
    // arms below so a user-registered fn shadows a built-in name
    // (matching C lookup order).
    //
    // C zsh's `Src/math.c:1037-1116 callmathfunc` walks the
    // canonical `mathfuncs` table (Src/module.c:1258) — a shell
    // function with the same name as the math call is NOT
    // dispatched unless an MFF_USERFUNC entry was installed via
    // `functions -M`. Bug #360: previously zshrs dispatched ANY
    // matching shfunc, so unregistered `myadd() {…}; $((myadd(2,3)))`
    // entered doshfunc and produced a math-error rather than
    // "unknown function: myadd" (the zsh behavior).
    //
    // Gate the dispatch on a present MFF_USERFUNC entry whose
    // shfunc handler resolves to `name` (per C math.c:1108's
    // `if (f->flags & MFF_USERFUNC)` check).
    // c:1109 — `shfnam = f->module ? f->module : n`. A `functions -M`
    // entry can map the math name to a DIFFERENT implementing shell
    // function (the optional 4th arg): `functions -M addtwo 2 2 _addtwo`
    // dispatches to `_addtwo`. Resolve the impl name from the entry's
    // `module` field, falling back to the math name.
    // c:1108-1109 + c:1106-1107 — resolve the implementing shfunc name
    // AND the registered [minargs, maxargs] bounds together, so the
    // arg-count check below sees the same entry that doshfunc dispatches
    // to.
    let userfunc_impl: Option<(String, i32, i32)> = crate::ported::module::MATHFUNCS
        .lock()
        .ok()
        .and_then(|tab| {
            tab.iter()
                .find(|p| p.name == name && (p.flags & crate::ported::zsh_h::MFF_USERFUNC) != 0)
                .map(|p| {
                    (
                        p.module.clone().unwrap_or_else(|| p.name.clone()),
                        p.minargs,
                        p.maxargs,
                    )
                })
        });
    if let Some((impl_name, minargs, maxargs)) = userfunc_impl {
        if let Some(mut shfunc) = crate::ported::utils::getshfunc(&impl_name) {
            // c:1059-1062 — `addlinknode(l, n)`: the FIRST positional ($0)
            // is the MATH function NAME (`max`/`min`), NOT the implementing
            // shfunc name. A shared impl (zmathfunc registers max/min/sum to
            // one function) switches on $0, so it must see the math name.
            // The body to RUN is still the impl shfunc.
            let mut largs: Vec<String> = vec![name.to_string()];
            let argv_str: Vec<String> = call[paren..]
                .trim_start_matches('(')
                .trim_end_matches(')')
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            // c:Src/math.c:1106-1107 — `if (argc >= f->minargs &&
            // (f->maxargs < 0 || argc <= f->maxargs))`. The actual arg count
            // (NOT counting the math-fn name pushed as $0 at c:1061) must be
            // within the registered bounds; `maxargs < 0` means unbounded.
            // On mismatch C falls to c:1127 `zerr("wrong number of
            // arguments: %s", o)` where `o` is the original `name(args)`
            // call text, and aborts the math eval. Without this check zshrs
            // dispatched the body anyway (e.g. a 0-arg `functions -M` fn
            // called as `cube(3)` ran the body instead of erroring).
            let argc = argv_str.len() as i32;
            if argc < minargs || (maxargs >= 0 && argc > maxargs) {
                crate::ported::utils::zerr(&format!("wrong number of arguments: {}", call)); // c:1127
                crate::ported::utils::errflag.fetch_or(
                    crate::ported::zsh_h::ERRFLAG_ERROR,
                    std::sync::atomic::Ordering::Relaxed,
                );
                return mnumber {
                    l: 0,
                    d: 0.0,
                    type_: MN_INTEGER,
                };
            }
            largs.extend(argv_str.iter().cloned());
            let name_for_body = impl_name.clone();
            let body_args = argv_str.clone();
            let body_runner = move || -> i32 {
                crate::ported::exec::run_function_body(&name_for_body, &body_args).unwrap_or(0)
            };
            // c:1114 — `doshfunc(shfunc, l, 1)`. The body runs a nested
            // `(( ))` which RE-ENTERS this evaluator and clobbers the outer
            // parser's input/pos/stack thread-locals; save + restore them
            // around the call so the OUTER `$(( fn(x) ))` keeps parsing
            // (without this it errored "operand expected at end of string").
            // M_LASTMATHVAL is NOT part of save_state, so the body's last
            // `(( ))` result survives the restore.
            let saved = save_state();
            let _ = crate::ported::exec::doshfunc(&mut shfunc, largs, true, body_runner);
            restore_state(saved);
            // c:1115 — `return lastmathval`. The body's last arithmetic
            // evaluation (e.g. `(( REPLY = $1 + 2 ))`) is the function's value.
            return M_LASTMATHVAL.with(|c| c.get());
        } else {
            // c:Src/math.c:1110-1112 — the math function IS registered
            // (MFF_USERFUNC via `functions -M`), but its implementing shell
            // function doesn't exist: `zerr("no such function: %s", shfnam)`.
            // This is distinct from the `unknown function` of an
            // UN-registered math name (c:1131); zshrs previously fell
            // through to that generic message.
            crate::ported::utils::zerr(&format!("no such function: {}", impl_name)); // c:1112
            crate::ported::utils::errflag.fetch_or(
                crate::ported::zsh_h::ERRFLAG_ERROR,
                std::sync::atomic::Ordering::Relaxed,
            );
            return mnumber {
                l: 0,
                d: 0.0,
                type_: MN_INTEGER,
            };
        }
    } // close `if mathfunc_entry.is_some()`

    if is_module_func && !module_loaded {
        // c:Src/math.c:1050 — `if ((f = getmathfunc(n, 1)))`: the
        // lookup with autol=1 IS the autoload fire. `zmodload -af
        // zsh/mathfunc sin` installs a MATHFUNCS stub (module.c:1410
        // add_automathfunc); getmathfunc removes the stub and
        // ensurefeature-loads the owning module (module.c:1289-1301).
        // On a hit, fall through to the evaluation arms below (the
        // module is now booted). Without this, the registered
        // autoload never fired and `$(( sin(0) ))` errored
        // `unknown function: sin` despite the -af registration.
        let autoloaded = crate::ported::module::MODULESTAB
            .lock()
            .ok()
            .map(|mut tab| crate::ported::module::getmathfunc(&mut tab, name, 1).is_some())
            .unwrap_or(false);
        if !autoloaded {
            crate::ported::utils::zerr(&format!("unknown function: {}", name));
            crate::ported::utils::errflag.fetch_or(
                crate::ported::zsh_h::ERRFLAG_ERROR,
                std::sync::atomic::Ordering::Relaxed,
            );
            return mnumber {
                l: 0,
                d: 0.0,
                type_: MN_INTEGER,
            };
        }
    }
    let args_str = if paren < call.len() {
        &call[paren + 1..call.len() - 1]
    } else {
        ""
    };

    // c:Src/math.c:1051-1052 — `if ((f->flags & (MFF_STR|MFF_USERFUNC))
    // == MFF_STR) return f->sfunc(n, a, f->funcid);`. A pure string
    // math function receives the raw, UN-evaluated arg text (here the
    // name of the parameter holding the seed) and is dispatched before
    // the numeric arg-eval below. `rand48` (mathfunc.c:154
    // STRMATHFUNC("rand48", math_string, MS_RAND48)) is the only one.
    if name == "rand48" {
        return math_string(name, args_str, MS_RAND48);
    }

    // Parse arguments. Keep both the float view (for trig) and the
    // original mnumber so int-preserving functions (abs/min/max/
    // int/floor/ceil/trunc) can return integer when all inputs
    // were integer.
    let arg_nums: Vec<mnumber> = if args_str.is_empty() {
        vec![]
    } else {
        args_str
            .split(',')
            .filter_map(|arg| {
                // Save caller's eval state, sub-eval each arg in a
                // fresh state inheriting caller's variables, restore.
                // C `mathevall()` xyy* save/restore (math.c:367).
                let saved = save_state();
                let inherited_vars = saved.variables.clone();
                new(arg.trim());
                m_variables_set(inherited_vars);
                let result = mathevall();
                restore_state(saved);
                // c:math.c::callmathfunc — when a function-arg subeval
                // fails, the C body's mathevall has already zerr'd the
                // parse error. Rust's mathevall captures the message
                // in Err; the previous .ok() discarded it silently,
                // so `$(( abs(1 2) ))` returned 0 instead of erroring.
                match result {
                    Ok(n) => Some(n),
                    Err(msg) => {
                        crate::ported::utils::zerr(&msg);
                        None
                    }
                }
            })
            .collect()
    };
    let args: Vec<f64> = arg_nums
        .iter()
        .map(|n| (if n.type_ == MN_FLOAT { n.d } else { n.l as f64 }))
        .collect();
    // c:Src/math.c:1106-1107 + c:1127 — every math function's arg count
    // must be within its registered [minargs, maxargs] bounds (maxargs<0
    // = unbounded); on mismatch C errors "wrong number of arguments:
    // NAME(args)" and aborts. The MFF_USERFUNC arity check above covers
    // shfunc-backed entries; this mirrors it for the NUMERIC built-in
    // dispatch so e.g. `atan(1,2,3)` errors (atan is registered 1..2)
    // instead of silently dropping the extra arg. Bounds come from the
    // ported NUMMATHFUNC table (modules/mathfunc.rs num()).
    if let Some((minargs, maxargs)) = crate::ported::module::MATHFUNCS
        .lock()
        .ok()
        .and_then(|tab| {
            tab.iter()
                .find(|p| p.name == name)
                .map(|p| (p.minargs, p.maxargs))
        })
    {
        let argc = args.len() as i32;
        if argc < minargs || (maxargs >= 0 && argc > maxargs) {
            crate::ported::utils::zerr(&format!("wrong number of arguments: {}", call)); // c:1127
            crate::ported::utils::errflag.fetch_or(
                crate::ported::zsh_h::ERRFLAG_ERROR,
                std::sync::atomic::Ordering::Relaxed,
            );
            return mnumber {
                l: 0,
                d: 0.0,
                type_: MN_INTEGER,
            };
        }
    }
    let all_int = !arg_nums.is_empty() && arg_nums.iter().all(|n| (n.type_ == MN_INTEGER));

    // c:Src/Modules/mathfunc.c:139 — only `int` has TFLAG(TF_NOASS)
    // which collapses the result to MN_INTEGER. `ceil`/`floor` lack
    // TF_NOASS so they return float (rendered as `5.` for whole
    // values), and `trunc` doesn't exist in zsh's mathfunc table at
    // all — it must error "unknown function: trunc" like zsh.
    // The previous Rust port forced all four to integer, so
    // `$(( ceil(1.1) ))` printed `2` instead of zsh's `2.`.
    let always_int = matches!(name, "int");
    if always_int {
        let i = match name {
            "int" => arg_nums
                .first()
                .map(|n| (if n.type_ == MN_FLOAT { n.d as i64 } else { n.l }))
                .unwrap_or(0),
            _ => 0,
        };
        return mnumber {
            l: i,
            d: 0.0,
            type_: MN_INTEGER,
        };
    }
    // c:Src/Modules/mathfunc.c:115 — only `abs` is a real mathfunc that
    // returns an integer when fed integers. `min`/`max` are NOT mathfunc
    // entries (zsh provides them only via the `zmathfunc` autoload, which
    // registers them as `functions -M` shfuncs handled by the userfunc
    // path above) — calling them through zsh/mathfunc errors "unknown
    // function". Keeping them here let `zmodload zsh/mathfunc; min(1,2)`
    // wrongly return a value.
    let int_preserving = matches!(name, "abs");
    if all_int && int_preserving {
        let i = match name {
            "abs" => arg_nums
                .first()
                .map(|n| (if n.type_ == MN_FLOAT { n.d as i64 } else { n.l }).abs())
                .unwrap_or(0),
            _ => 0,
        };
        return mnumber {
            l: i,
            d: 0.0,
            type_: MN_INTEGER,
        };
    }

    // c:Src/Modules/system.c:467/900 — zsh/system registers the
    // `systell` math function (NUMMATHFUNC("systell", math_systell)).
    // It returns lseek(fd, 0, SEEK_CUR). Dispatch to the ported
    // math_systell when zsh/system is loaded; otherwise fall through to
    // the "unknown function" error like any unregistered name (gated the
    // same way as the zsh/mathfunc functions above).
    if name == "systell" {
        let system_loaded = crate::ported::module::MODULESTAB
            .lock()
            .ok()
            .and_then(|tab| {
                tab.modules.get("zsh/system").map(|m| {
                    let flags = m.node.flags;
                    (flags & crate::ported::zsh_h::MOD_INIT_B) != 0
                        && (flags & crate::ported::zsh_h::MOD_UNLOAD) == 0
                })
            })
            .unwrap_or(false);
        if system_loaded {
            let argv: Vec<mnumber> = args
                .iter()
                .map(|&x| mnumber {
                    l: x as i64,
                    d: x,
                    type_: if x.fract() == 0.0 {
                        MN_INTEGER
                    } else {
                        MN_FLOAT
                    },
                })
                .collect();
            return crate::ported::modules::system::math_systell(
                "systell",
                argv.len() as i32,
                &argv,
                0,
            );
        }
    }

    // c:Src/Modules/mathfunc.c:24-44 — extern math fns provided by
    // libc on every UNIX. Rust's `f64` exposes most directly
    // (acosh/asinh/atanh/sqrt/...). The libgm-only ones (erf/erfc/
    // tgamma/lgamma/j0/j1/y0/y1/ilogb/logb/cbrt/expm1/log1p/
    // copysign/nextafter/fmod) need an explicit C ABI binding.
    #[cfg(unix)]
    extern "C" {
        fn erf(x: f64) -> f64;
        fn erfc(x: f64) -> f64;
        fn lgamma(x: f64) -> f64;
        fn tgamma(x: f64) -> f64;
        fn ilogb(x: f64) -> i32;
        fn logb(x: f64) -> f64;
        fn j0(x: f64) -> f64;
        fn j1(x: f64) -> f64;
        // c:Src/Modules/mathfunc.c:334/421 — `jn(argi, argd2)` /
        // `yn(argi, argd2)`: the ORDER is an int (TFLAG(TF_INT1)),
        // the argument a double.
        fn jn(n: i32, x: f64) -> f64;
        fn y0(x: f64) -> f64;
        fn y1(x: f64) -> f64;
        fn yn(n: i32, x: f64) -> f64;
        fn cbrt(x: f64) -> f64;
        fn expm1(x: f64) -> f64;
        fn log1p(x: f64) -> f64;
        fn copysign(x: f64, y: f64) -> f64;
        fn nextafter(x: f64, y: f64) -> f64;
        fn rint(x: f64) -> f64;
        fn fmod(x: f64, y: f64) -> f64;
        fn ldexp(x: f64, exp: i32) -> f64;
        fn scalbn(x: f64, exp: i32) -> f64;
    }
    // Built-in math functions — mirrors `math_func()` dispatch table
    // at Src/Modules/mathfunc.c:198-432.
    let result = match name {
        "abs" => args.first().map(|x| x.abs()).unwrap_or(0.0),
        "acos" => args.first().map(|x| x.acos()).unwrap_or(0.0),
        "acosh" => args.first().map(|x| x.acosh()).unwrap_or(0.0), // c:212
        "asin" => args.first().map(|x| x.asin()).unwrap_or(0.0),
        "asinh" => args.first().map(|x| x.asinh()).unwrap_or(0.0), // c:220
        // c:Src/Modules/mathfunc.c:225-229 — `atan` takes 1 OR 2 args:
        // the 2-arg form is atan2(y, x) (NUMMATHFUNC("atan", …, 1, 2)).
        // The previous port ignored the second arg and returned
        // atan(arg1), so `atan(3,2)` gave 1.249 instead of atan2(3,2)
        // = 0.98279. (The 3+-arg "wrong number of arguments" error
        // requires built-in math-func arity validation — see catalog.)
        "atan" => {
            if args.len() >= 2 {
                args[0].atan2(args[1]) // c:227
            } else {
                args.first().map(|x| x.atan()).unwrap_or(0.0) // c:229
            }
        }
        "atanh" => args.first().map(|x| x.atanh()).unwrap_or(0.0), // c:233
        "cbrt" => unsafe { cbrt(args.first().copied().unwrap_or(0.0)) }, // c:237
        "ceil" => args.first().map(|x| x.ceil()).unwrap_or(0.0),
        "copysign" => {
            let x = args.first().copied().unwrap_or(0.0);
            let y = args.get(1).copied().unwrap_or(0.0);
            unsafe { copysign(x, y) } // c:245
        }
        "cos" => args.first().map(|x| x.cos()).unwrap_or(1.0),
        "cosh" => args.first().map(|x| x.cosh()).unwrap_or(1.0),
        "erf" => unsafe { erf(args.first().copied().unwrap_or(0.0)) }, // c:257
        "erfc" => unsafe { erfc(args.first().copied().unwrap_or(0.0)) }, // c:261
        "exp" => args.first().map(|x| x.exp()).unwrap_or(1.0),
        "expm1" => unsafe { expm1(args.first().copied().unwrap_or(0.0)) }, // c:269
        "fabs" => args.first().map(|x| x.abs()).unwrap_or(0.0),            // c:273
        "floor" => args.first().map(|x| x.floor()).unwrap_or(0.0),
        "fmod" => {
            let x = args.first().copied().unwrap_or(0.0);
            let y = args.get(1).copied().unwrap_or(1.0);
            unsafe { fmod(x, y) } // c:285
        }
        "gamma" => unsafe { tgamma(args.first().copied().unwrap_or(0.0)) }, // c:289
        "hypot" => {
            let x = args.first().copied().unwrap_or(0.0);
            let y = args.get(1).copied().unwrap_or(0.0);
            x.hypot(y)
        }
        "ilogb" => unsafe { ilogb(args.first().copied().unwrap_or(0.0)) as f64 }, // c:304
        "int" => args.first().map(|x| x.trunc()).unwrap_or(0.0),
        "j0" => unsafe { j0(args.first().copied().unwrap_or(0.0)) }, // c:325
        "j1" => unsafe { j1(args.first().copied().unwrap_or(0.0)) }, // c:331
        // c:Src/Modules/mathfunc.c:144 `NUMMATHFUNC("jn", math_func, 2, 2,
        // MF_JN | TFLAG(TF_INT1))` + c:333-335 `retd = jn(argi, argd2);`.
        // TF_INT1 (c:106) means the FIRST argument is the integer one —
        // the mirror of ldexp/scalb's TF_INT2 below.
        "jn" => {
            let n = args.first().copied().unwrap_or(0.0) as i32;
            let x = args.get(1).copied().unwrap_or(0.0);
            unsafe { jn(n, x) } // c:334
        }
        "ldexp" => {
            // c:Src/Modules/mathfunc.c:337 MF_LDEXP — `ldexp(argd, argi)`,
            // 2nd arg coerced to int (TF_INT2). Returns x * 2^n.
            let x = args.first().copied().unwrap_or(0.0);
            let n = args.get(1).copied().unwrap_or(0.0) as i32;
            unsafe { ldexp(x, n) }
        }
        "scalb" => {
            // c:Src/Modules/mathfunc.c:378 MF_SCALB — `scalbn(argd, argi)`.
            let x = args.first().copied().unwrap_or(0.0);
            let n = args.get(1).copied().unwrap_or(0.0) as i32;
            unsafe { scalbn(x, n) }
        }
        "lgamma" => unsafe { lgamma(args.first().copied().unwrap_or(0.0)) }, // c:341
        "log" => args.first().map(|x| x.ln()).unwrap_or(0.0),
        "log10" => args.first().map(|x| x.log10()).unwrap_or(0.0),
        "log1p" => unsafe { log1p(args.first().copied().unwrap_or(0.0)) }, // c:357
        "log2" => args.first().map(|x| x.log2()).unwrap_or(0.0),
        "logb" => unsafe { logb(args.first().copied().unwrap_or(0.0)) }, // c:365
        "nextafter" => {
            let x = args.first().copied().unwrap_or(0.0);
            let y = args.get(1).copied().unwrap_or(0.0);
            unsafe { nextafter(x, y) } // c:373
        }
        // c:Src/Modules/mathfunc.c:374 — `retd = rint(argd)` (round to
        // nearest, ties to even). Note zsh has NO `round`/`pow`/`rand`
        // mathfunc — `**` is the power operator and `round` doesn't exist.
        "rint" => unsafe { rint(args.first().copied().unwrap_or(0.0)) },
        "sin" => args.first().map(|x| x.sin()).unwrap_or(0.0),
        "sinh" => args.first().map(|x| x.sinh()).unwrap_or(0.0),
        "sqrt" => args.first().map(|x| x.sqrt()).unwrap_or(0.0),
        "tan" => args.first().map(|x| x.tan()).unwrap_or(0.0),
        "tanh" => args.first().map(|x| x.tanh()).unwrap_or(0.0),
        "y0" => unsafe { y0(args.first().copied().unwrap_or(0.0)) }, // c:417
        "y1" => unsafe { y1(args.first().copied().unwrap_or(0.0)) }, // c:423
        // c:Src/Modules/mathfunc.c:168 `NUMMATHFUNC("yn", math_func, 2, 2,
        // MF_YN | TFLAG(TF_INT1))` + c:420-422 `retd = yn(argi, argd2);`.
        "yn" => {
            let n = args.first().copied().unwrap_or(0.0) as i32;
            let x = args.get(1).copied().unwrap_or(0.0);
            unsafe { yn(n, x) } // c:421
        }
        // `float(x)` — widen int/float to float. Identity on
        // floats; on ints, returns same value tagged as float so
        // `printf "%.4f"` prints "3.0000" instead of "3". Direct
        // port of mathfunc.c's `to_float()`.
        "float" => args.first().copied().unwrap_or(0.0),
        _ => {
            m_error_set(format!("unknown function: {}", name));
            0.0
        }
    };

    // c:Src/Modules/mathfunc.c — MF_ILOGB / MF_INT / MF_ISINF / MF_ISNAN
    // set `ret.type = MN_INTEGER` (e.g. `ilogb(8)` → 3, not 3.). Tag the
    // integer-returning functions so the result prints as an int.
    if matches!(name, "ilogb" | "int") {
        return mnumber {
            l: result as i64,
            d: 0.0,
            type_: MN_INTEGER,
        };
    }
    mnumber {
        l: 0,
        d: result,
        type_: MN_FLOAT,
    }
}

/// `MS_RAND48` — the only id in the string-mathfunc enum.
/// c:Src/Modules/mathfunc.c:90-92 `enum { MS_RAND48 };`.
const MS_RAND48: i32 = 0;

/// Port of `math_string(name, arg, id)` (Src/Modules/mathfunc.c:438).
/// String math functions receive their argument VERBATIM (un-evaluated)
/// — for `rand48` the arg is the name of the parameter holding (and
/// receiving) the 48-bit seed state as 12 hex digits.
fn math_string(_name: &str, arg: &str, id: i32) -> mnumber {
    extern "C" {
        // c:erand48(xsubi[3]) — next double in [0,1), advances seed in place.
        fn erand48(xsubi: *mut u16) -> f64;
        fn seed48(seed16v: *mut u16) -> *mut u16;
        fn rand() -> i32;
    }
    let mut ret = mnumber {
        l: 0,
        d: 0.0,
        type_: MN_INTEGER,
    }; // c:440 zero_mnumber
       // c:446-453 — trim leading/trailing blanks from the verbatim arg.
    let arg = arg.trim_matches(|c: char| c == ' ' || c == '\t');
    match id {
        MS_RAND48 => {
            // c:460-461 — `static unsigned short seedbuf[3]; static int
            // seedbuf_init;` persist the default (no-arg) seed across calls.
            thread_local! {
                static SEEDBUF: std::cell::Cell<[u16; 3]> = const { std::cell::Cell::new([0; 3]) };
                static SEEDBUF_INIT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
            }
            let mut tmp_seedbuf: [u16; 3] = [0; 3]; // c:462
            let use_static; // seedbufptr == seedbuf (the static default)
            let mut do_init = true; // c:463
            if !arg.is_empty() {
                // c:465-494 — seed comes from the named parameter.
                use_static = false; // c:467 seedbufptr = tmp_seedbuf
                                    // c:468 — `(seedstr = getsparam(arg)) && strlen(seedstr) >= 12`
                if let Some(seedstr) = crate::ported::params::getsparam(arg) {
                    let sb = seedstr.as_bytes();
                    if sb.len() >= 12 {
                        do_init = false; // c:470
                        let mut p = 0usize; // walks seedstr (c: seedstr++)
                                            // c:474-492 — decode three u16 from 12 hex digits.
                        for i in 0..3 {
                            if do_init {
                                break;
                            } // c:474 `i < 3 && !do_init`
                            let mut val: u16 = 0; // c:476 *seedptr = 0
                            for j in 0..4 {
                                let b = sb[p];
                                if b.is_ascii_digit() {
                                    val = val.wrapping_add((b - b'0') as u16); // c:480
                                } else {
                                    let lc = (b as char).to_ascii_lowercase();
                                    if ('a'..='f').contains(&lc) {
                                        // c:482-483
                                        val = val.wrapping_add((lc as u8 - b'a' + 10) as u16);
                                    } else {
                                        do_init = true; // c:486
                                        break;
                                    }
                                }
                                p += 1; // c:489 seedstr++
                                if j < 3 {
                                    val = val.wrapping_mul(16); // c:491
                                }
                            }
                            tmp_seedbuf[i] = val;
                        }
                    }
                }
            } else {
                // c:497-506 — default static seed; initialise once, then
                // re-init on every later call (do_init stays/becomes true).
                use_static = true; // c:500 seedbufptr = seedbuf
                if !SEEDBUF_INIT.with(|c| c.get()) {
                    SEEDBUF_INIT.with(|c| c.set(true)); // c:503
                } else {
                    do_init = true; // c:505
                }
            }
            // Working seed in a local we can pass by mutable pointer.
            let mut seed: [u16; 3] = if use_static {
                SEEDBUF.with(|c| c.get())
            } else {
                tmp_seedbuf
            };
            if do_init {
                // c:507-518 — seed from rand(); seed48 for impls that need it.
                seed[0] = unsafe { rand() } as u16;
                seed[1] = unsafe { rand() } as u16;
                seed[2] = unsafe { rand() } as u16;
                unsafe {
                    seed48(seed.as_mut_ptr());
                }
            }
            ret.type_ = MN_FLOAT; // c:520
            ret.d = unsafe { erand48(seed.as_mut_ptr()) }; // c:521
            if use_static {
                SEEDBUF.with(|c| c.set(seed)); // persist advanced static state
            }
            if !arg.is_empty() {
                // c:523-529 — write the advanced state back as 12 hex digits.
                let outbuf = format!("{:04x}{:04x}{:04x}", seed[0], seed[1], seed[2]);
                crate::ported::params::setsparam(arg, &outbuf);
            }
        }
        _ => {}
    }
    ret
}

/// Port of `op(int what)` from `Src/math.c:1154`.
///
/// Apply a binary or unary operator to the operand stack. Pops
/// 1-2 values, applies the operation (with type coercion), and
/// pushes the result. Handles assignment (`OP_E2*` flag) by
/// writing through `setmathvar` and pushing the new value back
/// with the same lvalue so chained assigns work.
/// WARNING: param names don't match C — Rust=() vs C=(what)
pub(crate) fn op(what: i32) {
    if m_error_some() {
        return;
    }

    let tp = OP_TYPE[what as usize];

    // Binary operators
    if (tp & (OP_A2 | OP_A2IR | OP_A2IO | OP_E2 | OP_E2IO)) != 0 {
        if m_stack_len() < 2 {
            // zsh's exact wording for the same condition is
            // `bad math expression: operand expected at end of
            // string`. Matching it here means `let "1+"` and
            // `$((5+))` produce the same diagnostic shape that
            // scripts grep for.
            m_error_set("bad math expression: operand expected at end of string".to_string());
            return;
        }

        let b = pop();
        let mv_a = pop_with_lval();
        let a = if (mv_a.val.type_ == MN_UNSET) {
            if let Some(ref name) = mv_a.lval {
                getmathparam(name)
            } else {
                mnumber {
                    l: 0,
                    d: 0.0,
                    type_: MN_INTEGER,
                }
            }
        } else {
            mv_a.val
        };

        // Coerce types
        let (a, b) = if (tp & (OP_A2IO | OP_E2IO)) != 0 {
            // Must be integers
            (
                mnumber {
                    l: (if a.type_ == MN_FLOAT { a.d as i64 } else { a.l }),
                    d: 0.0,
                    type_: MN_INTEGER,
                },
                mnumber {
                    l: (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l }),
                    d: 0.0,
                    type_: MN_INTEGER,
                },
            )
        } else if (a.type_ == MN_FLOAT) != (b.type_ == MN_FLOAT) && what != COMMA {
            // Different types, coerce to float
            (
                mnumber {
                    l: 0,
                    d: (if a.type_ == MN_FLOAT { a.d } else { a.l as f64 }),
                    type_: MN_FLOAT,
                },
                mnumber {
                    l: 0,
                    d: (if b.type_ == MN_FLOAT { b.d } else { b.l as f64 }),
                    type_: MN_FLOAT,
                },
            )
        } else {
            (a, b)
        };

        let result = if m_noeval() > 0 {
            mnumber {
                l: 0,
                d: 0.0,
                type_: MN_INTEGER,
            }
        } else {
            let is_float = (a.type_ == MN_FLOAT);
            match what {
                AND | ANDEQ => mnumber {
                    l: (if a.type_ == MN_FLOAT { a.d as i64 } else { a.l })
                        & (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l }),
                    d: 0.0,
                    type_: MN_INTEGER,
                },
                XOR | XOREQ => mnumber {
                    l: (if a.type_ == MN_FLOAT { a.d as i64 } else { a.l })
                        ^ (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l }),
                    d: 0.0,
                    type_: MN_INTEGER,
                },
                OR | OREQ => mnumber {
                    l: (if a.type_ == MN_FLOAT { a.d as i64 } else { a.l })
                        | (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l }),
                    d: 0.0,
                    type_: MN_INTEGER,
                },

                MUL | MULEQ => {
                    if is_float {
                        mnumber {
                            l: 0,
                            d: (if a.type_ == MN_FLOAT { a.d } else { a.l as f64 })
                                * (if b.type_ == MN_FLOAT { b.d } else { b.l as f64 }),
                            type_: MN_FLOAT,
                        }
                    } else {
                        mnumber {
                            l: (if a.type_ == MN_FLOAT { a.d as i64 } else { a.l })
                                .wrapping_mul((if b.type_ == MN_FLOAT { b.d as i64 } else { b.l })),
                            d: 0.0,
                            type_: MN_INTEGER,
                        }
                    }
                }

                DIV | DIVEQ => {
                    // Float div-by-zero is NOT an error in zsh —
                    // it produces IEEE Inf/-Inf/NaN per IEEE 754.
                    // Only INTEGER div-by-zero raises the error.
                    // Without this gate `1/0.0` errored out instead
                    // of returning `Inf`.
                    if is_float {
                        // Let f64 semantics handle 0.0, -0.0, NaN.
                        mnumber {
                            l: 0,
                            d: (if a.type_ == MN_FLOAT { a.d } else { a.l as f64 })
                                / (if b.type_ == MN_FLOAT { b.d } else { b.l as f64 }),
                            type_: MN_FLOAT,
                        }
                    } else {
                        if !notzero(b) {
                            m_error_set("division by zero".to_string());
                            return;
                        }
                        let bi = (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l });
                        if bi == -1 {
                            mnumber {
                                l: (if a.type_ == MN_FLOAT { a.d as i64 } else { a.l })
                                    .wrapping_neg(),
                                d: 0.0,
                                type_: MN_INTEGER,
                            }
                        } else {
                            mnumber {
                                l: (if a.type_ == MN_FLOAT { a.d as i64 } else { a.l }) / bi,
                                d: 0.0,
                                type_: MN_INTEGER,
                            }
                        }
                    }
                }

                MOD | MODEQ => {
                    if is_float {
                        // float % 0.0 → NaN per IEEE; let it fall
                        // through to f64 semantics rather than
                        // raising the integer-only error.
                        mnumber {
                            l: 0,
                            d: (if a.type_ == MN_FLOAT { a.d } else { a.l as f64 })
                                % (if b.type_ == MN_FLOAT { b.d } else { b.l as f64 }),
                            type_: MN_FLOAT,
                        }
                    } else if !notzero(b) {
                        m_error_set("division by zero".to_string());
                        return;
                    } else {
                        let bi = (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l });
                        if bi == -1 {
                            mnumber {
                                l: 0,
                                d: 0.0,
                                type_: MN_INTEGER,
                            }
                        } else {
                            mnumber {
                                l: (if a.type_ == MN_FLOAT { a.d as i64 } else { a.l }) % bi,
                                d: 0.0,
                                type_: MN_INTEGER,
                            }
                        }
                    }
                }

                PLUS | PLUSEQ => {
                    if is_float {
                        mnumber {
                            l: 0,
                            d: (if a.type_ == MN_FLOAT { a.d } else { a.l as f64 })
                                + (if b.type_ == MN_FLOAT { b.d } else { b.l as f64 }),
                            type_: MN_FLOAT,
                        }
                    } else {
                        mnumber {
                            l: (if a.type_ == MN_FLOAT { a.d as i64 } else { a.l })
                                .wrapping_add((if b.type_ == MN_FLOAT { b.d as i64 } else { b.l })),
                            d: 0.0,
                            type_: MN_INTEGER,
                        }
                    }
                }

                MINUS | MINUSEQ => {
                    if is_float {
                        mnumber {
                            l: 0,
                            d: (if a.type_ == MN_FLOAT { a.d } else { a.l as f64 })
                                - (if b.type_ == MN_FLOAT { b.d } else { b.l as f64 }),
                            type_: MN_FLOAT,
                        }
                    } else {
                        mnumber {
                            l: (if a.type_ == MN_FLOAT { a.d as i64 } else { a.l })
                                .wrapping_sub((if b.type_ == MN_FLOAT { b.d as i64 } else { b.l })),
                            d: 0.0,
                            type_: MN_INTEGER,
                        }
                    }
                }

                SHLEFT | SHLEFTEQ => mnumber {
                    l: (if a.type_ == MN_FLOAT { a.d as i64 } else { a.l })
                        << ((if b.type_ == MN_FLOAT { b.d as i64 } else { b.l }) as u32 & 63),
                    d: 0.0,
                    type_: MN_INTEGER,
                },
                SHRIGHT | SHRIGHTEQ => mnumber {
                    l: (if a.type_ == MN_FLOAT { a.d as i64 } else { a.l })
                        >> ((if b.type_ == MN_FLOAT { b.d as i64 } else { b.l }) as u32 & 63),
                    d: 0.0,
                    type_: MN_INTEGER,
                },

                LES => mnumber {
                    l: if is_float {
                        ((if a.type_ == MN_FLOAT { a.d } else { a.l as f64 })
                            < (if b.type_ == MN_FLOAT { b.d } else { b.l as f64 }))
                            as i64
                    } else {
                        ((if a.type_ == MN_FLOAT { a.d as i64 } else { a.l })
                            < (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l }))
                            as i64
                    },
                    d: 0.0,
                    type_: MN_INTEGER,
                },
                LEQ => mnumber {
                    l: if is_float {
                        ((if a.type_ == MN_FLOAT { a.d } else { a.l as f64 })
                            <= (if b.type_ == MN_FLOAT { b.d } else { b.l as f64 }))
                            as i64
                    } else {
                        ((if a.type_ == MN_FLOAT { a.d as i64 } else { a.l })
                            <= (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l }))
                            as i64
                    },
                    d: 0.0,
                    type_: MN_INTEGER,
                },
                GRE => mnumber {
                    l: if is_float {
                        ((if a.type_ == MN_FLOAT { a.d } else { a.l as f64 })
                            > (if b.type_ == MN_FLOAT { b.d } else { b.l as f64 }))
                            as i64
                    } else {
                        ((if a.type_ == MN_FLOAT { a.d as i64 } else { a.l })
                            > (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l }))
                            as i64
                    },
                    d: 0.0,
                    type_: MN_INTEGER,
                },
                GEQ => mnumber {
                    l: if is_float {
                        ((if a.type_ == MN_FLOAT { a.d } else { a.l as f64 })
                            >= (if b.type_ == MN_FLOAT { b.d } else { b.l as f64 }))
                            as i64
                    } else {
                        ((if a.type_ == MN_FLOAT { a.d as i64 } else { a.l })
                            >= (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l }))
                            as i64
                    },
                    d: 0.0,
                    type_: MN_INTEGER,
                },
                DEQ => mnumber {
                    l: if is_float {
                        ((if a.type_ == MN_FLOAT { a.d } else { a.l as f64 })
                            == (if b.type_ == MN_FLOAT { b.d } else { b.l as f64 }))
                            as i64
                    } else {
                        ((if a.type_ == MN_FLOAT { a.d as i64 } else { a.l })
                            == (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l }))
                            as i64
                    },
                    d: 0.0,
                    type_: MN_INTEGER,
                },
                NEQ => mnumber {
                    l: if is_float {
                        ((if a.type_ == MN_FLOAT { a.d } else { a.l as f64 })
                            != (if b.type_ == MN_FLOAT { b.d } else { b.l as f64 }))
                            as i64
                    } else {
                        ((if a.type_ == MN_FLOAT { a.d as i64 } else { a.l })
                            != (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l }))
                            as i64
                    },
                    d: 0.0,
                    type_: MN_INTEGER,
                },

                DAND | DANDEQ => mnumber {
                    l: ((if a.type_ == MN_FLOAT { a.d as i64 } else { a.l }) != 0
                        && (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l }) != 0)
                        as i64,
                    d: 0.0,
                    type_: MN_INTEGER,
                },
                DOR | DOREQ => mnumber {
                    l: ((if a.type_ == MN_FLOAT { a.d as i64 } else { a.l }) != 0
                        || (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l }) != 0)
                        as i64,
                    d: 0.0,
                    type_: MN_INTEGER,
                },
                DXOR | DXOREQ => {
                    let ai = (if a.type_ == MN_FLOAT { a.d as i64 } else { a.l }) != 0;
                    let bi = (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l }) != 0;
                    mnumber {
                        l: (ai != bi) as i64,
                        d: 0.0,
                        type_: MN_INTEGER,
                    }
                }

                POWER | POWEREQ => {
                    // c:1335 — POWER '**'
                    let mut a = a;
                    let mut b = b;
                    let mut cf = is_float; // c.type == MN_FLOAT
                                           // c:1337 — integer base with a negative integer exponent
                                           // "produces a real result, so cast to real." The cast of a
                                           // to float MUST happen before the zero check below: notzero
                                           // never faults on a float zero, so the all-integer
                                           // `0 ** -n` becomes pow(0.0,-n)=Inf rather than an error.
                    if !cf && b.l < 0 {
                        a = mnumber {
                            l: 0,
                            d: a.l as f64,
                            type_: MN_FLOAT,
                        }; // c:1340
                        b = mnumber {
                            l: 0,
                            d: b.l as f64,
                            type_: MN_FLOAT,
                        }; // c:1341
                        cf = true; // c:1339 (a.type = b.type = c.type = MN_FLOAT)
                    }
                    if !cf {
                        // c:1344 — for (c.u.l = 1; b.u.l--; c.u.l *= a.u.l).
                        // zsh's naive O(e) loop times out on a pathological
                        // exponent (`0 ** 4.6e9` loops billions of times).
                        // zshrs computes the IDENTICAL value via
                        // exponentiation-by-squaring in O(log e): multiplication
                        // mod 2^64 is associative, so the wrapped product is
                        // bit-identical to the repeated-multiply result for every
                        // base/exponent (verified against zsh across overflow
                        // cases). b.l is >= 0 here (negative exponents were cast
                        // to float above).
                        let base = a.l;
                        let mut e = b.l;
                        let mut result = 1i64;
                        let mut acc = base;
                        while e > 0 {
                            if e & 1 == 1 {
                                result = result.wrapping_mul(acc);
                            }
                            e >>= 1;
                            if e > 0 {
                                acc = acc.wrapping_mul(acc);
                            }
                        }
                        mnumber {
                            l: result,
                            d: 0.0,
                            type_: MN_INTEGER,
                        }
                    } else {
                        let af = (if a.type_ == MN_FLOAT { a.d } else { a.l as f64 });
                        let bf = (if b.type_ == MN_FLOAT { b.d } else { b.l as f64 });
                        // c:1346 — `if (b.u.d <= 0 && !notzero(a)) return;`
                        // notzero faults (division by zero) only on an INTEGER
                        // zero, so a base that was cast to float above slips
                        // through and yields Inf; a genuine integer-zero base
                        // (e.g. `0 ** -4.0`, no cast) still errors.
                        if bf <= 0.0 && !notzero(a) {
                            m_error_set("division by zero".to_string());
                            return;
                        }
                        // c:1348 — (-num ** b) with non-integer b is imaginary
                        if af < 0.0 && bf != bf.trunc() {
                            m_error_set("bad math expression: imaginary power".to_string()); // c:1350
                            return;
                        }
                        mnumber {
                            l: 0,
                            d: af.powf(bf), // c:1356
                            type_: MN_FLOAT,
                        }
                    }
                }

                COMMA => b,
                EQ => b,

                _ => mnumber {
                    l: 0,
                    d: 0.0,
                    type_: MN_INTEGER,
                },
            }
        };

        // Handle assignment
        if (tp & (OP_E2 | OP_E2IO)) != 0 {
            if let Some(ref name) = mv_a.lval {
                let final_val = setmathvar(name, result);
                push(final_val, Some(name.clone()));
            } else {
                // c:Src/math.c:997 — `zerr("bad math expression: lvalue
                // required")`. The prefix was missing here (unlike the sibling
                // sites at getvar/setvar), so `(( 1 = 2 ))` printed
                // `lvalue required` instead of `bad math expression: lvalue
                // required`. Bug #1025.
                m_error_set("bad math expression: lvalue required".to_string());
                push(
                    mnumber {
                        l: 0,
                        d: 0.0,
                        type_: MN_INTEGER,
                    },
                    None,
                );
            }
        } else {
            push(result, None);
        }
        return;
    }

    // Unary operators
    if m_stack_is_empty() {
        // zsh: unary op with empty stack -> `bad math
        // expression: operand expected at end of string`.
        // zshrs's bare `stack empty` had no match for scripts
        // grepping zsh's canonical wording.
        m_error_set("bad math expression: operand expected at end of string".to_string());
        return;
    }

    let mv = pop_with_lval();
    let val = if (mv.val.type_ == MN_UNSET) {
        if let Some(ref name) = mv.lval {
            getmathparam(name)
        } else {
            mnumber {
                l: 0,
                d: 0.0,
                type_: MN_INTEGER,
            }
        }
    } else {
        mv.val
    };

    match what {
        NOT => {
            let result = mnumber {
                l: if ((val.type_ == MN_INTEGER && val.l == 0)
                    || (val.type_ == MN_FLOAT && val.d == 0.0)
                    || val.type_ == MN_UNSET)
                {
                    1
                } else {
                    0
                },
                d: 0.0,
                type_: MN_INTEGER,
            };
            push(result, None);
        }
        COMP => {
            let result = mnumber {
                l: !(if val.type_ == MN_FLOAT {
                    val.d as i64
                } else {
                    val.l
                }),
                d: 0.0,
                type_: MN_INTEGER,
            };
            push(result, None);
        }
        UPLUS => {
            push(val, None);
        }
        UMINUS => {
            let result = if (val.type_ == MN_FLOAT) {
                mnumber {
                    l: 0,
                    d: -(if val.type_ == MN_FLOAT {
                        val.d
                    } else {
                        val.l as f64
                    }),
                    type_: MN_FLOAT,
                }
            } else {
                // c:Src/math.c UMINUS — negating INT_MIN is UB in
                // C but in two's complement wraps to INT_MIN. zsh
                // prints \`-9223372036854775808\` for \`\$((-(2**63)))\`.
                // Rust's plain unary `-` panics in debug builds on
                // i64::MIN, so use wrapping_neg.
                let v = if val.type_ == MN_FLOAT {
                    val.d as i64
                } else {
                    val.l
                };
                mnumber {
                    l: v.wrapping_neg(),
                    d: 0.0,
                    type_: MN_INTEGER,
                }
            };
            push(result, None);
        }
        POSTPLUS => {
            // ++/-- on a literal (`5++`, `--5`) is a zsh error:
            // "bad math expression: lvalue required". Without the
            // mv.lval guard, zshrs silently incremented the
            // literal value and returned it, masking the bug.
            if mv.lval.is_none() {
                m_error_set("bad math expression: lvalue required".to_string());
                return;
            }
            let name = mv.lval.as_ref().unwrap();
            let new_val = if (val.type_ == MN_FLOAT) {
                mnumber {
                    l: 0,
                    d: (if val.type_ == MN_FLOAT {
                        val.d
                    } else {
                        val.l as f64
                    }) + 1.0,
                    type_: MN_FLOAT,
                }
            } else {
                mnumber {
                    l: (if val.type_ == MN_FLOAT {
                        val.d as i64
                    } else {
                        val.l
                    }) + 1,
                    d: 0.0,
                    type_: MN_INTEGER,
                }
            };
            setmathvar(name, new_val);
            push(val, None); // Return original value
        }
        POSTMINUS => {
            if mv.lval.is_none() {
                m_error_set("bad math expression: lvalue required".to_string());
                return;
            }
            let name = mv.lval.as_ref().unwrap();
            let new_val = if (val.type_ == MN_FLOAT) {
                mnumber {
                    l: 0,
                    d: (if val.type_ == MN_FLOAT {
                        val.d
                    } else {
                        val.l as f64
                    }) - 1.0,
                    type_: MN_FLOAT,
                }
            } else {
                mnumber {
                    l: (if val.type_ == MN_FLOAT {
                        val.d as i64
                    } else {
                        val.l
                    }) - 1,
                    d: 0.0,
                    type_: MN_INTEGER,
                }
            };
            setmathvar(name, new_val);
            push(val, None);
        }
        PREPLUS => {
            if mv.lval.is_none() {
                m_error_set("bad math expression: lvalue required".to_string());
                return;
            }
            let name = mv.lval.as_ref().unwrap();
            let new_val = if (val.type_ == MN_FLOAT) {
                mnumber {
                    l: 0,
                    d: (if val.type_ == MN_FLOAT {
                        val.d
                    } else {
                        val.l as f64
                    }) + 1.0,
                    type_: MN_FLOAT,
                }
            } else {
                mnumber {
                    l: (if val.type_ == MN_FLOAT {
                        val.d as i64
                    } else {
                        val.l
                    }) + 1,
                    d: 0.0,
                    type_: MN_INTEGER,
                }
            };
            setmathvar(name, new_val);
            push(new_val, mv.lval);
        }
        PREMINUS => {
            if mv.lval.is_none() {
                m_error_set("bad math expression: lvalue required".to_string());
                return;
            }
            let name = mv.lval.as_ref().unwrap();
            let new_val = if (val.type_ == MN_FLOAT) {
                mnumber {
                    l: 0,
                    d: (if val.type_ == MN_FLOAT {
                        val.d
                    } else {
                        val.l as f64
                    }) - 1.0,
                    type_: MN_FLOAT,
                }
            } else {
                mnumber {
                    l: (if val.type_ == MN_FLOAT {
                        val.d as i64
                    } else {
                        val.l
                    }) - 1,
                    d: 0.0,
                    type_: MN_INTEGER,
                }
            };
            setmathvar(name, new_val);
            push(new_val, mv.lval);
        }
        QUEST => {
            // Ternary: stack has [cond, true_val, false_val]
            // val already popped = false_val
            // Need to pop true_val and cond
            if m_stack_len() < 2 {
                m_error_set("?: needs 3 operands".to_string());
                return;
            }
            let false_val = val;
            let true_val = pop();
            let cond = pop();
            let result = if !((cond.type_ == MN_INTEGER && cond.l == 0)
                || (cond.type_ == MN_FLOAT && cond.d == 0.0)
                || cond.type_ == MN_UNSET)
            {
                true_val
            } else {
                false_val
            };
            push(result, None);
        }
        COLON => {
            m_error_set("bad math expression: ':' without '?'".to_string()); // c:1427
        }
        _ => {
            m_error_set("unknown operator".to_string());
        }
    }
}

/// Port of `bop(int tk)` from `Src/math.c:1454`.
///
/// Short-circuit boolean prologue. Inspects (without popping) the
/// top of stack and bumps `m_noeval()` for the parse-only side of
/// `&&` / `||` / their assignment forms. The matching decrement
/// happens after `mathparse` recurses for the RHS.
/// WARNING: param names don't match C — Rust=() vs C=(tk)
pub(crate) fn bop(tk: i32) {
    if m_stack_is_empty() {
        return;
    }
    let mv = m_stack_top_clone().unwrap();
    let val = if (mv.val.type_ == MN_UNSET) {
        if let Some(ref name) = mv.lval {
            getmathparam(name)
        } else {
            mnumber {
                l: 0,
                d: 0.0,
                type_: MN_INTEGER,
            }
        }
    } else {
        mv.val
    };

    // c:Src/math.c:1461 — `tst = (spval->type & MN_FLOAT) ? (zlong)spval->u.d
    // : spval->u.l;`. A FLOAT operand is TRUNCATED to integer for the
    // short-circuit truth test (`(zlong)0.5` == 0 → falsy), NOT compared
    // against 0.0. zsh's `&&`/`||` therefore treat a fractional float like
    // 0.5 as false. The prior `val.d == 0.0` test made 0.5 spuriously TRUE,
    // so `0.5 || (2+3)` short-circuited on the truthy 0.5 and set noeval for
    // the RHS; a COMPOUND RHS under noeval collapses to a dummy 0, and the
    // `||` operator then combined truncated-0 with that 0 → wrong result 0
    // (zsh evaluates the RHS and yields 1). A bare-literal RHS masked the
    // bug because its value survives noeval.
    let tst = if val.type_ & MN_FLOAT != 0 {
        (val.d as i64) != 0
    } else {
        val.l != 0
    };
    match tk {
        DAND | DANDEQ if !tst => {
            m_noeval_inc();
        }
        DOR | DOREQ if tst => {
            m_noeval_inc();
        }
        _ => {}
    }
}

/// Port of `mnumber matheval(char *s)` from `Src/math.c:1480`.
///
/// C body (c:1481-1500):
/// ```c
/// char *junk;
/// mnumber x;
/// int xmtok = mtok;
/// /* maintain outputradix and outputunderscore across levels of evaluation */
/// if (!mlevel)
///     outputradix = outputunderscore = 0;
///
/// if (*s == Nularg)
///     s++;
/// if (!*s) {
///     x.type = MN_INTEGER;
///     x.u.l = 0;
///     return x;
/// }
/// x = mathevall(s, MPREC_TOP, &junk);
/// mtok = xmtok;
/// if (*junk)
///     zerr("bad math expression: illegal character: %c", *junk);
/// return x;
/// ```
///
/// Three divergences in the previous Rust port:
///   1. Missing Nularg-byte skip at c:1489-1490 — `$(())` lexes
///      with a leading Nularg sentinel; without the skip, the math
///      evaluator chokes on the 0xa1 byte instead of evaluating
///      the empty expression as 0.
///   2. Missing empty-input fast path at c:1491-1495 — empty
///      string returned MN_INTEGER 0 in C; Rust port tried to
///      evaluate via mathevall and produced a parse error.
///   3. Missing mtok save/restore around mathevall (c:1483, c:1496) —
///      recursive math calls (e.g. `$((f($((g)))))`) overwrote the
///      outer call's mtok mid-parse.
pub fn matheval(s: &str) -> Result<mnumber, String> {
    // c:1480
    // c:1483 — `int xmtok = mtok;` save.
    let xmtok = M_MTOK.with(|c| c.get()); // c:1483

    // c:1489-1490 — `if (*s == Nularg) s++;`. The 0xa1 sentinel byte
    // can prefix expressions emerging from the parser; skip it.
    let s = if let Some(rest) = s.strip_prefix(Nularg) {
        // c:1489
        rest
    } else {
        s
    };
    // c:1486-1487 — `if (!mlevel) outputradix = outputunderscore = 0;`
    //
    // Only a TOP-LEVEL evaluation starts from a clean radix. When `mlevel` is
    // already nonzero this `matheval` is running underneath an in-flight one
    // (a `functions -M` math function whose body runs `(( … ))`), and C leaves
    // the radix alone so the outer `[#16]` still governs the printed result.
    if M_LEVEL.with(|c| c.get()) == 0 {
        // c:1486
        reset_output_format(); // c:1487
    }

    // c:1491-1495 — empty expression returns MN_INTEGER 0.
    if s.is_empty() {
        // c:1491
        return Ok(mnumber {
            l: 0,
            d: 0.0,
            type_: MN_INTEGER,
        }); // c:1493-1494
    }
    new(s);
    let result = mathevall();
    // c:1496 — `mtok = xmtok;` restore. Done even on error path.
    M_MTOK.with(|c| c.set(xmtok)); // c:1496
                                   // c:Src/math.c:1500 — `lastmathval = z;` records the result of this
                                   // top-level eval so callmathfunc's MFF_USERFUNC branch can return it.
    if let Ok(ref n) = result {
        M_LASTMATHVAL.with(|c| c.set(*n));
    }
    result
}

/// Port of `mnumber matheval(char *s)` integer-coerce front-end
/// `mod_export zlong mathevali(char *s)` from Src/math.c:1505.
///
/// C body (c:1505-1509):
/// ```c
/// mnumber x = matheval(s);
/// return (x.type & MN_FLOAT) ? (zlong)x.u.d : x.u.l;
/// ```
///
/// Uses bitwise AND against MN_FLOAT — `x.type` is a bitfield holding
/// MN_INTEGER (1), MN_FLOAT (2), MN_UNSET (4). The previous Rust port
/// did `n.type_ == MN_FLOAT` (strict equality) — which misclassifies
/// any result where MN_FLOAT is set alongside another flag (e.g. an
/// uninitialized-then-set result might carry MN_FLOAT|MN_UNSET = 6).
pub fn mathevali(s: &str) -> Result<i64, String> {
    // c:1505
    matheval(s).map(|n|                                                      // c:1506
        if (n.type_ & MN_FLOAT) != 0 { n.d as i64 } else { n.l }) // c:1508
}

/// Variant of `mathevali` that runs in NOEVAL mode — parses and
/// type-checks but does NOT execute side effects (assignments to
/// paramtab via setmathvar's c:1002-1003 noeval gate). Used by the
/// compile-time pre-check at compile_zsh.rs to validate `(( expr ))`
/// without polluting the param table. Bug #617.
pub fn mathevali_noeval(s: &str) -> Result<i64, String> {
    // new() inside matheval resets noeval to 0; we work around that
    // by intercepting at matheval's entry. Run matheval, but bump
    // noeval AFTER new() has reset it — by hooking via the mathevall
    // path with noeval pre-set wouldn't work because new() also resets.
    //
    // Solution: replicate matheval's setup but set noeval manually
    // before mathevall. mathevall itself respects noeval inside the
    // op() dispatch — setmathvar checks at c:1002.
    let xmtok = M_MTOK.with(|c| c.get());
    let s_skip = if let Some(rest) = s.strip_prefix(Nularg) {
        rest
    } else {
        s
    };
    if s_skip.is_empty() {
        return Ok(0);
    }
    new(s_skip);
    m_noeval_set(1); // bump AFTER new() reset
    let result = mathevall();
    m_noeval_set(0);
    M_MTOK.with(|c| c.set(xmtok));
    result.map(|n| {
        if (n.type_ & MN_FLOAT) != 0 {
            n.d as i64
        } else {
            n.l
        }
    })
}

/// Port of `zlong mathevalarg(char *s, char **ss)` from `Src/math.c:1514-1539`.
///
/// C body (c:1517-1538):
/// ```c
/// mnumber x;
/// int xmtok = mtok;
/// /* At this entry point we don't allow an empty expression,
///  * whereas we do with matheval(). */
/// if (*s == Nularg)
///     s++;
/// if (!*s) {
///     zerr("bad math expression: empty string");
///     return (zlong)0;
/// }
/// x = mathevall(s, MPREC_ARG, ss);
/// if (mtok == COMMA)
///     (*ss)--;
/// mtok = xmtok;
/// return (x.type & MN_FLOAT) ? (zlong)x.u.d : x.u.l;
/// ```
///
/// Two key differences from `matheval`:
///   1. Empty input is an ERROR (zerr + return 0), NOT silent 0.
///      C's comment: `$array[$ind]` where `$ind` is unset should
///      produce an error, not silently index 0.
///   2. Uses `MPREC_ARG` precedence so the parser stops at the
///      end-of-arg boundary (comma, close-paren) rather than
///      consuming everything as one top-level expression.
///      (Rust mathevall doesn't yet thread the prec_tp arg;
///      flagged for follow-up.)
pub(crate) fn mathevalarg(expr: &str) -> i64 {
    // c:1514
    // c:1517 — `int xmtok = mtok;` save.
    let xmtok = M_MTOK.with(|c| c.get()); // c:1517
                                          // c:1528-1529 — `if (*s == Nularg) s++;`. Skip the parser sentinel.
    let s = if let Some(rest) = expr.strip_prefix(Nularg) {
        // c:1528
        rest
    } else {
        expr
    };
    // c:1530-1532 — empty after Nularg-skip is a HARD error here.
    if s.is_empty() {
        // c:1530
        zerr("bad math expression: empty string"); // c:1531
        return 0; // c:1532
    }
    // c:1534 — `mathevall(s, MPREC_ARG, ss)`. The Rust port doesn't yet
    // thread the prec_tp arg through mathevall (uses C_PREC/Z_PREC toggle
    // only); structural follow-up.
    // c:1538 — `(x.type & MN_FLOAT) ? (zlong)x.u.d : x.u.l`. Bitwise
    // check against MN_FLOAT; strict equality `== MN_FLOAT` misclassifies
    // composite type bitfields (e.g. MN_FLOAT|MN_UNSET).
    let result = matheval(s).map(|n|                                         // c:1538
        if (n.type_ & MN_FLOAT) != 0 { n.d as i64 } else { n.l }
    ).unwrap_or(0);
    // c:1537 — `mtok = xmtok;` restore.
    M_MTOK.with(|c| c.set(xmtok)); // c:1537
    result
}

/// Port of `checkunary(int mtokc, char *mptr)` from `Src/math.c:1548`.
///
/// Two roles. (1) Validate that the just-lexed token (`m_mtok()`)
/// matches the parser's expectation: an operand was wanted but an
/// operator (`OP_*` flags) showed up, or vice versa. Mismatch
/// emits zsh's `bad math expression: <kind> expected at <ctx>`
/// with `<kind>` being `operator` or `operand` and `<ctx>` taken
/// from the input pointer at the start of the bad token. (2)
/// Update `m_unary()` for the next iteration based on `OP_OPF`.
/// WARNING: param names don't match C — Rust=() vs C=(mtokc, mptr)
pub(crate) fn checkunary() {
    // Direct port of zsh math.c checkunary() (line 1548).
    // Two roles:
    //   1. Validate that the just-lexed token (`m_mtok()`)
    //      matches the parser's expectation (operator vs
    //      operand). Mismatch emits zsh's
    //      "bad math expression: <kind> expected at <ctx>"
    //      with `<kind>` = `operator` (errmsg=2) or `operand`
    //      (errmsg=1). zshrs previously only did step 2,
    //      which left e.g. `let "5 5"` and `$((2#1011x))`
    //      silently accepting bogus input.
    //   2. Update `m_unary()` for the next iteration.
    let tp = OP_TYPE[m_mtok() as usize];
    let is_op_token = (tp & (OP_A2 | OP_A2IR | OP_A2IO | OP_E2 | OP_E2IO | OP_OP)) != 0;
    let errmsg = if is_op_token {
        if m_unary() {
            1
        } else {
            0
        }
    } else if !m_unary() {
        2
    } else {
        0
    };
    if errmsg != 0 && !m_error_some() {
        let errtype = if errmsg == 2 { "operator" } else { "operand" };
        // zsh's `mptr` is the input position BEFORE zzlex
        // consumed the bad token. We track the same via
        // `tok_start` which zzlex updates after whitespace
        // skip. Walk forward past whitespace (mirrors zsh's
        // `inblank` skip) so the error context starts at
        // the first visible char.
        let input_owned = m_input_clone();
        let bytes = input_owned.as_bytes();
        let mut start = m_tok_start();
        while start < bytes.len() && matches!(bytes[start], b' ' | b'\t' | b'\n') {
            start += 1;
        }
        // zsh truncates after 10 chars and appends `...` if
        // there's more remaining (the over flag in the C
        // source). Mirror that to keep error messages
        // bounded for long bogus expressions.
        let remaining = m_input_slice_from(start);
        let (ctx, over) = if remaining.chars().count() > 10 {
            let truncated: String = remaining.chars().take(10).collect();
            (truncated, true)
        } else {
            (remaining.to_string(), false)
        };
        if ctx.is_empty() {
            m_error_set(format!(
                "bad math expression: {} expected at end of string",
                errtype
            ));
        } else {
            m_error_set(format!(
                "bad math expression: {} expected at `{}{}'",
                errtype,
                ctx,
                if over { "..." } else { "" }
            ));
        }
    }
    m_unary_set((tp & OP_OPF) == 0);
}

/// Operator-precedence parser - closely follows zsh math.c mathparse()
/// Port of `mathparse(int pc)` from `Src/math.c:1594`.
/// WARNING: param names don't match C — Rust=() vs C=(pc)
pub(crate) fn mathparse(pc: u8) {
    if m_error_some() {
        return;
    }

    m_mtok_set(zzlex());

    // Handle empty input
    if pc == top_prec() && m_mtok() == EOI {
        return;
    }

    checkunary();

    while m_prec()[m_mtok() as usize] <= pc {
        if m_error_some() {
            return;
        }

        match m_mtok() {
            NUM => {
                push(m_yyval(), None);
            }
            ID => {
                let lval = m_yylval_clone();
                if m_noeval() > 0 {
                    push(
                        mnumber {
                            l: 0,
                            d: 0.0,
                            type_: MN_INTEGER,
                        },
                        Some(lval),
                    );
                } else {
                    push(
                        mnumber {
                            l: 0,
                            d: 0.0,
                            type_: MN_UNSET,
                        },
                        Some(lval),
                    );
                }
            }
            CID => {
                let lval = m_yylval_clone();
                let val = if m_noeval() > 0 {
                    mnumber {
                        l: 0,
                        d: 0.0,
                        type_: MN_INTEGER,
                    }
                } else {
                    getcvar(&lval)
                };
                push(val, Some(lval));
            }
            FUNC => {
                let func_call = m_yylval_clone();
                let val = if m_noeval() > 0 {
                    mnumber {
                        l: 0,
                        d: 0.0,
                        type_: MN_INTEGER,
                    }
                } else {
                    callmathfunc(&func_call)
                };
                push(val, None);
            }
            M_INPAR => {
                mathparse(top_prec());
                if m_mtok() != M_OUTPAR {
                    if !m_error_some() {
                        // Match zsh's `bad math expression: ')'
                        // expected` so error diagnostics align.
                        m_error_set("bad math expression: ')' expected".to_string());
                    }
                    return;
                }
            }
            QUEST => {
                // Ternary operator
                if m_stack_is_empty() {
                    m_error_set("bad math expression".to_string());
                    return;
                }
                let mv = m_stack_top_clone().unwrap();
                let cond = get_value(&mv);

                let q = !((cond.type_ == MN_INTEGER && cond.l == 0)
                    || (cond.type_ == MN_FLOAT && cond.d == 0.0)
                    || cond.type_ == MN_UNSET);
                if !q {
                    m_noeval_inc();
                }
                let colon_prec = m_prec()[COLON as usize];
                let stack_before = m_stack_len();
                mathparse(colon_prec - 1);
                if !q {
                    m_noeval_dec();
                }

                if m_mtok() != COLON {
                    if !m_error_some() {
                        // Distinguish whether the inner parse
                        // produced an operand: stack grew →
                        // colon expected; stack same → operand
                        // missing (input ran out at end of
                        // string after `?`).
                        if m_stack_len() > stack_before {
                            m_error_set("bad math expression: ':' expected".to_string());
                        } else {
                            m_error_set(
                                "bad math expression: operand expected at end of string"
                                    .to_string(),
                            );
                        }
                    }
                    return;
                }

                if q {
                    m_noeval_inc();
                }
                let quest_prec = m_prec()[QUEST as usize];
                mathparse(quest_prec);
                if q {
                    m_noeval_dec();
                }

                op(QUEST);
                continue;
            }
            _ => {
                // Binary/unary operator
                let otok = m_mtok();
                let onoeval = m_noeval();
                let tp = OP_TYPE[otok as usize];
                // Orphan binary at start: `let "*"`, `let "*5"`,
                // `let "/"`. zsh keeps its input pointer at the
                // start of the bad operator and emits `operand
                // expected at \`<remaining>'`. zshrs previously
                // collapsed every operand-missing case into "at
                // end of string" which lost the operator
                // location for orphan-at-start expressions.
                let is_binary = (tp & (OP_A2 | OP_A2IR | OP_A2IO | OP_E2 | OP_E2IO)) != 0;
                if m_stack_is_empty() && is_binary {
                    let remaining = m_input_slice_from(m_tok_start());
                    m_error_set(format!(
                        "bad math expression: operand expected at `{}'",
                        remaining
                    ));
                    return;
                }
                if (tp & 0x03) == BOOL {
                    bop(otok);
                }
                let otok_prec = m_prec()[otok as usize];
                // Right-to-left gets same prec, left-to-right gets prec-1
                let adjust = if (tp & 0x01) != RL { 1 } else { 0 };
                mathparse(otok_prec - adjust);
                m_noeval_set(onoeval);
                op(otok);
                continue;
            }
        }

        // After operand (Num, Id, Func, InPar), get next token
        m_mtok_set(zzlex());
        checkunary();
    }
}
/// Zsh precedence table (default)
static Z_PREC: [u8; TOKCOUNT] = [
    1, 137, 2, 2, 2, // InPar OutPar Not Comp PostPlus
    2, 2, 2, 4, 5, // PostMinus UPlus UMinus And Xor
    6, 8, 8, 8, 9, // Or Mul Div Mod Plus
    9, 3, 3, 10, 10, // Minus ShLeft ShRight Les Leq
    10, 10, 11, 11, 12, // Gre Geq Deq Neq DAnd
    13, 13, 14, 15, 16, // DOr DXor Quest Colon Eq
    16, 16, 16, 16, 16, // PlusEq MinusEq MulEq DivEq ModEq
    16, 16, 16, 16, 16, // AndEq XorEq OrEq ShLeftEq ShRightEq
    16, 16, 16, 17, 200, // DAndEq DOrEq DXorEq Comma Eoi
    2, 2, 0, 0, 7, // PrePlus PreMinus Num Id Power
    0, 16, 0, // CId PowerEq Func
];

/// C precedence table (used with C_PRECEDENCES option)
static C_PREC: [u8; TOKCOUNT] = [
    1, 137, 2, 2, 2, 2, 2, 2, 9, 10, 11, 4, 4, 4, 5, 5, 6, 6, 7, 7, 7, 7, 8, 8, 12, 14, 13, 15, 16,
    17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 18, 200, 2, 2, 0, 0, 3, 0, 17, 0,
];

/// Operator type table (matches C math.c type[] array)
static OP_TYPE: [u16; TOKCOUNT] = [
    // InPar, OutPar, Not, Comp, PostPlus
    LR,
    LR | OP_OP | OP_OPF,
    RL,
    RL,
    RL | OP_OP | OP_OPF,
    // PostMinus, UPlus, UMinus, And, Xor
    RL | OP_OP | OP_OPF,
    RL,
    RL,
    LR | OP_A2IO,
    LR | OP_A2IO,
    // Or, Mul, Div, Mod, Plus
    LR | OP_A2IO,
    LR | OP_A2,
    LR | OP_A2,
    LR | OP_A2,
    LR | OP_A2,
    // Minus, ShLeft, ShRight, Les, Leq
    LR | OP_A2,
    LR | OP_A2IO,
    LR | OP_A2IO,
    LR | OP_A2IR,
    LR | OP_A2IR,
    // Gre, Geq, Deq, Neq, DAnd
    LR | OP_A2IR,
    LR | OP_A2IR,
    LR | OP_A2IR,
    LR | OP_A2IR,
    BOOL | OP_A2IO,
    // DOr, DXor, Quest, Colon, Eq
    BOOL | OP_A2IO,
    LR | OP_A2IO,
    RL | OP_OP,
    RL | OP_OP,
    RL | OP_E2,
    // PlusEq, MinusEq, MulEq, DivEq, ModEq
    RL | OP_E2,
    RL | OP_E2,
    RL | OP_E2,
    RL | OP_E2,
    RL | OP_E2,
    // AndEq, XorEq, OrEq, ShLeftEq, ShRightEq
    RL | OP_E2IO,
    RL | OP_E2IO,
    RL | OP_E2IO,
    RL | OP_E2IO,
    RL | OP_E2IO,
    // DAndEq, DOrEq, DXorEq, Comma, Eoi
    BOOL | OP_E2IO,
    BOOL | OP_E2IO,
    RL | OP_A2IO,
    RL | OP_A2,
    RL | OP_OP,
    // PrePlus, PreMinus, Num, Id, Power
    RL,
    RL,
    LR | OP_OPF,
    LR | OP_OPF,
    RL | OP_A2,
    // CId, PowerEq, Func
    LR | OP_OPF,
    RL | OP_E2,
    LR | OP_OPF,
];

// WARNING: NOT IN MATH.C — Rust-only helper. See save_state above.
fn restore_state(saved: xyy_locals) {
    m_input_set(saved.input);
    m_pos_set(saved.pos);
    m_tok_start_set(saved.tok_start);
    m_yyval_set(saved.yyval);
    m_yylval_set(saved.yylval);
    M_STACK.with(|c| *c.borrow_mut() = saved.stack);
    m_mtok_set(saved.mtok);
    m_unary_set(saved.unary);
    m_noeval_set(saved.noeval);
    M_ERROR.with(|c| *c.borrow_mut() = saved.error);
    m_variables_set(saved.variables);
    m_string_variables_set(saved.string_variables);
    m_prec_set(saved.prec);
    m_c_precedences_set(saved.c_precedences);
    m_force_float_set(saved.force_float);
    m_octal_zeroes_set(saved.octal_zeroes);
    M_LASTBASE.with(|c| c.set(saved.lastbase));
}

// MathState struct DELETED — state now lives in M_* thread_locals
// (matching C math.c's module statics + mathevall's xyy* save/restore).

// WARNING: NOT IN MATH.C — Rust-only initializer. C `mathevall()`
// (math.c:367) takes the input as a parameter and seeds the module
// statics inline at function entry; Rust port factors that seeding
// out so call sites can chain `with_*` setters before invoking
// `mathevall()`.
/// Initialize thread_local math state from a fresh input string.
/// Mirrors the entry-side state setup in C `mathevall()` (math.c:367).
pub(crate) fn new(input: &str) {
    m_input_set(input.to_string());
    m_pos_set(0);
    m_tok_start_set(0);
    m_yyval_set(mnumber {
        l: 0,
        d: 0.0,
        type_: MN_INTEGER,
    });
    m_yylval_set(String::new());
    M_STACK.with(|c| {
        c.borrow_mut().clear();
    });
    m_mtok_set(EOI);
    m_unary_set(true);
    m_noeval_set(0);
    m_lastbase_set(-1);
    m_prec_set(&Z_PREC);
    m_c_precedences_set(false);
    m_force_float_set(false);
    m_octal_zeroes_set(false);
    m_variables_set(HashMap::new());
    m_string_variables_set(HashMap::new());
    m_lastval_set(0);
    m_pid_set(std::process::id() as i64);
    m_error_clear();
}

// WARNING: NOT IN MATH.C — Rust-only setter. zsh C reads parameters
// directly from the global param table on demand; the Rust port
// caller seeds an in-memory map up front via this fn.
pub(crate) fn with_variables(vars: HashMap<String, mnumber>) {
    m_variables_set(vars);
}

// WARNING: NOT IN MATH.C — Rust-only setter. Parses each value as
// numeric → `mnumber` if possible, otherwise stores the raw string
// for `getmathparam`'s recursive-eval path (e.g. `a="3+2"; $((a))`).
/// Inject variables from string->string mapping (for shell integration)
pub(crate) fn with_string_variables(vars: &HashMap<String, String>) {
    for (k, v) in vars {
        if let Ok(i) = v.parse::<i64>() {
            m_variables_insert(
                k.clone(),
                mnumber {
                    l: i,
                    d: 0.0,
                    type_: MN_INTEGER,
                },
            );
        } else if let Ok(f) = v.parse::<f64>() {
            m_variables_insert(
                k.clone(),
                mnumber {
                    l: 0,
                    d: f,
                    type_: MN_FLOAT,
                },
            );
        } else if !v.is_empty() {
            // Non-numeric string — keep raw so getmathparam can
            // recursively evaluate it as an arith expression.
            // zsh: `a="3+2"; $((a))` returns 5.
            m_string_variables_insert(k.clone(), v.clone());
        }
    }
}

// WARNING: NOT IN MATH.C — Rust-only accessor. zsh C writes back
// to the global param table during evaluation; ShellExecutor
// integration uses this to harvest the post-eval variables map and
// merge it into its own `variables` table.
/// Extract modified variables as string->string mapping (for shell integration)
pub(crate) fn extract_string_variables() -> HashMap<String, String> {
    M_VARIABLES.with(|c| {
        c.borrow()
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    match v.type_ {
                        MN_INTEGER => v.l.to_string(),
                        MN_FLOAT => {
                            let f = v.d;
                            if isnan(f) {
                                "NaN".to_string()
                            } else if isinf(f) {
                                if f > 0.0 {
                                    "Inf".to_string()
                                } else {
                                    "-Inf".to_string()
                                }
                            } else {
                                format!("{:.10}", f)
                            }
                        }
                        _ => "0".to_string(),
                    },
                )
            })
            .collect()
    })
}

// WARNING: NOT IN MATH.C — Rust-only setopt mirror. zsh C reads
// the option flag directly from `isset(CPRECEDENCES)` inside
// `mathevall()`; this setter caches the bit so the evaluator
// avoids re-reading the option tree on every token.
pub(crate) fn with_c_precedences(enable: bool) {
    m_c_precedences_set(enable);
    m_prec_set(if enable { &C_PREC } else { &Z_PREC });
}

// WARNING: NOT IN MATH.C — Rust-only setopt mirror for FORCE_FLOAT.
pub(crate) fn with_force_float(enable: bool) {
    m_force_float_set(enable);
}

// WARNING: NOT IN MATH.C — Rust-only setopt mirror for OCTAL_ZEROES.
pub(crate) fn with_octal_zeroes(enable: bool) {
    m_octal_zeroes_set(enable);
}

// WARNING: NOT IN MATH.C — Rust-only setter for `$?` (last command
// status) so the `?`-token in unary position can read it. zsh C
// reads `lastval` directly as a global.
pub(crate) fn with_lastval(val: i32) {
    m_lastval_set(val);
}

// WARNING: NOT IN MATH.C — Rust-only cursor read. C uses `*ptr`
// directly without an fn-shaped wrapper.
pub(crate) fn peek() -> Option<char> {
    // Read the char in place. This used to be
    // `m_input_clone()[m_pos()..].chars().next()`, which heap-allocates a
    // COPY OF THE WHOLE EXPRESSION to look at one character — and `advance()`
    // calls this per character, so lexing an n-char expression performed n
    // full-string allocations, O(n^2) bytes copied per parse. `(( … ))` is
    // parsed on every evaluation (compile_arith emits LoadConst(<expr text>)
    // + CallBuiltin(ARITH_EVAL); nothing is compiled ahead of time), so this
    // ran once per character per loop iteration. c:Src/math.c uses `*ptr`
    // on the caller's buffer — no copy at all.
    M_INPUT.with(|c| c.borrow()[m_pos()..].chars().next())
}

// WARNING: NOT IN MATH.C — Rust-only cursor advance. C uses
// `*ptr++` directly.
pub(crate) fn advance() -> Option<char> {
    let c = peek()?;
    m_pos_add(c.len_utf8());
    Some(c)
}

// WARNING: NOT IN MATH.C — Rust-only char classifier. C uses
// ctype.h `idigit()` macro directly.
fn is_digit(c: char) -> bool {
    c.is_ascii_digit()
}

// WARNING: NOT IN MATH.C — Rust-only char classifier. C uses
// `iident()` / `isalpha()` macros directly.
fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

// WARNING: NOT IN MATH.C — Rust-only char classifier. C uses
// `iident()` macro directly.
fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

// WARNING: NOT IN MATH.C — Rust-only stack helper. C inlines
// this inside `pop()` (math.c:931) — its `noget` flag controls
// whether to resolve the deferred Unset+lval read; zshrs splits
// the two paths into separate ported so the resolved-vs-raw choice
// is at the call site.
pub(crate) fn pop_with_lval() -> mathvalue {
    m_stack_pop().unwrap_or_default()
}

// WARNING: NOT IN MATH.C — Rust-only value-resolver. C inlines
// the deferred-variable-read pattern inside `pop()` and `op()`
// (math.c:931, 1154); the Rust port factors it out for `bop`
// and `mathparse` to inspect-without-consuming.
pub(crate) fn get_value(mv: &mathvalue) -> mnumber {
    if (mv.val.type_ == MN_UNSET) {
        if let Some(ref name) = mv.lval {
            return getmathparam(name);
        }
    }
    mv.val
}

// WARNING: NOT IN MATH.C — Rust-only helper. C inlines the
// expression `prec[COMMA] + 1` directly in mathparse() and
// mathevall() everywhere it's needed (math.c:1594, 367).
pub(crate) fn top_prec() -> u8 {
    m_prec()[COMMA as usize] + 1
}

// WARNING: NOT IN MATH.C — Rust-only accessor (note plural — singular
// `getmathparam` IS in math.c:337). zsh C's caller reads the param
// table directly post-eval; this returns a snapshot of the in-memory
// variables map for ShellExecutor integration.
/// Get updated variables after evaluation
pub(crate) fn getmathparams() -> HashMap<String, mnumber> {
    m_variables_clone()
}

// ===========================================================
// Methods moved verbatim from src/ported/vm_helper because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: math
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

// ===========================================================
// Free ported moved verbatim from src/ported/vm_helper.
// ===========================================================
// BEGIN moved-from-exec-rs (free ported)
/// Pop argc arguments from the VM stack into a Vec<String>.
///
/// `Value::Array` entries (produced by `${arr[@]}`, glob expansion, brace
/// expansion, etc.) splice into multiple argv-style args — same flattening
/// rule as fusevm's `Op::Exec`. Without this, a builtin like `echo
/// ${arr[@]}` with `arr=(x y z)` would receive a single space-joined arg
/// `"x y z"` instead of three separate args.
/// Subscript-arith parser namespace. Holds the three pre-resolve parsers
/// `eval_arith_expr` runs against an expression before substituting array
/// references — the C source's `mathexpr()` (Src/math.c) inlines this work
/// inside the lexer, but Rust splits it out so the assignment-target arms
/// don't get confused with read sites.
// WARNING: NOT IN MATH.C — Rust-only string parser. C `setmathvar`
// (math.c:972) walks the lvalue pointer left in place by zzlex,
// so subscripted compound assigns fall out of the lexer for free.
// zshrs sees `((a[i]+=v))` as raw text and must split it before
// pre_resolve_array_subscripts substitutes the read value in place.
#[inline]
/// Detect `name[idx]=rhs` (or `name[idx]+=rhs`, etc.) at the start of
/// an arith expression. Returns (name, idx_expr, rhs). Used by
/// `eval_arith_expr` to handle `((a[i]=expr))` — the regular pre-
/// resolve pass would substitute a[i] with its current value first,
/// turning the expression into `0=42` which is invalid.
/// Parse `name[idx]OP rhs?` where OP is `++`, `--`, `+=`, `-=`, etc.
/// Returns (name, idx_expr, op, rhs). For `++`/`--`, rhs is empty.
pub(crate) fn parse_compound(expr: &str) -> Option<(String, String, String, String)> {
    let trimmed = expr.trim();
    let bytes = trimmed.as_bytes();
    if bytes.is_empty() || !(bytes[0] == b'_' || bytes[0].is_ascii_alphabetic()) {
        return None;
    }
    let mut i = 1;
    while i < bytes.len() && (bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric()) {
        i += 1;
    }
    let name = trimmed[..i].to_string();
    if i >= bytes.len() || bytes[i] != b'[' {
        return None;
    }
    let idx_start = i + 1;
    let mut depth = 1;
    let mut j = idx_start;
    while j < bytes.len() && depth > 0 {
        match bytes[j] {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        j += 1;
    }
    if j >= bytes.len() {
        return None;
    }
    let idx_expr = trimmed[idx_start..j].to_string();
    let mut k = j + 1;
    while k < bytes.len() && bytes[k].is_ascii_whitespace() {
        k += 1;
    }
    if k >= bytes.len() {
        return None;
    }
    let rest = &bytes[k..];
    // Try 3-char operators first (`<<=`, `>>=`, `**=`), then 2-char
    // (`++`, `--`, `+=`, `-=`, `*=`, `/=`, `%=`, `&=`, `|=`, `^=`).
    let (op, op_len) = match rest {
        [b'<', b'<', b'=', ..] => ("<<=", 3),
        [b'>', b'>', b'=', ..] => (">>=", 3),
        [b'*', b'*', b'=', ..] => ("**=", 3),
        [b'+', b'+', ..] => ("++", 2),
        [b'-', b'-', ..] => ("--", 2),
        [b'+', b'=', ..] => ("+=", 2),
        [b'-', b'=', ..] => ("-=", 2),
        [b'*', b'=', ..] => ("*=", 2),
        [b'/', b'=', ..] => ("/=", 2),
        [b'%', b'=', ..] => ("%=", 2),
        [b'&', b'=', ..] => ("&=", 2),
        [b'|', b'=', ..] => ("|=", 2),
        [b'^', b'=', ..] => ("^=", 2),
        _ => return None,
    };
    let rhs = trimmed[k + op_len..].trim().to_string();
    // For `++` / `--`, the rhs MUST be empty (anything else would be
    // a parse error). For `+=` etc., rhs is the value expression.
    if (op == "++" || op == "--") && !rhs.is_empty() {
        return None;
    }
    Some((name, idx_expr, op.to_string(), rhs))
}
// WARNING: NOT IN MATH.C — Rust-only string parser. C handles
// `++NAME[IDX]` via the lexer leaving the lvalue pointer set; the
// Rust port pre-parses the text. See parse_compound above.
/// Pre-increment/decrement on subscript: `++NAME[IDX]` / `--NAME[IDX]`.
/// Returns (name, idx_expr, op) where op is "++" or "--".
pub(crate) fn parse_pre_inc(expr: &str) -> Option<(String, String, String)> {
    let trimmed = expr.trim();
    let (after_op, pre_op) = if let Some(s) = trimmed.strip_prefix("++") {
        (s, "++")
    } else if let Some(s) = trimmed.strip_prefix("--") {
        (s, "--")
    } else {
        return None;
    };
    let after_op = after_op.trim_start();
    let bytes = after_op.as_bytes();
    if bytes.is_empty() || !(bytes[0] == b'_' || bytes[0].is_ascii_alphabetic()) {
        return None;
    }
    let mut i = 1;
    while i < bytes.len() && (bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric()) {
        i += 1;
    }
    let name = after_op[..i].to_string();
    if i >= bytes.len() || bytes[i] != b'[' {
        return None;
    }
    let idx_start = i + 1;
    let mut depth = 1;
    let mut j = idx_start;
    while j < bytes.len() && depth > 0 {
        match bytes[j] {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        j += 1;
    }
    if j >= bytes.len() {
        return None;
    }
    let idx_expr = after_op[idx_start..j].to_string();
    // After ], must be end of input (or whitespace).
    let mut k = j + 1;
    while k < bytes.len() && bytes[k].is_ascii_whitespace() {
        k += 1;
    }
    if k != bytes.len() {
        return None;
    }
    Some((name, idx_expr, pre_op.to_string()))
}
// WARNING: NOT IN MATH.C — Rust-only string parser for `NAME[IDX]=v`.
// See parse_compound above for the rationale.
pub(crate) fn parse_assign(expr: &str) -> Option<(String, String, String)> {
    let trimmed = expr.trim();
    let bytes = trimmed.as_bytes();
    if bytes.is_empty() || !(bytes[0] == b'_' || bytes[0].is_ascii_alphabetic()) {
        return None;
    }
    let mut i = 1;
    while i < bytes.len() && (bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric()) {
        i += 1;
    }
    let name = trimmed[..i].to_string();
    if i >= bytes.len() || bytes[i] != b'[' {
        return None;
    }
    let idx_start = i + 1;
    let mut depth = 1;
    let mut j = idx_start;
    while j < bytes.len() && depth > 0 {
        match bytes[j] {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        j += 1;
    }
    if j >= bytes.len() {
        return None;
    }
    let idx_expr = trimmed[idx_start..j].to_string();
    // Skip ]
    let mut k = j + 1;
    while k < bytes.len() && bytes[k].is_ascii_whitespace() {
        k += 1;
    }
    if k >= bytes.len() || bytes[k] != b'=' {
        return None;
    }
    // Reject `==` and `=~` (comparison/regex, not assignment).
    if k + 1 < bytes.len() && (bytes[k + 1] == b'=' || bytes[k + 1] == b'~') {
        return None;
    }
    let rhs = trimmed[k + 1..].trim().to_string();
    Some((name, idx_expr, rhs))
}

// END moved-from-exec-rs (free ported)

// ===========================================================
// Numeric formatting helpers moved from src/ported/vm_helper.
// Mirror Src/math.c / Src/utils.c base+digit-grouping logic.
// ===========================================================

// WARNING: NOT IN MATH.C — `convbase` lives in `Src/params.c:5632`
// (called from math.c:1089). This file holds a duplicate that
// predates the params.rs port; canonical home is
// `convbase`. This entry is drift pending
// cleanup; do not add new callers — use `convbase`.
/// Format an integer in the given base (2-36) using zsh's
// `convbase` lives in params.rs (matching C: defined in params.c:5632
// as a 1-line delegation to `convbase_ptr` at params.c:5586). The
// math.rs entry that used to duplicate it is removed; callers should
// import `convbase` directly.

#[cfg(test)]
mod tests {
    use super::*;

    /// `setmathvar` writes through to paramtab via `setnparam`.
    /// After the call, `getsparam(name)` should return the value.
    #[test]
    fn setmathvar_writes_to_paramtab() {
        let _g = crate::test_util::global_state_lock();
        // setnparam early-returns under `unset(EXECOPT)`. Enable it
        // (default in interactive shells; tests run with all opts
        // unset by default).
        opt_state_set("exec", true);
        // Sanity: a direct setiparam call should also work.
        unsetparam("mvar1_baseline");
        crate::ported::params::setiparam("mvar1_baseline", 42);
        let baseline = getsparam("mvar1_baseline");
        assert_eq!(
            baseline.as_deref(),
            Some("42"),
            "baseline setiparam path; got {:?}",
            baseline
        );
        unsetparam("mvar1_baseline");

        unsetparam("mvar1");
        let v = mnumber {
            l: 42,
            d: 0.0,
            type_: MN_INTEGER,
        };
        let returned = setmathvar("mvar1", v);
        assert_eq!(returned.l, 42);
        let stored = getsparam("mvar1");
        assert_eq!(
            stored.as_deref(),
            Some("42"),
            "setmathvar should write through; got {:?}",
            stored
        );
        unsetparam("mvar1");
        opt_state_set("exec", false);
    }

    /// `setmathvar` with empty name emits zerr and returns 0.
    #[test]
    fn setmathvar_empty_name_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let v = mnumber {
            l: 99,
            d: 0.0,
            type_: MN_INTEGER,
        };
        let returned = setmathvar("", v);
        assert_eq!(returned.l, 0);
        assert_eq!(returned.type_, MN_INTEGER);
    }

    /// End-to-end round trip: `setmathvar` writes through paramtab;
    /// `getmathparam` reads back the same value via `getsparam`. Pins
    /// the full math ↔ paramtab integration that the recent setmathvar
    /// and getsparam-PM_INTEGER fixes enable together.
    #[test]
    fn setmathvar_getmathparam_roundtrip() {
        let _g = crate::test_util::global_state_lock();
        opt_state_set("exec", true);
        unsetparam("rt_int");
        unsetparam("rt_float");

        // Integer roundtrip
        let n_in = mnumber {
            l: 123,
            d: 0.0,
            type_: MN_INTEGER,
        };
        setmathvar("rt_int", n_in);
        let n_out = getmathparam("rt_int");
        assert_eq!(n_out.type_, MN_INTEGER);
        assert_eq!(n_out.l, 123);

        // Float roundtrip
        let f_in = mnumber {
            l: 0,
            d: 3.14,
            type_: MN_FLOAT,
        };
        setmathvar("rt_float", f_in);
        let f_out = getmathparam("rt_float");
        // Stored as paramtab PM_FFLOAT (per setnparam c:3687); read
        // back as MN_FLOAT.
        assert_eq!(f_out.type_, MN_FLOAT);
        assert!(
            (f_out.d - 3.14).abs() < 1e-9,
            "expected ~3.14, got {}",
            f_out.d
        );

        unsetparam("rt_int");
        unsetparam("rt_float");
        opt_state_set("exec", false);
    }

    /// `setmathvar` with subscript splits at `[` and writes to the
    /// base name (subscript handling is upstream).
    #[test]
    fn setmathvar_subscript_writes_to_base_name() {
        let _g = crate::test_util::global_state_lock();
        opt_state_set("exec", true);
        unsetparam("mvar2");
        let v = mnumber {
            l: 7,
            d: 0.0,
            type_: MN_INTEGER,
        };
        setmathvar("mvar2[5]", v);
        let stored = getsparam("mvar2");
        // The base "mvar2" got the value; subscript element handling
        // is upstream so we just confirm the param was created.
        assert!(stored.is_some());
        unsetparam("mvar2");
        opt_state_set("exec", false);
    }

    /// `setmathvar` MUST short-circuit when `noeval` is set (c:1002-1003).
    /// Used by the unused branch of a math ternary: `(( cond ? a=1 : b=2 ))`
    /// evaluates only ONE side; the other side runs with noeval=1 to
    /// type-check without side effects. A regression that ignores
    /// noeval would assign BOTH sides, corrupting the unselected
    /// variable on every conditional expression.
    ///
    /// Pin: with noeval set, assigning to "ne_var" must NOT create
    /// the param. The return value still equals the input (so the
    /// arithmetic stack sees a sane value); paramtab stays unchanged.
    #[test]
    fn setmathvar_noeval_skips_paramtab_write() {
        let _g = crate::test_util::global_state_lock();
        opt_state_set("exec", true);
        unsetparam("ne_var");

        // Set the math-local noeval counter.
        M_NOEVAL.with(|n| n.set(1));

        let v = mnumber {
            l: 42,
            d: 0.0,
            type_: MN_INTEGER,
        };
        let ret = setmathvar("ne_var", v);
        assert_eq!(
            ret.l, 42,
            "c:1003 — `return v` so the stack still sees the value"
        );
        // The paramtab MUST NOT have a new entry.
        assert!(
            getsparam("ne_var").is_none(),
            "c:1002-1003 — noeval suppresses the paramtab write"
        );

        // Restore noeval so subsequent tests aren't affected.
        M_NOEVAL.with(|n| n.set(0));
        opt_state_set("exec", false);
    }

    /// `setmathvar` returns a value RE-TYPED to match the destination
    /// param's type (C c:1014-1027). Assigning a FLOAT to a PM_INTEGER
    /// param must return an integer-typed mnumber with the float
    /// truncated. This matches C's "assignment returns the typed
    /// value" contract — used by chained assignment expressions like
    /// `(( a = b = 3.7 ))` where `a`'s type drives the cascade.
    #[test]
    fn setmathvar_float_into_integer_coerces_return_type() {
        let _g = crate::test_util::global_state_lock();
        opt_state_set("exec", true);
        unsetparam("intvar");
        // Pre-create as PM_INTEGER so the type is fixed.
        crate::ported::params::setiparam("intvar", 0);

        // Assign a float; expect integer return with truncated value.
        let v = mnumber {
            l: 0,
            d: 3.7,
            type_: MN_FLOAT,
        };
        let ret = setmathvar("intvar", v);
        assert_eq!(
            ret.type_, MN_INTEGER,
            "c:1016-1020 — PM_INTEGER target must return MN_INTEGER"
        );
        assert_eq!(
            ret.l, 3,
            "c:1018 — float→int truncates (3.7 → 3, not rounded)"
        );

        unsetparam("intvar");
        opt_state_set("exec", false);
    }

    /// Pin `mathevalarg` empty-string error path per c:1530-1532.
    /// Unlike `matheval` which returns MN_INTEGER 0 on empty, this
    /// entry point treats empty as a HARD error (emits zerr and
    /// returns 0). Used by callers like `$array[$ind]` where unset
    /// `$ind` should produce a diagnostic rather than silently index 0.
    #[test]
    fn mathevalarg_empty_emits_error_and_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        // Empty input → returns 0 with error message emitted.
        let r = mathevalarg("");
        assert_eq!(r, 0, "c:1532 — empty input returns 0");

        // Nularg-only → also empty after skip, returns 0.
        let nularg_only: String = "\u{a1}".to_string();
        let r = mathevalarg(&nularg_only);
        assert_eq!(
            r, 0,
            "c:1528-1532 — Nularg-only is empty after skip, returns 0"
        );

        // Valid expression → real evaluation.
        let r = mathevalarg("1 + 2");
        assert_eq!(r, 3, "c:1534 — non-empty expression evaluates normally");

        // Nularg-prefixed expression → skipped, then evaluated.
        let nularg_plus: String = "\u{a1}5 * 5".to_string();
        let r = mathevalarg(&nularg_plus);
        assert_eq!(
            r, 25,
            "c:1529 — Nularg skipped, then `5 * 5` evaluates to 25"
        );
    }

    /// Pin `matheval` empty + Nularg fast paths per c:1489-1495.
    /// Empty input MUST return MN_INTEGER 0 without invoking the
    /// parser; the Nularg sentinel (0xa1) byte at the start of the
    /// input MUST be skipped before the empty check.
    #[test]
    fn matheval_empty_input_returns_zero_int() {
        let _g = crate::test_util::global_state_lock();
        // Empty string → MN_INTEGER 0 (c:1491-1494).
        let r = matheval("").expect("empty string must return 0, not error");
        assert_eq!(
            r.type_, MN_INTEGER,
            "c:1493 — empty input returns MN_INTEGER"
        );
        assert_eq!(r.l, 0, "c:1494 — empty input value is 0");

        // Nularg-only string → also returns 0 (c:1489-1494).
        let nularg_only: String = "\u{a1}".to_string();
        let r = matheval(&nularg_only)
            .expect("Nularg-only must return 0 (treated as empty after skip)");
        assert_eq!(
            r.type_, MN_INTEGER,
            "c:1489-1493 — Nularg-only input treated as empty → MN_INTEGER"
        );
        assert_eq!(r.l, 0, "c:1494 — Nularg-only input value is 0");

        // Nularg + expression → evaluates the expression (c:1490 skip).
        let nularg_plus: String = "\u{a1}1 + 2".to_string();
        let r =
            matheval(&nularg_plus).expect("Nularg prefix must be skipped and expression evaluated");
        let v = if r.type_ == MN_FLOAT { r.d as i64 } else { r.l };
        assert_eq!(v, 3, "c:1490 — Nularg skipped, then `1 + 2` evaluates to 3");
    }

    #[test]
    fn test_basic_arithmetic() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(mathevali("1 + 2").unwrap(), 3);
        assert_eq!(mathevali("10 - 3").unwrap(), 7);
        assert_eq!(mathevali("4 * 5").unwrap(), 20);
        assert_eq!(mathevali("20 / 4").unwrap(), 5);
        assert_eq!(mathevali("17 % 5").unwrap(), 2);
    }

    #[test]
    fn test_precedence() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(mathevali("2 + 3 * 4").unwrap(), 14);
        assert_eq!(mathevali("(2 + 3) * 4").unwrap(), 20);
        assert_eq!(mathevali("2 ** 3 ** 2").unwrap(), 512); // Right associative
    }

    #[test]
    fn test_comparison() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(mathevali("5 > 3").unwrap(), 1);
        assert_eq!(mathevali("5 < 3").unwrap(), 0);
        assert_eq!(mathevali("5 == 5").unwrap(), 1);
        assert_eq!(mathevali("5 != 3").unwrap(), 1);
        assert_eq!(mathevali("5 >= 5").unwrap(), 1);
        assert_eq!(mathevali("5 <= 5").unwrap(), 1);
    }

    #[test]
    fn test_logical() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(mathevali("1 && 1").unwrap(), 1);
        assert_eq!(mathevali("1 && 0").unwrap(), 0);
        assert_eq!(mathevali("1 || 0").unwrap(), 1);
        assert_eq!(mathevali("0 || 0").unwrap(), 0);
        assert_eq!(mathevali("!0").unwrap(), 1);
        assert_eq!(mathevali("!1").unwrap(), 0);
    }

    #[test]
    fn test_bitwise() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(mathevali("5 & 3").unwrap(), 1);
        assert_eq!(mathevali("5 | 3").unwrap(), 7);
        assert_eq!(mathevali("5 ^ 3").unwrap(), 6);
        assert_eq!(mathevali("~0").unwrap(), -1);
        assert_eq!(mathevali("1 << 4").unwrap(), 16);
        assert_eq!(mathevali("16 >> 2").unwrap(), 4);
    }

    #[test]
    fn test_ternary() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(mathevali("1 ? 10 : 20").unwrap(), 10);
        assert_eq!(mathevali("0 ? 10 : 20").unwrap(), 20);
        assert_eq!(mathevali("(5 > 3) ? 100 : 200").unwrap(), 100);
    }

    #[test]
    fn test_power() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(mathevali("2 ** 10").unwrap(), 1024);
        assert_eq!(mathevali("3 ** 3").unwrap(), 27);
        assert!(
            (matheval("2.0 ** 0.5")
                .map(|n| (if n.type_ == MN_FLOAT { n.d } else { n.l as f64 }))
                .unwrap()
                - std::f64::consts::SQRT_2)
                .abs()
                < 0.0001
        );
    }

    #[test]
    fn test_float() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            (matheval("3.14 + 0.01")
                .map(|n| (if n.type_ == MN_FLOAT { n.d } else { n.l as f64 }))
                .unwrap()
                - 3.15)
                .abs()
                < 0.0001
        );
        assert!(
            (matheval("1.5 * 2.0")
                .map(|n| (if n.type_ == MN_FLOAT { n.d } else { n.l as f64 }))
                .unwrap()
                - 3.0)
                .abs()
                < 0.0001
        );
    }

    #[test]
    fn test_unary() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(mathevali("-5").unwrap(), -5);
        assert_eq!(mathevali("- -5").unwrap(), 5); // space needed to avoid --
        assert_eq!(mathevali("+5").unwrap(), 5);
        assert_eq!(mathevali("-(-5)").unwrap(), 5);
    }

    #[test]
    fn test_base() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(mathevali("0xFF").unwrap(), 255);
        assert_eq!(mathevali("0b1010").unwrap(), 10);
        assert_eq!(mathevali("16#FF").unwrap(), 255);
        assert_eq!(mathevali("2#1010").unwrap(), 10);
        assert_eq!(mathevali("[16]FF").unwrap(), 255);
    }

    #[test]
    fn test_variables() {
        let _g = crate::test_util::global_state_lock();
        let mut vars = HashMap::new();
        vars.insert(
            "x".to_string(),
            mnumber {
                l: 10,
                d: 0.0,
                type_: MN_INTEGER,
            },
        );
        vars.insert(
            "y".to_string(),
            mnumber {
                l: 20,
                d: 0.0,
                type_: MN_INTEGER,
            },
        );

        new("x + y");
        with_variables(vars);
        assert_eq!(
            ({
                let __m = mathevall().unwrap();
                if __m.type_ == MN_FLOAT {
                    __m.d as i64
                } else {
                    __m.l
                }
            }),
            30
        );
    }

    #[test]
    fn test_assignment() {
        let _g = crate::test_util::global_state_lock();
        new("x = 5");
        mathevall().unwrap();
        assert_eq!(
            ({
                let __m = m_variables_get("x").unwrap();
                if __m.type_ == MN_FLOAT {
                    __m.d as i64
                } else {
                    __m.l
                }
            }),
            5
        );

        new("x = 5, x += 3");
        let result = mathevall().unwrap();
        assert_eq!(
            (if result.type_ == MN_FLOAT {
                result.d as i64
            } else {
                result.l
            }),
            8
        );
    }

    #[test]
    fn test_increment() {
        let _g = crate::test_util::global_state_lock();
        let mut vars = HashMap::new();
        vars.insert(
            "x".to_string(),
            mnumber {
                l: 5,
                d: 0.0,
                type_: MN_INTEGER,
            },
        );

        new("++x");
        with_variables(vars.clone());
        assert_eq!(
            ({
                let __m = mathevall().unwrap();
                if __m.type_ == MN_FLOAT {
                    __m.d as i64
                } else {
                    __m.l
                }
            }),
            6
        );
        assert_eq!(
            ({
                let __m = m_variables_get("x").unwrap();
                if __m.type_ == MN_FLOAT {
                    __m.d as i64
                } else {
                    __m.l
                }
            }),
            6
        );

        new("x++");
        with_variables(vars.clone());
        assert_eq!(
            ({
                let __m = mathevall().unwrap();
                if __m.type_ == MN_FLOAT {
                    __m.d as i64
                } else {
                    __m.l
                }
            }),
            5
        );
        assert_eq!(
            ({
                let __m = m_variables_get("x").unwrap();
                if __m.type_ == MN_FLOAT {
                    __m.d as i64
                } else {
                    __m.l
                }
            }),
            6
        );
    }

    #[test]
    fn test_functions() {
        let _g = crate::test_util::global_state_lock();
        // c:Src/math.c:1037 — `callmathfunc` requires `zsh/mathfunc`
        // to be in the loaded-modules table. zsh -fc returns
        // "unknown function: sqrt" without `zmodload zsh/mathfunc`
        // — the same gating now applies in zshrs. Boot the module
        // here so the unit test exercises the math-function bodies
        // not the missing-module guard.
        crate::ported::module::MODULESTAB
            .lock()
            .unwrap()
            .load_module("zsh/mathfunc", None, false);
        assert!(
            (matheval("sqrt(4)")
                .map(|n| (if n.type_ == MN_FLOAT { n.d } else { n.l as f64 }))
                .unwrap()
                - 2.0)
                .abs()
                < 0.0001
        );
        assert!(
            (matheval("sin(0)")
                .map(|n| (if n.type_ == MN_FLOAT { n.d } else { n.l as f64 }))
                .unwrap())
            .abs()
                < 0.0001
        );
        assert!(
            (matheval("cos(0)")
                .map(|n| (if n.type_ == MN_FLOAT { n.d } else { n.l as f64 }))
                .unwrap()
                - 1.0)
                .abs()
                < 0.0001
        );
        assert!(
            (matheval("abs(-5)")
                .map(|n| (if n.type_ == MN_FLOAT { n.d } else { n.l as f64 }))
                .unwrap()
                - 5.0)
                .abs()
                < 0.0001
        );
        assert!(
            (matheval("floor(3.7)")
                .map(|n| (if n.type_ == MN_FLOAT { n.d } else { n.l as f64 }))
                .unwrap()
                - 3.0)
                .abs()
                < 0.0001
        );
        assert!(
            (matheval("ceil(3.2)")
                .map(|n| (if n.type_ == MN_FLOAT { n.d } else { n.l as f64 }))
                .unwrap()
                - 4.0)
                .abs()
                < 0.0001
        );
    }

    #[test]
    fn test_special_values() {
        let _g = crate::test_util::global_state_lock();
        assert!(matheval("Inf")
            .map(|n| (if n.type_ == MN_FLOAT { n.d } else { n.l as f64 }))
            .unwrap()
            .is_infinite());
        assert!(matheval("NaN")
            .map(|n| (if n.type_ == MN_FLOAT { n.d } else { n.l as f64 }))
            .unwrap()
            .is_nan());
    }

    #[test]
    fn test_errors() {
        let _g = crate::test_util::global_state_lock();
        assert!(matheval("1 / 0").is_err());
        assert!(matheval("1 +").is_err());
        // Empty arith expression is a parse error in zsh:
        //   $ zsh -c '(( ))'; echo $?   →   1
        // zsh aborts with `bad math expression: empty parentheses`.
        assert!(matheval("()").is_err());
    }

    #[test]
    fn test_underscore_in_numbers() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(mathevali("1_000_000").unwrap(), 1000000);
        assert_eq!(mathevali("0xFF_FF").unwrap(), 65535);
    }

    #[test]
    fn test_comma_operator() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(mathevali("1, 2, 3").unwrap(), 3);
        assert_eq!(mathevali("(x = 1, y = 2, x + y)").unwrap(), 3);
    }

    /// c:1505 — integer divide-by-zero is a runtime error in `$(( ))`.
    /// A regression returning 0 silently masks programmer errors.
    #[test]
    fn mathevali_divide_by_zero_errors() {
        let _g = crate::test_util::global_state_lock();
        assert!(mathevali("1/0").is_err());
        assert!(mathevali("5/(2-2)").is_err());
    }

    /// Bug #1025: assigning to a non-lvalue (`1 = 2`) must fail with the FULL
    /// "bad math expression: lvalue required" (c:Src/math.c:997), not the
    /// prefix-stripped "lvalue required" the assignment-operator arm emitted.
    #[test]
    fn mathevali_assign_to_nonlvalue_keeps_bad_math_prefix() {
        let _g = crate::test_util::global_state_lock();
        for expr in ["1 = 2", "5 = 3 + 2", "(1+1) = 3"] {
            let err = mathevali(expr).expect_err("assign to non-lvalue must error");
            assert_eq!(
                err, "bad math expression: lvalue required",
                "expr {expr:?} must carry the `bad math expression:` prefix"
            );
        }
        // A real lvalue still assigns.
        assert_eq!(mathevali("x = 5").unwrap(), 5);
    }

    /// c:1505-1508 — `mathevali` returns `(x.type & MN_FLOAT) ?
    /// (zlong)x.u.d : x.u.l`. A float result rounded toward zero;
    /// MN_INTEGER returns x.u.l unchanged. Regression target: the
    /// previous Rust port used strict equality `== MN_FLOAT` which
    /// misclassifies any composite MN_FLOAT|MN_X bitfield.
    #[test]
    fn mathevali_truncates_float_via_bitmask_not_strict_eq() {
        let _g = crate::test_util::global_state_lock();
        // Float expression → truncated toward zero (3.7 → 3, -3.7 → -3).
        assert_eq!(mathevali("3.7").unwrap(), 3);
        assert_eq!(mathevali("-3.7").unwrap(), -3);
        // Pure integer expression → MN_INTEGER path, no truncation.
        assert_eq!(mathevali("42").unwrap(), 42);
    }

    /// c:1480 — mod-by-zero is also an error (matches POSIX).
    #[test]
    fn mathevali_mod_by_zero_errors() {
        let _g = crate::test_util::global_state_lock();
        assert!(mathevali("5 % 0").is_err());
    }

    /// c:1505 — operator precedence: `*` binds tighter than `+`.
    /// Regression flipping this would silently break every
    /// `$(( a + b * c ))` users compute.
    #[test]
    fn mathevali_respects_multiplicative_over_additive_precedence() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(mathevali("1 + 2 * 3").unwrap(), 7);
        assert_eq!(mathevali("(1 + 2) * 3").unwrap(), 9);
        assert_eq!(mathevali("10 - 2 * 3").unwrap(), 4);
    }

    /// c:1505 — bitshift `<<` `>>` from `$(( ))` grammar. Regression
    /// dropping them breaks any hex-mask / bit-pack computation.
    #[test]
    fn mathevali_bitshift_operators() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(mathevali("1 << 4").unwrap(), 16);
        assert_eq!(mathevali("256 >> 3").unwrap(), 32);
    }

    /// c:1505 — `&&` short-circuits on zero LHS. Regression that
    /// evaluates the RHS would surface side-effects (or divide-
    /// by-zero) the user expected NOT to fire.
    #[test]
    fn mathevali_logical_and_short_circuits_on_zero_lhs() {
        let _g = crate::test_util::global_state_lock();
        // If RHS evaluated, `1/0` would error. Short-circuit must skip.
        assert_eq!(mathevali("0 && 1/0").unwrap(), 0);
    }

    /// c:1505 — `||` short-circuits on non-zero LHS. Same rationale.
    #[test]
    fn mathevali_logical_or_short_circuits_on_nonzero_lhs() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(mathevali("1 || 1/0").unwrap(), 1);
    }

    /// c:1505 — ternary `cond ? a : b` evaluates ONLY the selected
    /// branch. Regression that evaluates both surfaces side-effects
    /// in the unused branch.
    #[test]
    fn mathevali_ternary_evaluates_only_selected_branch() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(mathevali("1 ? 42 : 1/0").unwrap(), 42);
        assert_eq!(mathevali("0 ? 1/0 : 42").unwrap(), 42);
    }

    /// `Src/math.c:467-490` — `$(( ))` integer arithmetic supports
    /// hex (`0x`/`0X`), binary (`0b`/`0B`). Octal-via-leading-zero
    /// (`0777`) is OPT-IN behind the OCTALZEROES option — by default
    /// `0777` parses as decimal 777 (matches C's c:489 conditional).
    /// Pin both the hex/binary path AND the default-decimal behavior
    /// for leading-zero.
    #[test]
    fn mathevali_parses_hex_and_binary_literals() {
        let _g = crate::test_util::global_state_lock();
        // octalzeroes is reset to OFF by global_state_lock (test_util.rs)
        // so the `0777`-as-decimal pin works regardless of which test
        // ran first.
        // Hex literals at c:471 (lowchar 'x').
        assert_eq!(mathevali("0xff").unwrap(), 255);
        assert_eq!(mathevali("0x10").unwrap(), 16);
        assert_eq!(mathevali("0xff + 1").unwrap(), 256);
        // Binary literals at c:471 (lowchar 'b').
        assert_eq!(mathevali("0b1010").unwrap(), 10);
        assert_eq!(mathevali("0b11111111").unwrap(), 255);
        // Default-OCTALZEROES-off: `0777` is decimal 777, NOT octal 511.
        assert_eq!(
            mathevali("0777").unwrap(),
            777,
            "c:489 — leading-zero parses as decimal when OCTALZEROES off"
        );
    }

    /// c:1505 — bitwise AND/OR/XOR. Each operator has its own
    /// precedence tier between shifts and logical ops. Regression
    /// flipping precedence between `&` and `|` would break `$(( a &
    /// b | c ))` (must be `(a&b) | c`, not `a & (b|c)`).
    #[test]
    fn mathevali_bitwise_operators_and_or_xor() {
        let _g = crate::test_util::global_state_lock();
        // Boolean truth-table cases.
        assert_eq!(mathevali("12 & 10").unwrap(), 8, "1100 & 1010 = 1000");
        assert_eq!(mathevali("12 | 10").unwrap(), 14, "1100 | 1010 = 1110");
        assert_eq!(mathevali("12 ^ 10").unwrap(), 6, "1100 ^ 1010 = 0110");
        // Precedence: `&` > `^` > `|`, so `a & b | c` == `(a&b) | c`.
        assert_eq!(
            mathevali("12 & 10 | 1").unwrap(),
            9,
            "c:1505 — & binds tighter than | : (12 & 10) | 1 = 8 | 1 = 9"
        );
    }

    /// c:1505 — comparison ops produce 0 or 1 (Boolean semantics).
    /// Pin all six relational ops.
    #[test]
    fn mathevali_comparison_operators_return_zero_or_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(mathevali("1 < 2").unwrap(), 1);
        assert_eq!(mathevali("2 < 1").unwrap(), 0);
        assert_eq!(mathevali("2 > 1").unwrap(), 1);
        assert_eq!(mathevali("1 > 2").unwrap(), 0);
        assert_eq!(mathevali("2 <= 2").unwrap(), 1);
        assert_eq!(mathevali("2 >= 2").unwrap(), 1);
        assert_eq!(mathevali("2 == 2").unwrap(), 1);
        assert_eq!(mathevali("2 != 2").unwrap(), 0);
    }

    /// c:1505 — unary minus and bitwise NOT. The C parser must
    /// distinguish `1 - 2` (binary) from `1 + -2` (unary).
    #[test]
    fn mathevali_unary_minus_and_bitwise_not() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(mathevali("-5").unwrap(), -5);
        assert_eq!(mathevali("-(2 + 3)").unwrap(), -5);
        assert_eq!(mathevali("1 + -2").unwrap(), -1);
        // Bitwise NOT (~).
        assert_eq!(mathevali("~0").unwrap(), -1, "two's-complement: ~0 = -1");
        assert_eq!(mathevali("~5").unwrap(), -6);
    }

    /// c:1505 — logical NOT operator `!`. Maps 0 → 1, anything-else → 0.
    #[test]
    fn mathevali_logical_not() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(mathevali("!0").unwrap(), 1);
        assert_eq!(mathevali("!1").unwrap(), 0);
        assert_eq!(mathevali("!42").unwrap(), 0);
        // Double-NOT normalises to 0/1.
        assert_eq!(mathevali("!!42").unwrap(), 1);
        assert_eq!(mathevali("!!0").unwrap(), 0);
    }

    /// `Src/math.c:109-161` — math token IDs are `#define`d as a
    /// densely-packed integer ladder used as indices into the precedence
    /// (`Z_PREC` / `C_PREC`) and type (`OP_TYPE`) tables. Position is
    /// load-bearing: shifting any value silently mis-routes every math
    /// expression at runtime. Pin every value individually so a reorder
    /// or off-by-one fails this test. QUEST=27 and COMMA=43 specifically
    /// were previously typed in Title-case (`Quest`/`Comma`) violating
    /// the C-source casing rule.
    #[test]
    fn math_token_ids_match_c_source_position_for_position() {
        let _g = crate::test_util::global_state_lock();
        let table = [
            ("M_INPAR", M_INPAR, 0),
            ("M_OUTPAR", M_OUTPAR, 1),
            ("NOT", NOT, 2),
            ("COMP", COMP, 3),
            ("POSTPLUS", POSTPLUS, 4),
            ("POSTMINUS", POSTMINUS, 5),
            ("UPLUS", UPLUS, 6),
            ("UMINUS", UMINUS, 7),
            ("AND", AND, 8),
            ("XOR", XOR, 9),
            ("OR", OR, 10),
            ("MUL", MUL, 11),
            ("DIV", DIV, 12),
            ("MOD", MOD, 13),
            ("PLUS", PLUS, 14),
            ("MINUS", MINUS, 15),
            ("SHLEFT", SHLEFT, 16),
            ("SHRIGHT", SHRIGHT, 17),
            ("LES", LES, 18),
            ("LEQ", LEQ, 19),
            ("GRE", GRE, 20),
            ("GEQ", GEQ, 21),
            ("DEQ", DEQ, 22),
            ("NEQ", NEQ, 23),
            ("DAND", DAND, 24),
            ("DOR", DOR, 25),
            ("DXOR", DXOR, 26),
            ("QUEST", QUEST, 27), // c:136 — was Title-case Quest, divergent
            ("COLON", COLON, 28),
            ("EQ", EQ, 29),
            ("PLUSEQ", PLUSEQ, 30),
            ("MINUSEQ", MINUSEQ, 31),
            ("MULEQ", MULEQ, 32),
            ("DIVEQ", DIVEQ, 33),
            ("MODEQ", MODEQ, 34),
            ("ANDEQ", ANDEQ, 35),
            ("XOREQ", XOREQ, 36),
            ("OREQ", OREQ, 37),
            ("SHLEFTEQ", SHLEFTEQ, 38),
            ("SHRIGHTEQ", SHRIGHTEQ, 39),
            ("DANDEQ", DANDEQ, 40),
            ("DOREQ", DOREQ, 41),
            ("DXOREQ", DXOREQ, 42),
            ("COMMA", COMMA, 43), // c:152 — was Title-case Comma, divergent
            ("EOI", EOI, 44),
            ("PREPLUS", PREPLUS, 45),
            ("PREMINUS", PREMINUS, 46),
            ("NUM", NUM, 47),
            ("ID", ID, 48),
            ("POWER", POWER, 49),
            ("CID", CID, 50),
            ("POWEREQ", POWEREQ, 51),
            ("FUNC", FUNC, 52),
        ];
        for (name, got, want) in table {
            assert_eq!(
                got, want,
                "c:109-161 — {} must be {} (C source value)",
                name, want
            );
        }
        // TOKCOUNT = 53 must equal the table length (no holes).
        assert_eq!(
            TOKCOUNT,
            table.len() + 0,
            "c:162 — TOKCOUNT must match the number of tokens"
        );
        // QUEST sits between DXOR and COLON; gap was 26→28 BEFORE the
        // QUEST=27 fix, exposing a missing index. Pin the ordering.
        assert_eq!(QUEST, DXOR + 1, "c:136 — QUEST immediately follows DXOR");
        assert_eq!(COLON, QUEST + 1, "c:137 — COLON immediately follows QUEST");
        assert_eq!(
            COMMA,
            DXOREQ + 1,
            "c:152 — COMMA immediately follows DXOREQ"
        );
        assert_eq!(EOI, COMMA + 1, "c:153 — EOI immediately follows COMMA");
    }

    /// `Src/math.c:109-162` — the precedence and type tables MUST have
    /// length `TOKCOUNT`. Pin both lengths so a future token addition
    /// without table updates fails immediately.
    #[test]
    fn math_dispatch_tables_match_tokcount() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            Z_PREC.len(),
            TOKCOUNT,
            "Z_PREC must have one slot per math token"
        );
        assert_eq!(
            C_PREC.len(),
            TOKCOUNT,
            "C_PREC must have one slot per math token"
        );
        assert_eq!(
            OP_TYPE.len(),
            TOKCOUNT,
            "OP_TYPE must have one slot per math token"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // matheval / mathevali — anchored to `zsh -c 'echo $(( ... ))'`.
    // Each expected value verified against zsh 5.9. Where zshrs diverges
    // the test FAILS, exposing the bug. matheval returns mnumber; we
    // mostly use mathevali for integer comparisons.
    // ═══════════════════════════════════════════════════════════════════

    fn mi(expr: &str) -> i64 {
        let _g = crate::test_util::global_state_lock();
        mathevali(expr).unwrap_or_else(|e| panic!("mathevali({expr:?}) → Err({e})"))
    }

    // ── Literals ────────────────────────────────────────────────────
    /// `echo $(( 42 ))` → 42
    #[test]
    fn matheval_decimal_literal() {
        assert_eq!(mi("42"), 42);
    }

    /// `echo $(( -7 ))` → -7
    #[test]
    fn matheval_unary_minus_literal() {
        assert_eq!(mi("-7"), -7);
    }

    /// `echo $(( 0xff ))` → 255
    #[test]
    fn matheval_hex_literal_lowercase() {
        assert_eq!(mi("0xff"), 255);
    }

    /// `echo $(( 0XDEAD ))` → 57005
    #[test]
    fn matheval_hex_literal_uppercase() {
        assert_eq!(mi("0XDEAD"), 0xDEAD);
    }

    /// `echo $(( 16#FF ))` → 255 (zsh base# literal)
    #[test]
    fn matheval_base_hash_hex() {
        assert_eq!(mi("16#FF"), 255);
    }

    /// `echo $(( 2#1010 ))` → 10
    #[test]
    fn matheval_base_hash_binary() {
        assert_eq!(mi("2#1010"), 10);
    }

    /// `echo $(( 8#17 ))` → 15
    #[test]
    fn matheval_base_hash_octal() {
        assert_eq!(mi("8#17"), 15);
    }

    /// `echo $(( 010 ))` → 10 (zsh default: NOT octal unless OCTAL_ZEROES set)
    /// Test relies on `test_util::global_state_lock()` (acquired inside `mi`)
    /// to reset `octalzeroes` to OFF on entry — see test_util.rs:53.
    #[test]
    fn matheval_leading_zero_is_decimal_not_octal() {
        assert_eq!(mi("010"), 10);
    }

    // ── Binary arithmetic ──────────────────────────────────────────
    /// `echo $(( 1 + 2 + 3 ))` → 6
    #[test]
    fn matheval_addition_chain() {
        assert_eq!(mi("1 + 2 + 3"), 6);
    }

    /// `echo $(( 2 * 3 + 4 ))` → 10 (precedence: * over +)
    #[test]
    fn matheval_precedence_mul_over_add() {
        assert_eq!(mi("2 * 3 + 4"), 10);
    }

    /// `echo $(( 2 * (3 + 4) ))` → 14 (parens override)
    #[test]
    fn matheval_parens_override_precedence() {
        assert_eq!(mi("2 * (3 + 4)"), 14);
    }

    /// `echo $(( 17 / 5 ))` → 3 (integer division, truncates toward 0)
    #[test]
    fn matheval_integer_division_truncates() {
        assert_eq!(mi("17 / 5"), 3);
    }

    /// `echo $(( 1 / 4 ))` → 0 (integer division of small numerator)
    #[test]
    fn matheval_integer_division_below_one() {
        assert_eq!(mi("1 / 4"), 0);
    }

    /// `echo $(( 17 % 5 ))` → 2
    #[test]
    fn matheval_modulo() {
        assert_eq!(mi("17 % 5"), 2);
    }

    /// `echo $(( 2 ** 10 ))` → 1024
    #[test]
    fn matheval_power() {
        assert_eq!(mi("2 ** 10"), 1024);
    }

    /// `echo $(( 3 ** 3 ))` → 27
    #[test]
    fn matheval_power_small_cubed() {
        assert_eq!(mi("3 ** 3"), 27);
    }

    /// `echo $(( -2 ** 2 ))` → 4 (zsh: unary binds tighter than **)
    #[test]
    fn matheval_unary_binds_tighter_than_power() {
        assert_eq!(mi("-2 ** 2"), 4);
    }

    // ── Bitwise ─────────────────────────────────────────────────────
    /// `echo $(( 0xff & 0x0f ))` → 15
    #[test]
    fn matheval_bitand() {
        assert_eq!(mi("0xff & 0x0f"), 15);
    }

    /// `echo $(( 0xff | 0x100 ))` → 511
    #[test]
    fn matheval_bitor() {
        assert_eq!(mi("0xff | 0x100"), 511);
    }

    /// `echo $(( 0xff ^ 0x0f ))` → 240
    #[test]
    fn matheval_bitxor() {
        assert_eq!(mi("0xff ^ 0x0f"), 240);
    }

    /// `echo $(( ~0 ))` → -1 (two's-complement bitwise NOT)
    #[test]
    fn matheval_bitnot_zero_is_minus_one() {
        assert_eq!(mi("~0"), -1);
    }

    /// `echo $(( 1 << 8 ))` → 256
    #[test]
    fn matheval_left_shift() {
        assert_eq!(mi("1 << 8"), 256);
    }

    /// `echo $(( 256 >> 4 ))` → 16
    #[test]
    fn matheval_right_shift() {
        assert_eq!(mi("256 >> 4"), 16);
    }

    /// `echo $(( -1 >> 1 ))` → -1 (arithmetic shift, sign-preserving)
    #[test]
    fn matheval_arithmetic_right_shift_preserves_sign() {
        assert_eq!(mi("-1 >> 1"), -1);
    }

    // ── Comparison & logical ───────────────────────────────────────
    /// `echo $(( 5 == 5 ))` → 1
    #[test]
    fn matheval_eq_true() {
        assert_eq!(mi("5 == 5"), 1);
    }

    /// `echo $(( 5 != 6 ))` → 1
    #[test]
    fn matheval_ne_true() {
        assert_eq!(mi("5 != 6"), 1);
    }

    /// `echo $(( 3 < 5 ))` → 1
    #[test]
    fn matheval_lt_true() {
        assert_eq!(mi("3 < 5"), 1);
    }

    /// `echo $(( 5 <= 5 ))` → 1
    #[test]
    fn matheval_le_true_on_equal() {
        assert_eq!(mi("5 <= 5"), 1);
    }

    /// `echo $(( 1 && 1 ))` → 1
    #[test]
    fn matheval_logand_both_true() {
        assert_eq!(mi("1 && 1"), 1);
    }

    /// `echo $(( 0 || 1 ))` → 1
    #[test]
    fn matheval_logor_one_true() {
        assert_eq!(mi("0 || 1"), 1);
    }

    /// `echo $(( !0 ))` → 1
    #[test]
    fn matheval_lognot_false() {
        assert_eq!(mi("!0"), 1);
    }

    /// `echo $(( !5 ))` → 0
    #[test]
    fn matheval_lognot_truthy() {
        assert_eq!(mi("!5"), 0);
    }

    // ── Ternary ─────────────────────────────────────────────────────
    /// `echo $(( 1 ? 10 : 20 ))` → 10
    #[test]
    fn matheval_ternary_true_branch() {
        assert_eq!(mi("1 ? 10 : 20"), 10);
    }

    /// `echo $(( 0 ? 10 : 20 ))` → 20
    #[test]
    fn matheval_ternary_false_branch() {
        assert_eq!(mi("0 ? 10 : 20"), 20);
    }

    // ── Comma operator ─────────────────────────────────────────────
    /// `echo $(( (1,2,3) ))` → 3 (comma returns last)
    #[test]
    fn matheval_comma_returns_last() {
        assert_eq!(mi("(1,2,3)"), 3);
    }

    // ── Floats via matheval (mnumber tag) ──────────────────────────
    /// `1.0 / 4` returns a float (MN_FLOAT) — pin the type discriminator.
    #[test]
    fn matheval_float_div_returns_mn_float_type() {
        let _g = crate::test_util::global_state_lock();
        let n = matheval("1.0 / 4").expect("matheval");
        // MN_FLOAT flag must be set on the result type.
        assert_ne!(
            n.type_ & MN_FLOAT,
            0,
            "1.0 / 4 must carry MN_FLOAT in type; got type_=0x{:x}",
            n.type_
        );
        // d field holds the float value; should be ~0.25.
        assert!(
            (n.d - 0.25).abs() < 1e-9,
            "1.0 / 4 d-field should be 0.25; got {}",
            n.d
        );
    }

    /// `42` returns an integer (MN_INTEGER, not MN_FLOAT).
    #[test]
    fn matheval_integer_literal_returns_mn_integer_type() {
        let _g = crate::test_util::global_state_lock();
        let n = matheval("42").expect("matheval");
        assert_eq!(
            n.type_ & MN_FLOAT,
            0,
            "42 must NOT carry MN_FLOAT; got type_=0x{:x}",
            n.type_
        );
        assert_eq!(n.l, 42);
    }

    /// matheval on empty string → MN_INTEGER 0 (C c:1491-1495 fast path).
    #[test]
    fn matheval_empty_input_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let n = matheval("").expect("matheval(\"\") must succeed");
        assert_eq!(n.l, 0, "empty input → 0");
        assert_eq!(
            n.type_, MN_INTEGER,
            "empty input → MN_INTEGER (c:1491-1495)"
        );
    }

    // ── mathevali (integer-coerce front-end) ───────────────────────
    /// `mathevali` integer-coerces float results via `(zlong)x.u.d` — pin
    /// the truncation semantics (away from zero is wrong; C truncates).
    #[test]
    fn mathevali_truncates_float_toward_zero() {
        let _g = crate::test_util::global_state_lock();
        // 7.9 → truncates to 7 (NOT rounds to 8)
        assert_eq!(mathevali("7.9").unwrap(), 7);
        // -7.9 → truncates to -7 (NOT rounds to -8)
        assert_eq!(mathevali("-7.9").unwrap(), -7);
    }

    // ═══════════════════════════════════════════════════════════════════
    // zsh test-corpus pins — Test/C01arith.ztst arithmetic regression
    // suite. Each test cites the ztst line range; pass = lock current
    // correct behavior, #[ignore = "ZSHRS BUG: ..."] = tracked gap.
    // ═══════════════════════════════════════════════════════════════════

    /// `Test/C01arith.ztst:6-10` — basic integer literal.
    #[test]
    fn zsh_corpus_basic_integer_literal() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(matheval("42").unwrap().l, 42, "ztst:9 — int literal");
    }

    /// `Test/C01arith.ztst:22-25` — `((29.1 % 13.0 * 10) + 0.5)` = 31.6
    /// → integer-coerced to 31.
    #[test]
    fn zsh_corpus_float_modulo_then_int_truncation() {
        let _g = crate::test_util::global_state_lock();
        let r = mathevali("(29.1 % 13.0 * 10) + 0.5");
        assert_eq!(r.ok(), Some(31), "ztst:25 — float % then int truncation");
    }

    /// `Test/C01arith.ztst:27-29` — multi-base input:
    /// `0x10 + 0X01 + 2#1010` = 16 + 1 + 10 = 27.
    #[test]
    fn zsh_corpus_multi_base_input() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            mathevali("0x10 + 0X01 + 2#1010").unwrap(),
            27,
            "ztst:29 — hex + 2#binary sum",
        );
    }

    /// `Test/C01arith.ztst:41-44` — float→int truncation:
    /// `(( i = 32.5 ))` then int → 32.
    #[test]
    fn zsh_corpus_float_truncates_in_integer_context() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            mathevali("32.5").unwrap(),
            32,
            "ztst:44 — truncate, not round"
        );
    }

    /// `Test/C01arith.ztst:46-50` — operator precedence chain:
    /// `4 - - 3 * 7 << 1 & 7 ^ 1 | 16 ** 2` = 1591 (zsh-default
    /// MATH_OPS precedence, NOT C precedence).
    #[test]
    fn zsh_corpus_zsh_precedence_chain() {
        let _g = crate::test_util::global_state_lock();
        let r = mathevali("4 - - 3 * 7 << 1 & 7 ^ 1 | 16 ** 2");
        assert_eq!(r.ok(), Some(1591), "ztst:50 — zsh-default precedence");
    }

    /// `Test/C01arith.ztst:96-97` — mixed int+float:
    /// `3 + 5 * 1.75` = 11.75 (float promotion).
    #[test]
    fn zsh_corpus_mixed_int_float_promotes_to_float() {
        let _g = crate::test_util::global_state_lock();
        let n = matheval("3 + 5 * 1.75").unwrap();
        assert!(
            (n.d - 11.75).abs() < 1e-9,
            "ztst:96 — 3+5*1.75 = 11.75 (float promotion)"
        );
    }

    /// `Test/C01arith.ztst:62-64` — logical precedence:
    /// `1 < 2 || 2 < 2 && 3 > 4` = 1 (|| lower than &&).
    #[test]
    fn zsh_corpus_logical_precedence_or_low_and_high() {
        let _g = crate::test_util::global_state_lock();
        let n = mathevali("1 < 2 || 2 < 2 && 3 > 4").unwrap();
        assert_eq!(n, 1, "ztst:64 — || lower than &&");
    }

    /// `Test/C01arith.ztst:66-68` — nested ternary right-associative:
    /// `1+4 ? 3+2 ? 4+3 ? 5+6 ? 4*8 : 0 : 0 : 0 : 0` = 32.
    #[test]
    fn zsh_corpus_ternary_right_associative_nested() {
        let _g = crate::test_util::global_state_lock();
        let n = mathevali("1+4 ? 3+2 ? 4+3 ? 5+6 ? 4*8 : 0 : 0 : 0 : 0").unwrap();
        assert_eq!(n, 32, "ztst:68 — nested ternary right-associative");
    }

    /// `Test/C01arith.ztst:78-80` — comma returns last:
    /// `0, 4 ? 3 : 1, 5` = 5.
    #[test]
    fn zsh_corpus_comma_returns_last_value() {
        let _g = crate::test_util::global_state_lock();
        let n = mathevali("0, 4 ? 3 : 1, 5").unwrap();
        assert_eq!(n, 5, "ztst:80 — comma operator returns last");
    }

    /// `Test/C01arith.ztst:9` — `1 + 2 * 3` = 7 (precedence).
    #[test]
    fn zsh_corpus_integer_precedence_mul_before_add() {
        let _g = crate::test_util::global_state_lock();
        let n = mathevali("1 + 2 * 3").unwrap();
        assert_eq!(n, 7, "ztst:9 — *,/ before +,-");
    }

    /// `Test/C01arith.ztst:18` — `1.5 + 2.5` = 4.0.
    #[test]
    fn zsh_corpus_basic_float_add() {
        let _g = crate::test_util::global_state_lock();
        let n = matheval("1.5 + 2.5").unwrap();
        assert!((n.d - 4.0).abs() < 1e-9, "ztst:18 — 1.5+2.5=4.0");
    }

    /// `Test/C01arith.ztst:24-26` — `7.5 % 2.5` = 0.0 (float modulo).
    #[test]
    fn zsh_corpus_float_modulo_exact_division() {
        let _g = crate::test_util::global_state_lock();
        let n = matheval("7.5 % 2.5").unwrap();
        assert!(n.d.abs() < 1e-9, "ztst:24 — 7.5%2.5=0.0");
    }

    /// `Test/C01arith.ztst:46-50` — full zsh precedence chain.
    /// `4 - - 3 * 7 << 1 & 7 ^ 1 | 16 ** 2` = 1591 under default
    /// (non-C-precedence) zsh.
    #[test]
    fn zsh_corpus_full_zsh_precedence_chain() {
        let _g = crate::test_util::global_state_lock();
        let n = mathevali("4 - - 3 * 7 << 1 & 7 ^ 1 | 16 ** 2").unwrap();
        assert_eq!(n, 1591, "ztst:50 — default zsh precedence = 1591");
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests pinning Src/math.c output-format + lastbase
    // accessor helpers.
    // ═══════════════════════════════════════════════════════════════════

    /// `outputradix()` returns the current `$OUTPUT_RADIX` value.
    /// C: reads the global `outputradix` int.
    #[test]
    fn outputradix_returns_int_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _r = outputradix();
    }

    /// `outputunderscore()` returns the current digit-group setting.
    #[test]
    fn outputunderscore_returns_int_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _r = outputunderscore();
    }

    /// `reset_output_format()` is a no-panic clearing call.
    #[test]
    fn reset_output_format_no_panic() {
        let _g = crate::test_util::global_state_lock();
        reset_output_format();
        reset_output_format();
    }

    /// `lastbase()` returns the integer base of the last math
    /// literal parsed (C's `lastbase` global).
    #[test]
    fn lastbase_returns_int_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _r = lastbase();
    }

    /// `set_lastbase(N)` then `lastbase()` returns N.
    #[test]
    fn set_lastbase_round_trips() {
        let _g = crate::test_util::global_state_lock();
        let saved = lastbase();
        set_lastbase(16);
        assert_eq!(lastbase(), 16);
        set_lastbase(saved);
    }

    /// `m_noeval_set(0)` then `m_noeval()` returns 0.
    #[test]
    fn m_noeval_default_is_zero() {
        let _g = crate::test_util::global_state_lock();
        m_noeval_set(0);
        assert_eq!(m_noeval(), 0);
    }

    /// `m_noeval_set(N)` round-trips through getter.
    #[test]
    fn m_noeval_set_round_trips() {
        let _g = crate::test_util::global_state_lock();
        let saved = m_noeval();
        m_noeval_set(3);
        assert_eq!(m_noeval(), 3);
        m_noeval_set(saved);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/math.c matheval + mathevali +
    // outputradix / outputunderscore / reset_output_format.
    // ═══════════════════════════════════════════════════════════════════

    /// c:1491 — `matheval("")` returns MN_INTEGER 0.
    #[test]
    fn matheval_empty_returns_integer_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = matheval("").expect("empty must succeed");
        assert_eq!(r.l, 0);
        assert_eq!(r.type_, MN_INTEGER);
    }

    /// c:1489 — Nularg-prefixed expression is treated as the suffix.
    /// `Nularg + "42"` → 42.
    #[test]
    fn matheval_nularg_prefix_skipped() {
        let _g = crate::test_util::global_state_lock();
        let s = format!("{}42", Nularg);
        let r = matheval(&s).expect("must succeed");
        assert_eq!(r.l, 42, "Nularg prefix stripped, '42' evaluated");
    }

    /// c:1480 — `matheval("1+2")` returns 3.
    #[test]
    fn matheval_basic_addition() {
        let _g = crate::test_util::global_state_lock();
        let r = matheval("1+2").expect("must succeed");
        assert_eq!(r.l, 3);
    }

    /// c:1480 — `matheval("10*5")` returns 50.
    #[test]
    fn matheval_basic_multiplication() {
        let _g = crate::test_util::global_state_lock();
        let r = matheval("10*5").expect("must succeed");
        assert_eq!(r.l, 50);
    }

    /// c:1505 — `mathevali("3+4")` returns 7 (integer-coerce).
    #[test]
    fn mathevali_integer_result() {
        let _g = crate::test_util::global_state_lock();
        let r = mathevali("3+4").expect("must succeed");
        assert_eq!(r, 7);
    }

    /// c:1505 — `mathevali("3.7")` truncates to 3 (MN_FLOAT → i64).
    #[test]
    fn mathevali_float_truncates() {
        let _g = crate::test_util::global_state_lock();
        let r = mathevali("3.7").expect("must succeed");
        assert_eq!(r, 3, "float must truncate (cast to i64)");
    }

    /// c:1505 — `mathevali("-2.7")` truncates toward zero → -2.
    #[test]
    fn mathevali_negative_float_truncates_toward_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = mathevali("-2.7").expect("must succeed");
        assert_eq!(r, -2, "-2.7 truncates to -2 (toward zero, not -3)");
    }

    /// c:898 — `reset_output_format()` clears both radix + underscore.
    #[test]
    fn reset_output_format_clears_both_state() {
        let _g = crate::test_util::global_state_lock();
        reset_output_format();
        assert_eq!(outputradix(), 0);
        assert_eq!(outputunderscore(), 0);
    }

    /// c:889 — `outputradix` is deterministic right after reset.
    #[test]
    fn outputradix_zero_after_reset() {
        let _g = crate::test_util::global_state_lock();
        reset_output_format();
        for _ in 0..5 {
            assert_eq!(outputradix(), 0);
        }
    }

    /// c:1049 — `lastbase` accessor returns valid base value.
    #[test]
    fn lastbase_accessor_returns_value() {
        let _g = crate::test_util::global_state_lock();
        let saved = lastbase();
        set_lastbase(16);
        assert_eq!(lastbase(), 16);
        set_lastbase(saved);
    }

    /// c:1061 — `set_lastbase(8)` then `lastbase()` returns 8.
    #[test]
    fn set_lastbase_round_trips_pin() {
        let _g = crate::test_util::global_state_lock();
        let saved = lastbase();
        set_lastbase(8);
        assert_eq!(lastbase(), 8);
        set_lastbase(saved);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/math.c
    // c:889 outputradix / c:894 outputunderscore / c:901 reset_output_format
    // c:1022 m_noeval / c:1049 lastbase / c:2952 matheval / c:2995 mathevali
    // ═══════════════════════════════════════════════════════════════════

    /// c:889 — `outputradix` returns i32 (compile-time type pin).
    #[test]
    fn outputradix_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = outputradix();
    }

    /// c:894 — `outputunderscore` returns i32 (compile-time type pin).
    #[test]
    fn outputunderscore_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = outputunderscore();
    }

    /// c:1022 — `m_noeval` returns i32 (compile-time type pin).
    #[test]
    fn m_noeval_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = m_noeval();
    }

    /// c:1022 + c:1029 — `m_noeval` set/get round-trip preserves value.
    #[test]
    fn m_noeval_set_get_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let saved = m_noeval();
        m_noeval_set(42);
        assert_eq!(m_noeval(), 42, "m_noeval round-trips");
        m_noeval_set(saved);
    }

    /// c:1049 — `lastbase` returns i32 (compile-time type pin).
    #[test]
    fn lastbase_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = lastbase();
    }

    /// c:2952 — `matheval("")` empty returns Result<mnumber, String> type.
    #[test]
    fn matheval_returns_result_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Result<mnumber, String> = matheval("");
    }

    /// c:2995 — `mathevali("")` empty returns Result<i64, String>.
    #[test]
    fn mathevali_returns_result_i64_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Result<i64, String> = mathevali("");
    }

    /// c:2952 — `matheval("0")` returns Ok with l=0.
    #[test]
    fn matheval_zero_returns_ok_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = matheval("0").expect("0 must parse");
        assert_eq!(r.l, 0, "matheval('0').l = 0");
    }

    /// c:2995 — `mathevali("1+1")` returns Ok(2).
    #[test]
    fn mathevali_simple_addition_returns_two() {
        let _g = crate::test_util::global_state_lock();
        let r = mathevali("1+1").expect("1+1 must parse");
        assert_eq!(r, 2, "1+1 must equal 2");
    }

    /// c:2952 — `matheval` is pure for the same input (no side effects).
    #[test]
    fn matheval_is_pure_for_constants() {
        let _g = crate::test_util::global_state_lock();
        for s in ["0", "1", "42", "100"] {
            let first = matheval(s).map(|r| r.l).unwrap_or(0);
            for _ in 0..3 {
                let r = matheval(s).map(|r| r.l).unwrap_or(0);
                assert_eq!(r, first, "matheval({:?}) must be pure", s);
            }
        }
    }

    /// c:2995 — `mathevali` accepts an undefined identifier as 0/parameter
    /// lookup per zsh math semantics (unset → 0); pin the deterministic
    /// fallback behavior rather than expecting Err. C body c:3030+
    /// passes unknown idents through `getmathparam` which returns 0 for
    /// unset names by zsh-default contract.
    #[test]
    fn mathevali_undefined_ident_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = mathevali("__never_real_var_xyz__");
        for _ in 0..3 {
            assert_eq!(
                mathevali("__never_real_var_xyz__"),
                first,
                "mathevali on undefined ident must be deterministic"
            );
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Regression pins for setmathvar subscripted-lvalue writes.
    //
    // Prior to commit 2026-05-29, setmathvar stripped the subscript
    // before calling setnparam, so `(( h[k]++ ))` wrote to the bare
    // base name and wiped the assoc/array. C's setmathvar at
    // Src/math.c:1004 passes the FULL `mvp->lval` (subscript and
    // all) to setnparam — fixed by routing subscripted writes
    // through assignsparam after pre-evaluating numeric subscripts
    // via matheval.
    // ═══════════════════════════════════════════════════════════════════

    /// Helper — read assoc element directly out of the hashed-storage
    /// backing map. `getsparam("h[k]")` doesn't resolve the subscript
    /// form in our port; the canonical hash-element read goes through
    /// expansion (subst), so these unit tests poke the storage map
    /// the same way `assignsparam`'s subscript-write path does.
    fn assoc_read(base: &str, key: &str) -> Option<String> {
        let m = crate::ported::params::paramtab_hashed_storage()
            .lock()
            .ok()?;
        m.get(base).and_then(|map| map.get(key).cloned())
    }

    /// c:Src/math.c:1004 — `(( assoc[key]++ ))` must mutate the
    /// hash element, not the bare assoc param. Bug: silent no-op
    /// (counts[apple] stayed at 10).
    #[test]
    fn setmathvar_assoc_post_increment_mutates_hash_element() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::unsetparam("counts");
        // Create assoc with apple=10 — `sethparam` is the `typeset -A`
        // equivalent. A bare `counts[apple]=10` on an UNSET name is not:
        // zsh math-evaluates the subscript of a non-assoc, so it fails
        // with "assignment to invalid subscript range" (verified against
        // /bin/zsh) and no param is created.
        let _ =
            crate::ported::params::sethparam("counts", vec!["apple".to_string(), "10".to_string()]);
        // (( counts[apple]++ )) → read 10, write 11.
        let _ = setmathvar(
            "counts[apple]",
            mnumber {
                l: 11,
                d: 0.0,
                type_: MN_INTEGER,
            },
        );
        assert_eq!(
            assoc_read("counts", "apple"),
            Some("11".to_string()),
            "(( counts[apple]++ )) must mutate the hash element",
        );
        crate::ported::params::unsetparam("counts");
    }

    /// c:Src/math.c:1004 — `(( assoc[key] = N ))` on an existing
    /// assoc creates/updates the slot without wiping siblings.
    #[test]
    fn setmathvar_assoc_assign_creates_slot_preserving_siblings() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::unsetparam("h");
        let _ = crate::ported::params::sethparam(
            "h",
            vec![
                "a".to_string(),
                "1".to_string(),
                "b".to_string(),
                "2".to_string(),
            ],
        );
        let _ = setmathvar(
            "h[c]",
            mnumber {
                l: 99,
                d: 0.0,
                type_: MN_INTEGER,
            },
        );
        assert_eq!(
            assoc_read("h", "a"),
            Some("1".to_string()),
            "sibling h[a] preserved",
        );
        assert_eq!(
            assoc_read("h", "b"),
            Some("2".to_string()),
            "sibling h[b] preserved",
        );
        assert_eq!(
            assoc_read("h", "c"),
            Some("99".to_string()),
            "h[c] created with assigned value",
        );
        crate::ported::params::unsetparam("h");
    }

    /// c:Src/math.c:1004 — `(( arr[i] = N ))` on indexed array
    /// must write element i, not wipe the array and replace with
    /// a scalar.
    #[test]
    fn setmathvar_indexed_array_assign_writes_element() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::unsetparam("arr");
        let _ = crate::ported::params::assignaparam(
            "arr",
            vec!["10".to_string(), "20".to_string(), "30".to_string()],
            0,
        );
        let _ = setmathvar(
            "arr[2]",
            mnumber {
                l: 99,
                d: 0.0,
                type_: MN_INTEGER,
            },
        );
        assert_eq!(
            crate::ported::params::getaparam("arr"),
            Some(vec!["10".to_string(), "99".to_string(), "30".to_string()]),
            "(( arr[2]=99 )) must write slot 2 only",
        );
        crate::ported::params::unsetparam("arr");
    }

    /// c:Src/math.c:1004 + Src/params.c:1367 — `(( arr[i + 1] = N ))`
    /// inside math context: subscript body is a math expression and
    /// must be evaluated. Pinned because the previous Rust port
    /// passed "arr[i + 1]" verbatim and assignsparam's i64-parse on
    /// the body failed, auto-vivifying as PM_HASHED.
    #[test]
    fn setmathvar_array_with_math_subscript_pre_evaluates_index() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::unsetparam("arr");
        crate::ported::params::unsetparam("i");
        let _ = crate::ported::params::setiparam("i", 2);
        let _ = crate::ported::params::assignaparam(
            "arr",
            vec![
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
                "d".to_string(),
            ],
            0,
        );
        // arr[i + 1] → arr[3] after matheval.
        let _ = setmathvar(
            "arr[i + 1]",
            mnumber {
                l: 77,
                d: 0.0,
                type_: MN_INTEGER,
            },
        );
        let got = crate::ported::params::getaparam("arr");
        assert_eq!(
            got.as_ref().and_then(|v| v.get(2)).map(|s| s.as_str()),
            Some("77"),
            "arr[i+1] with i=2 must write slot 3",
        );
        crate::ported::params::unsetparam("arr");
        crate::ported::params::unsetparam("i");
    }

    /// c:Src/math.c:1004 — chained `(( h[k]++ ))` calls compound:
    /// three increments on a fresh slot yield 3, not 1.
    #[test]
    fn setmathvar_assoc_three_increments_compound_to_three() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::unsetparam("hc");
        let _ = crate::ported::params::sethparam("hc", vec!["x".to_string(), "0".to_string()]);
        for _ in 0..3 {
            // Read current then write read+1 — like (( hc[x]++ )).
            let cur = assoc_read("hc", "x")
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(0);
            let _ = setmathvar(
                "hc[x]",
                mnumber {
                    l: cur + 1,
                    d: 0.0,
                    type_: MN_INTEGER,
                },
            );
        }
        assert_eq!(
            assoc_read("hc", "x"),
            Some("3".to_string()),
            "three (( hc[x]++ )) must compound to 3",
        );
        crate::ported::params::unsetparam("hc");
    }

    /// c:Src/math.c:1004 — `(( h[fresh]++ ))` auto-vivifies the
    /// slot to value 1 (read NULL → 0, write 0+1).
    #[test]
    fn setmathvar_assoc_post_increment_on_unset_slot_creates_with_one() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::unsetparam("hv");
        // Create the assoc first (typeset -A hv).
        let _ = crate::ported::params::sethparam("hv", vec!["seed".to_string(), "0".to_string()]);
        // (( hv[fresh]++ )) — fresh slot should become 1.
        let _ = setmathvar(
            "hv[fresh]",
            mnumber {
                l: 1,
                d: 0.0,
                type_: MN_INTEGER,
            },
        );
        assert_eq!(
            assoc_read("hv", "fresh"),
            Some("1".to_string()),
            "(( hv[fresh]++ )) on unset slot must create with 1",
        );
        crate::ported::params::unsetparam("hv");
    }

    /// c:Src/math.c:1002 — NO_EXEC mode: setmathvar returns val
    /// without any param-table mutation, even for subscripted
    /// names. Pinned because the new subscript branch must respect
    /// the noeval guard added by the previous code path.
    #[test]
    fn setmathvar_subscript_respects_noeval_guard() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::unsetparam("nev");
        let _ = crate::ported::params::sethparam("nev", vec!["k".to_string(), "1".to_string()]);
        M_NOEVAL.with(|n| n.set(1));
        let v = mnumber {
            l: 999,
            d: 0.0,
            type_: MN_INTEGER,
        };
        let ret = setmathvar("nev[k]", v);
        M_NOEVAL.with(|n| n.set(0));
        assert_eq!(ret.l, 999, "noeval returns val unchanged");
        assert_eq!(
            assoc_read("nev", "k"),
            Some("1".to_string()),
            "noeval must suppress the paramtab write",
        );
        crate::ported::params::unsetparam("nev");
    }
}
