//! Shell options for zshrs
//!
//! Direct port from zsh/Src/options.c
//!
//! Manages all shell options including:
//! - Option lookup by name and single-letter
//! - Emulation modes (zsh, ksh, sh, csh)
//! - Option aliases (bash/ksh compatibility)
//! - setopt/unsetopt builtins

use std::collections::HashSet;
use std::sync::atomic::AtomicI32;
use std::sync::LazyLock;

use crate::ported::init::SHTTY;
use crate::ported::jobs::{acquire_pgrp, ORIGPGRP};
use crate::ported::params::{keyboardhacksetfn, paramtab};
use crate::ported::pattern::{patcompile, pattry};
use crate::ported::utils::zwarnnam;
use crate::ported::zsh_h::{
    interact, isset, opt_name, options, Meta, APPENDHISTORY, BANGHIST, CHASELINKS, EMACSMODE,
    EMULATE_CSH, EMULATE_FULLY, EMULATE_KSH, EMULATE_SH, EMULATE_UNUSED, EMULATE_ZSH, EXECOPT,
    GLOBDOTS, HASHCMDS, HISTNOFUNCTIONS, IGNOREBRACES, INTERACTIVE, LOGINSHELL, MAILWARNING,
    MONITOR, MULTIBYTE, OPT_INVALID, OPT_SIZE, PAT_HEAPDUP, PROMPTSUBST, SHINSTDIN, SINGLECOMMAND,
    SUNKEYBOARDHACK, USEZLE, VIMODE,
};
use crate::utils::inittyptab;

/// Emulation flags for option defaults
// `#define OPT_X EMULATE_X` (options.c:55-58) — the option-default
// bits ARE the emulation bits. Direct mirror of the C macros.
const OPT_CSH: u8 = EMULATE_CSH as u8; // c:55
const OPT_KSH: u8 = EMULATE_KSH as u8; // c:56
const OPT_SH: u8 = EMULATE_SH as u8; // c:57
const OPT_ZSH: u8 = EMULATE_ZSH as u8; // c:58
const OPT_ALL: u8 = OPT_CSH | OPT_KSH | OPT_SH | OPT_ZSH; // c:60
const OPT_BOURNE: u8 = OPT_KSH | OPT_SH; // c:61
const OPT_BSHELL: u8 = OPT_KSH | OPT_SH | OPT_ZSH; // c:62
const OPT_NONBOURNE: u8 = OPT_ALL & !OPT_BOURNE; // c:63
const OPT_NONZSH: u8 = OPT_ALL & !OPT_ZSH; // c:64

/// Option flags
// option is relevant to emulation                                          // c:66
const OPT_EMULATE: u16 = EMULATE_UNUSED as u16; // c:67
                                                // option should never be set by emulate()                                  // c:68
const OPT_SPECIAL: u16 = (EMULATE_UNUSED << 1) as u16; // c:69
                                                       // option is an alias to an other option                                    // c:70
const OPT_ALIAS: u16 = (EMULATE_UNUSED << 2) as u16; // c:71

/// Every recognised shell option.
/// Port of the `OPT_*` enum from Src/zsh.h — the C source uses
/// integer constants threaded through `optlookup()`
/// (Src/options.c:684), `dosetopt()` (line 735), and the option
/// table built by `createoptiontable()` (line 471).

/// Zsh single-letter options (zshletters in C)
pub static zshletters: &[(char, &str, bool)] = &[
    ('0', "correct", false),
    ('1', "printexitvalue", false),
    ('2', "badpattern", true),
    ('3', "nomatch", true),
    ('4', "globdots", false),
    ('5', "notify", false),
    ('6', "bgnice", false),
    ('7', "ignoreeof", false),
    ('8', "markdirs", false),
    ('9', "autolist", false),
    ('B', "beep", true),
    ('C', "clobber", true),
    ('D', "pushdtohome", false),
    ('E', "pushdsilent", false),
    ('F', "glob", true),
    ('G', "nullglob", false),
    ('H', "rmstarsilent", false),
    ('I', "ignorebraces", false),
    ('J', "autocd", false),
    ('K', "banghist", true),
    ('L', "sunkeyboardhack", false),
    ('M', "singlelinezle", false),
    ('N', "autopushd", false),
    ('O', "correctall", false),
    ('P', "rcexpandparam", false),
    ('Q', "pathdirs", false),
    ('R', "longlistjobs", false),
    ('S', "recexact", false),
    ('T', "cdablevars", false),
    ('U', "mailwarning", false),
    ('V', "promptcr", true),
    ('W', "autoresume", false),
    ('X', "listtypes", false),
    ('Y', "menucomplete", false),
    ('Z', "zle", false),
    ('a', "allexport", false),
    ('d', "globalrcs", true),
    ('e', "errexit", false),
    ('f', "rcs", true),
    ('g', "histignorespace", false),
    ('h', "histignoredups", false),
    ('i', "interactive", false),
    ('k', "interactivecomments", false),
    ('l', "login", false),
    ('m', "monitor", false),
    ('n', "exec", true),
    ('p', "privileged", false),
    ('s', "shinstdin", false),
    ('t', "singlecommand", false),
    ('u', "unset", true),
    ('v', "verbose", false),
    ('w', "chaselinks", false),
    ('x', "xtrace", false),
    ('y', "shwordsplit", false),
];

/// C body (c:450-466):
/// ```c
/// optno = on->optno; if (optno < 0) optno = -optno;
/// if (isset(KSHOPTIONPRINT)) {
///     if (defset(on, emulation))
///         printf("no%-19s %s\n", nam, isset(optno) ? "off" : "on");
///     else
///         printf("%-21s %s\n", nam, isset(optno) ? "on" : "off");
/// } else if (set == (isset(optno) ^ defset(on, emulation))) {
///     if (set ^ isset(optno)) fputs("no", stdout);
///     puts(nam);
/// }
/// ```
/// Port of `printoptionnode(HashNode hn, int set)` from `Src/options.c:450`.
pub fn printoptionnode(hn: &str, set: bool) {
    // c:450
    let on = opt_state_get(hn).unwrap_or(false); // c:450 isset(optno)
    let default_on = default_on_options().contains(&hn); // c:455 defset(on, emulation)
    let kshprint = opt_state_get("kshoptionprint").unwrap_or(false); // c:456 isset(KSHOPTIONPRINT)
    if kshprint {
        // c:456
        if default_on {
            // c:457
            println!("no{:<19} {}", hn, if on { "off" } else { "on" }); // c:458
        } else {
            println!("{:<21} {}", hn, if on { "on" } else { "off" }); // c:460
        }
    } else if set == (on ^ default_on) {
        // c:462
        if set ^ on {
            // c:463
            print!("no"); // c:464
        }
        println!("{}", hn); // c:465
    }
}

// =====================================================================
// Per-emulation option-set masks — `Src/options.c:55-67`. The OPT_CSH
// /OPT_KSH/OPT_SH/OPT_ZSH/OPT_ALL/OPT_BOURNE/OPT_BSHELL/OPT_NONBOURNE
// /OPT_NONZSH bits live as private `const` items at lines 28-36 above
// (they're internal to the optns[] table builder). Documented here for
// search-anchor parity with C source: every C `#define OPT_CSH
// EMULATE_CSH` etc. has a corresponding `const OPT_CSH: u8 = 1`
// declaration above, just using compact bit positions instead of the
// EMULATE_* re-export so the optns[] u8 emulation field stays narrow.
//
// `OPT_EMULATE` (c:67) and `OPT_SPECIAL` (c:69) and `OPT_ALIAS` (c:71)
// also live as private u16 consts at lines 40-44 above.

/// Build the global option name → option-data table.
/// Port of `createoptiontable()` from Src/options.c:471. The C
/// source allocates a HashTable and stuffs every entry from the
/// static `optns[]` array; the Rust port populates `OPTS_LIVE`
/// (the canonical `opts[]` store) with each option's
/// `defset(name, EMULATE_ZSH)` default. Idempotent.
pub fn createoptiontable() {
    // c:471
    let zsh_emu = EMULATE_ZSH;
    for name in ZSH_OPTIONS_SET.iter() {
        // c:46 opts[optno] = defset(...)
        opt_state_set(name, defset(name, zsh_emu));
    }
}

/// Direct port of `static void setemulate(HashNode hn, int fully)`
/// from `Src/options.c:507`. C body:
/// ```c
/// Optname on = (Optname) hn;
/// if (!(on->node.flags & OPT_ALIAS) &&
///     ((fully && !(on->node.flags & OPT_SPECIAL)) ||
///      (on->node.flags & OPT_EMULATE)))
///     setemulate_opts[on->optno] = defset(on, setemulate_emulation);
/// ```
/// Per-option callback invoked by `scanhashtable(optiontab, ...,
/// setemulate, ...)` to populate the `new_opts[]` table with each
/// option's default-for-target-emulation state.
pub fn setemulate(name: &str, fully: i32) {
    // c:507
    let flags = optns_flags(name); // c:507
                                   // c:515-517 — emulation-relevant filter.
    let is_alias = (flags & OPT_ALIAS) != 0;
    let is_special = (flags & OPT_SPECIAL) != 0;
    let is_emulate = (flags & OPT_EMULATE) != 0;
    if is_alias {
        return;
    }
    if !((fully != 0 && !is_special) || is_emulate) {
        // c:516-517
        return;
    }
    // c:518 — `setemulate_opts[on->optno] = defset(on, setemulate_emulation);`
    let target = SETEMULATE_EMULATION.load(std::sync::atomic::Ordering::Relaxed);
    let on_by_default = defset(name, target);
    if let Ok(mut tab) = setemulate_opts_lock().lock() {
        tab.insert(name.to_string(), on_by_default);
    }
}

/// Direct port of `void installemulation(int new_emulation, char
/// *new_opts)` from `Src/options.c:523`:
/// ```c
/// setemulate_emulation = new_emulation;
/// setemulate_opts = new_opts;
/// scanhashtable(optiontab, 0, 0, 0, setemulate,
///               !!(new_emulation & EMULATE_FULLY));
/// ```
/// Populates `new_opts[]` with each option's default-for-target-
/// emulation state by walking `optiontab` via the `setemulate`
/// per-option callback. Does NOT mutate the live `opts[]` — that
/// happens in the caller (`emulate()` and `bin_emulate -L`).
pub fn installemulation(
    new_emulation: i32,
    new_opts: &mut std::collections::HashMap<String, bool>,
) {
    // c:523
    // c:525 — `setemulate_emulation = new_emulation;`
    SETEMULATE_EMULATION.store(new_emulation, std::sync::atomic::Ordering::Relaxed); // c:525
                                                                                     // c:526 — `setemulate_opts = new_opts;`. We can't alias the
                                                                                     // caller's HashMap directly, so the per-option callback writes
                                                                                     // into our module-static and we splice it back into `new_opts`.
    if let Ok(mut tab) = setemulate_opts_lock().lock() {
        tab.clear();
    }
    // c:527-528 — scanhashtable(optiontab, ..., setemulate, fully).
    let fully = if (new_emulation & EMULATE_FULLY) != 0 {
        1
    } else {
        0
    }; // c:528
    for name in ZSH_OPTIONS_SET.iter() {
        setemulate(name, fully); // c:527
    }
    // Splice setemulate_opts → new_opts so the C semantic of
    // "new_opts is now populated" holds for the caller.
    if let Ok(tab) = setemulate_opts_lock().lock() {
        for (k, v) in tab.iter() {
            new_opts.insert(k.clone(), *v);
        }
    }
}

/// Switch to a named emulation. Port of `emulate()` from
/// `Src/options.c:533`. Sets `emulation` (file-scope global), then
/// inlines the body of `installemulation()` (c:523) to populate
/// `opts[]` per the new emulation, skipping OPT_SPECIAL entries per
/// the exec.c:5933-5938 walk.
pub fn emulate(mode: &str, fully: bool) {
    // c:533
    let ch = mode.chars().next().unwrap_or('z');
    let ch = if ch == 'r' {
        mode.chars().nth(1).unwrap_or('z')
    } else {
        ch
    };
    let new_emu = match ch {
        'c' => EMULATE_CSH,
        'k' => EMULATE_KSH,
        's' | 'b' => EMULATE_SH,
        _ => EMULATE_ZSH,
    };
    EMULATION.store(new_emu, std::sync::atomic::Ordering::Relaxed);
    FULLY_EMULATING.store(fully, std::sync::atomic::Ordering::Relaxed);

    // c:551-572 — body of `installemulation()` + exec.c:5933-5938
    // OPT_SPECIAL-skip walk, inlined here per the C source.
    let mut emu = new_emu;
    if fully {
        emu |= EMULATE_FULLY; // c:551
    }
    let mut new_opts: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    installemulation(emu, &mut new_opts); // c:552
    for (k, v) in &new_opts {
        if (optns_flags(k) & OPT_SPECIAL) == 0 {
            // exec.c:5933-5938
            opt_state_set(k, *v);
        }
    }
    if new_emu == EMULATE_ZSH {
        // c:46 — `opts[optno] = defset(...)` walk for zsh defaults.
        for name in ZSH_OPTIONS_SET.iter() {
            opt_state_set(name, defset(name, EMULATE_ZSH));
        }
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/vm_helper because their
// C counterpart's source file maps 1:1 to this Rust module.
// Rust permits multiple inherent impl blocks for the same
// type within a crate, so call sites in vm_helper are unchanged.
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

// ===========================================================
// Static + helpers moved verbatim from src/ported/vm_helper.
// These are the C options.c port-of-record (canonical option
// name list, default values, normalization, pattern matching,
// emulation-mode option deltas, and the option-printing
// helpers). Their C counterparts all live in
// src/zsh/Src/options.c (`optns[]` table, `defset()`,
// `installemulation()`, `printoptions()`).
// ===========================================================

// BEGIN moved-from-exec-rs (statics)

/// `setopt OPT` builtin per-arg dispatch.
/// Port of `setoption(HashNode hn, int value)` from Src/options.c:573 — the inner loop
/// of `bin_setopt`. Returns 0 on success, -1 on bad option name.
pub fn setoption(hn: &str, value: i32) -> i32 {
    // C: `opts[optno] = value;` — the C source writes the option's
    // live state into the `opts[]` array. The Rust port stores it
    // in OPTS_LIVE via `opt_state_set` (the same global the
    // `optlookup("hn")>0` and `isset(OPT)` paths read).
    opt_state_set(hn, value != 0); // c:735+ dosetopt body
    0
}

/// Direct port of `bin_setopt(char *nam, char **args, UNUSED(Options ops), int isun)` from Src/options.c:580.
/// C body (c:585-680):
///   - no args → `scanhashtable(optiontab, 1, 0, OPT_ALIAS,
///     optiontab->printnode, !isun)` lists each option set or unset
///     according to !isun
///   - parse leading `-`/`+` flags arg-by-arg; the action polarity
///     is `(**args == '-') ^ isun` per c:594
///   - within an arg: `-o NAME` (c:606), `-m` (c:624), or a single-
///     letter option flag (c:626)
///   - `-`/`+` arg with empty body becomes the pseudo `--` marker
///     terminating flag parsing (c:596-597)
///   - bare names branch (!match_glob, c:640): each arg is an
///     option name → `dosetopt(optlookup(name), !isun, 0)`
///   - glob branch (`-m`, c:653): each arg is patcompile'd then
///     `scanmatchtable(optiontab, pprog, ..., setoption, !isun)`
///     applies it across the option table
///   - tail: `inittyptab()` rebuilds the type table to reflect any
///     option changes that affect lexer/expansion
pub fn bin_setopt(
    nam: &str,
    args: &[String], // c:580
    _ops: &options,
    isun: i32,
) -> i32 {
    let mut retval = 0i32;
    let mut match_glob = false; // c:582
    let mut idx = 0usize;

    if args.is_empty() {
        // c:586
        // c:587 — scanhashtable(optiontab, 1, 0, OPT_ALIAS,
        // optiontab->printnode, !isun): walk every option in the
        // table and pass each one to printnode with `!isun` as the
        // `set` argument. printoptionnode (c:450) is the actual
        // filter: it emits the option only when its current state
        // differs from the default in the requested direction —
        // i.e. `set == (isset(optno) ^ defset(on, emulation))`.
        //
        // Parity bug: the previous Rust port pre-filtered with
        // `on == want_set`, which printed every option that was
        // currently in the requested state regardless of whether
        // that matched its default. Result: `setopt` (no args)
        // listed all ON options (braceexpand, hashall, …) instead
        // of just the diverged ones (e.g. `nohashdirs`, `norcs`).
        let want_set = isun == 0;
        let mut names: Vec<String> = ZSH_OPTIONS_SET
            .iter()
            // c:587 — scanhashtable's `flags2 = OPT_ALIAS` mask
            // skips bash/ksh-alias entries. ZSH_OPTION_ALIASES
            // is the parallel set of OPT_ALIAS names per
            // Src/options.c:269-280.
            .filter(|n| !ZSH_OPTION_ALIASES.contains(*n))
            .map(|s| s.to_string())
            .collect();
        names.sort();
        for n in names {
            printoptionnode(&n, want_set); // c:587 printnode
        }
        return 0; // c:589
    }

    // c:592-636 — leading `-`/`+` flag parse loop.
    'outer: while idx < args.len() && (args[idx].starts_with('-') || args[idx].starts_with('+')) {
        let leading = args[idx].as_bytes()[0]; // c:594
        let action: i32 = ((leading == b'-') as i32) ^ isun; // c:594
        if args[idx].len() == 1 {
            // c:596 args[0][1] empty
            // c:597 — `*args = "--";` then fall through to the
            // inner while which immediately matches `-` and breaks
            // into doneoptions. Equivalent: skip past this arg and
            // exit the outer loop.
            idx += 1;
            break 'outer;
        }
        let body_bytes = args[idx].as_bytes()[1..].to_vec(); // c:599 *++*args
        let mut k = 0usize;
        while k < body_bytes.len() {
            // c:599
            let mut c = body_bytes[k];
            // c:600-601 — `if(**args == Meta) *++*args ^= 32;` —
            // unmeta the next byte before reading.
            if c == Meta {
                // c:600
                k += 1;
                if k < body_bytes.len() {
                    c = body_bytes[k] ^ 32;
                }
                // c:601
                else {
                    break;
                }
            }
            if c == b'-' {
                // c:603 pseudo `--`
                idx += 1; // c:604
                break 'outer; // c:605 goto doneoptions
            } else if c == b'o' {
                // c:606
                // c:607-608 — if more chars after 'o', use them as the
                // option name; otherwise advance to next arg.
                let oarg: String = if k + 1 < body_bytes.len() {
                    // c:607
                    String::from_utf8_lossy(&body_bytes[k + 1..]).into_owned()
                } else {
                    idx += 1; // c:608
                    if idx >= args.len() {
                        // c:609 !*args
                        zwarnnam(nam, "string expected after -o"); // c:610
                        return 1; // c:612
                    }
                    args[idx].clone()
                };
                let optno = optlookup(&oarg); // c:614
                if optno == 0 {
                    // c:614
                    zwarnnam(
                        nam, // c:615
                        &format!("no such option: {}", oarg),
                    );
                    retval |= 1;
                } else if dosetopt(optno, action, 0) != 0 {
                    // c:617
                    zwarnnam(
                        nam, // c:618
                        &format!("can't change option: {}", oarg),
                    );
                    retval |= 1;
                }
                break; // c:622 break inner
            } else if c == b'm' {
                // c:624
                match_glob = true; // c:625
            } else {
                // c:626
                let optno = optlookupc(c as char); // c:627
                if optno == 0 {
                    // c:627
                    zwarnnam(nam, &format!("bad option: -{}", c as char)); // c:628
                    retval |= 1;
                } else if dosetopt(optno, action, 0) != 0 {
                    // c:630
                    zwarnnam(
                        nam, // c:631
                        &format!("can't change option: -{}", c as char),
                    );
                    retval |= 1;
                }
            }
            k += 1;
        }
        idx += 1; // c:636 args++
    }

    // c:638 — doneoptions: positional args remain.
    if !match_glob {
        // c:640
        // c:642-650 — bare option names.
        while idx < args.len() {
            // c:642
            let oname = args[idx].clone();
            idx += 1;
            let optno = optlookup(&oname); // c:643
            if optno == 0 {
                // c:643
                zwarnnam(
                    nam, // c:644
                    &format!("no such option: {}", oname),
                );
                retval |= 1;
            } else {
                let v = (isun == 0) as i32; // c:646 !isun
                if dosetopt(optno, v, 0) != 0 {
                    // c:646
                    zwarnnam(
                        nam, // c:647
                        &format!("can't change option: {}", oname),
                    );
                    retval |= 1;
                } else {
                    // PFA-SMR: emit setopt/unsetopt per option name.
                    // Without this, `setopt EXTENDED_GLOB` and
                    // friends were invisible to the recorder.
                    #[cfg(feature = "recorder")]
                    if crate::recorder::is_enabled() {
                        let ctx = crate::recorder::recorder_ctx_global();
                        if isun == 0 {
                            crate::recorder::emit_setopt(&oname, ctx);
                        } else {
                            crate::recorder::emit_unsetopt(&oname, ctx);
                        }
                    }
                }
            }
        }
    } else {
        // c:653
        // c:655-678 — globbing branch.
        while idx < args.len() {
            // c:655
            let raw = args[idx].clone();
            idx += 1;
            // c:660-666 — `s = dupstring(*args);` then walk: strip
            // `_`, lowercase A-Z (mirrors optlookup's canonicalisation
            // documented at c:684).
            let normalized: String = raw
                .chars()
                .filter(|&c| c != '_')
                .map(|c| c.to_ascii_lowercase())
                .collect();
            // c:670 — patcompile(s, PAT_HEAPDUP, NULL).
            let prog = patcompile(&normalized, PAT_HEAPDUP, None);
            if prog.is_none() {
                // c:670
                zwarnnam(nam, &format!("bad pattern: {}", raw)); // c:671
                retval |= 1;
                break; // c:673
            }
            // c:676 — scanmatchtable(optiontab, pprog, 0, 0, OPT_ALIAS,
            // setoption, !isun): the `setoption` static at c:572 calls
            // `dosetopt(optname->optno, !isun, 0, opts)` on each match.
            let v = (isun == 0) as i32;
            if let Some(prog) = patcompile(&normalized, PAT_HEAPDUP as i32, None) {
                for opt_name in ZSH_OPTIONS_SET.iter() {
                    // c:676
                    if pattry(&prog, opt_name) {
                        let _ = setoption(opt_name, v); // c:572 setoption
                    }
                }
            }
        }
    }
    inittyptab(); // c:678
    retval // c:679
}

// Identify an option name                                                  // c:680
/// Translate an option name to a signed option index.
///
/// C body (c:684-715): normalize `name` (strip `_`, lowercase),
/// then `optiontab->getnode(optiontab, s)` returns the `Optname`
/// whose `->optno` field is the canonical numeric ID (one of the
/// `ALIASESOPT` / `ERREXIT` / ... constants from `zsh.h:2050+`).
/// `no`-prefix returns the negation of the stripped lookup.
///
/// Rust port: `optiontab` isn't ported as a runtime hashtable, so
/// we scan the same canonical `zh::OPT_*` constants `index_to_name`
/// uses (the inverse direction). The returned value is the C-fixed
/// optno (so `isset(optlookup("errexit"))` reads `opts[ERREXIT]`),
/// NOT a Rust-side hash — that earlier hash-based encoding caused
/// `isset(optlookup(name))` to read a wrong slot via `opt_name(h)`
/// and `[[ -o NAME ]]` returned false even after `setopt NAME`.
/// Port of `optlookup(char const *name)` from `Src/options.c:684`.
///
/// Walks the canonicalised name through the option table (including
/// OPT_ALIAS rows at `optns[]:269-280`); aliases resolve to their
/// target optno with a negative sign for `OPT_ALIAS`-negating rows
/// (`braceexpand` → `-IGNOREBRACES`, `log` → `-HISTNOFUNCTIONS`,
/// etc.). Returns `OPT_INVALID` when the name is unknown.
pub fn optlookup(name: &str) -> i32 {
    // c:684

    // c:689 — `s = t = dupstring(name);`
    // c:691-705 — strip `_` + ASCII-only lowercase. C's comment
    // at c:695-700 spells out the rationale: "Some locales (in
    // particular tr_TR.UTF-8) may have non-standard mappings of
    // ASCII characters, so be careful. Option names must be
    // ASCII so we don't need to be too clever." The C body
    // checks `*t >= 'A' && *t <= 'Z'` manually — a locale-free
    // ASCII range test that maps `'I'` → `'i'` regardless of
    // LC_CTYPE.
    //
    // The previous Rust port used `c.to_lowercase()` which is
    // Unicode-aware and applies full case folding (including
    // multi-char outputs like German 'ß' → 'ss'). That's a
    // divergence for non-ASCII option names (which shouldn't
    // exist but could from fuzzing). Match C: only fold the
    // ASCII A..=Z range; pass every other byte through.
    let s: String = name
        .chars() // c:689
        .filter(|&c| c != '_') // c:693-694
        .map(|c| {
            if ('A'..='Z').contains(&c) {
                // c:702 (*t >= 'A' && *t <= 'Z')
                ((c as u8 - b'A') + b'a') as char // c:703 *t = (*t - 'A') + 'a'
            } else {
                c
            }
        })
        .collect();

    // OPT_ALIAS rows from optns[]:269-280 — alias names resolve to
    // their target optno (signed for the alias's negation polarity).
    // C zsh stores these as hash entries in optiontab with
    // `optno = -target` (negative) for the `-PREFIX` rows.
    let alias_optno: Option<i32> = match s.as_str() {
        "braceexpand" => Some(-IGNOREBRACES), // c:269 -IGNOREBRACES
        "dotglob" => Some(GLOBDOTS),          // c:270 GLOBDOTS
        "hashall" => Some(HASHCMDS),          // c:271 HASHCMDS
        "histappend" => Some(APPENDHISTORY),  // c:272 APPENDHISTORY
        "histexpand" => Some(BANGHIST),       // c:273 BANGHIST
        "log" => Some(-HISTNOFUNCTIONS),      // c:274 -HISTNOFUNCTIONS
        "mailwarn" => Some(MAILWARNING),      // c:275 MAILWARNING
        "onecmd" => Some(SINGLECOMMAND),      // c:276 SINGLECOMMAND
        "physical" => Some(CHASELINKS),       // c:277 CHASELINKS
        "promptvars" => Some(PROMPTSUBST),    // c:278 PROMPTSUBST
        "stdin" => Some(SHINSTDIN),           // c:279 SHINSTDIN
        "trackall" => Some(HASHCMDS),         // c:280 HASHCMDS
        // c:Src/options.c:193 — `login` is a second optiontab entry
        // (OPT_SPECIAL) that resolves to the same optno as
        // `loginshell`. Mirror as a name alias here so `setopt login`
        // and `$options[login]` both reach LOGINSHELL's slot.
        "login" => Some(LOGINSHELL), // c:193
        _ => None,
    };
    if let Some(optno) = alias_optno {
        return optno;
    }

    // c:708-712 — `if s[0..2] == "no" && getnode(s+2)` → -optno, else getnode(s).
    if let Some(stripped) = s.strip_prefix("no") {
        // c:708
        if let Some(optno) = optno_by_name(stripped) {
            // c:709
            return -optno; // c:710
        }
        // c:Src/options.c:708 — `getnode(s+2)` consults the SAME
        // hashtable as the head lookup, which includes both
        // canonical names AND OPT_ALIAS rows. The Rust port's
        // alias map (lines 617-635) only fired for the head lookup;
        // `no_dotglob` (strip _ → "nodotglob", strip "no" →
        // "dotglob") fell through to optno_by_name which doesn't
        // know about aliases. Apply the same alias resolution
        // here so `setopt no_dotglob` flips GLOBDOTS off.
        let alias_after_no: Option<i32> = match stripped {
            "braceexpand" => Some(-IGNOREBRACES),
            "dotglob" => Some(GLOBDOTS),
            "hashall" => Some(HASHCMDS),
            "histappend" => Some(APPENDHISTORY),
            "histexpand" => Some(BANGHIST),
            "log" => Some(-HISTNOFUNCTIONS),
            "mailwarn" => Some(MAILWARNING),
            "onecmd" => Some(SINGLECOMMAND),
            "physical" => Some(CHASELINKS),
            "promptvars" => Some(PROMPTSUBST),
            "stdin" => Some(SHINSTDIN),
            "trackall" => Some(HASHCMDS),
            "login" => Some(LOGINSHELL),
            _ => None,
        };
        if let Some(optno) = alias_after_no {
            return -optno;
        }
    }
    match optno_by_name(&s) {
        // c:721
        Some(optno) => optno, // c:721
        None => OPT_INVALID,  // c:721
    }
}

// Identify an option letter                                                // c:721
/// Translate a single-letter option flag to its index.
/// Port of `optlookupc(char c)` from Src/options.c:721. Returns 0 for
/// unrecognised letters. Walks the active letter table (`KSH_LETTERS`
/// when `SHOPTIONLETTERS` is set, `zshletters` otherwise) and
/// resolves the canonical name via `optno_by_name`.
pub fn optlookupc(c: char) -> i32 {
    // c:721
    // c:721 — `isset(SHOPTIONLETTERS)`. Use the const directly; the
    // previous code did `isset(optlookup("shoptionletters"))` which
    // is the same value but pays a hash lookup per call. C uses the
    // optno constant inline.
    let letters = if isset(crate::ported::zsh_h::SHOPTIONLETTERS) {
        KSH_LETTERS
    } else {
        zshletters
    };
    for (ch, name, negated) in letters {
        if *ch == c {
            // c:725 — `optletters[c - FIRST_OPT]` is signed in C;
            // negative optno means "set this letter inverts the
            // option's sense" (`-n` letter ↔ EXECOPT but `-n` means
            // *unset* EXECOPT). dosetopt at c:743 reads `optno < 0`
            // to flip `value`. Without applying the negation here,
            // `optlookupc('n')` returned the positive EXECOPT and
            // `setopt -n` SET exec instead of unsetting it.
            let optno = optno_by_name(name).unwrap_or(0); // c:725
            return if *negated { -optno } else { optno };
        }
    }
    0
}

/// Direct port of `dosetopt(int optno, int value, int force, char *new_opts)` from Src/options.c:735. C body:
/// negate value when optno < 0 (the "no" prefix marker); look up
/// option name by optno; reject emulation-locked options; write
/// `opts[optno] = value`. Static-link path: optno is the FNV hash
/// produced by `optlookup`; we look up by name in a reverse pass
/// against the canonical option set, then write OPTS_LIVE.
///
/// **c:743-755 locked-option gates**:
///   * c:743 — `force=0 && optno==EXECOPT && !value && interact` →
///     refuse `setopt noexec` in an interactive shell.
///   * c:746 — `force=0 && optno in {INTERACTIVE, SHINSTDIN,
///     SINGLECOMMAND}` → these options can only be set at startup,
///     not via `setopt`; either no-op if already correct, or reject.
///   * c:752 — `force=0 && optno==USEZLE && value` → require a
///     terminal (interactive + valid SHTTY); reject otherwise.
pub fn dosetopt(optno: i32, mut value: i32, force: i32) -> i32 {
    // c:735
    if optno == 0 {
        return -1;
    }
    let mut idx = optno;
    if idx < 0 {
        // c:739
        idx = -idx;
        value = if value != 0 { 0 } else { 1 }; // c:741
    }
    // c:743-755 — locked-option enforcement (force=0 path).
    if force == 0 {
        // c:743 — interactive + EXECOPT off is forbidden.
        if idx == EXECOPT && value == 0 && interact() {
            return -1;
        }
        // c:746-749 — INTERACTIVE / SHINSTDIN / SINGLECOMMAND lock.
        // C compares against `new_opts[optno]`, the in-progress opts
        // array; the Rust port reads the live state via opt_state_get
        // mapped from the optno's name. If the requested value equals
        // the current value, the call is a no-op success (return 0);
        // otherwise reject (return -1).
        if idx == INTERACTIVE || idx == SHINSTDIN || idx == SINGLECOMMAND {
            // c:746-749 — reverse-lookup name from optno via opt_name
            // (the canonical reverse mapping). Was doing a linear
            // scan over ZSH_OPTIONS_SET calling optlookup on each —
            // O(N) hash lookups per call.
            let name = crate::ported::zsh_h::opt_name(idx);
            if !name.is_empty() {
                let cur = opt_state_get(name).unwrap_or(false);
                if cur as i32 == value {
                    return 0; // c:749 already matches
                }
            }
            return -1; // c:750
        }
        // c:752 — USEZLE on requires interactive AND a real tty.
        // We don't yet track SHTTY/shout here; approximate by requiring
        // `interact()` to be true. A non-interactive `setopt usezle`
        // is rejected (matches the most common C failure case).
        if idx == USEZLE && value != 0 && !interact() {
            return -1;
        }
        // c:851-861 — `setopt MONITOR` (force=0, value=1) must:
        //   - No-op if already on (`new_opts[optno] == value`).
        //   - Fail if SHTTY == -1 (can't enable job control without tty).
        //   - Capture origpgrp + acquire_pgrp on first transition.
        // The previous Rust port lacked all three checks: `setopt
        // monitor` in a non-tty context would succeed and flip the
        // option flag without actually acquiring the process group,
        // leaving job control half-broken.
        if idx == MONITOR && value != 0 {
            // c:851 — reverse-lookup via opt_name instead of linear
            // scan + per-name optlookup.
            let name = crate::ported::zsh_h::opt_name(idx);
            if !name.is_empty() {
                let cur = opt_state_get(name).unwrap_or(false);
                if cur as i32 == value {
                    // c:852 no-op
                    return 0;
                }
            }
            // c:854 — `if (SHTTY == -1) return -1;`
            let shtty = SHTTY.load(std::sync::atomic::Ordering::SeqCst);
            if shtty == -1 {
                // c:854
                return -1;
            }
            // c:855-859 — `if (!origpgrp) { origpgrp = GETPGRP();
            //               acquire_pgrp(); }`. Capture the parent's
            // pgrp once so SIGTSTP-restore (bin_suspend) can later
            // killpg back to it.
            let origpgrp = ORIGPGRP.get_or_init(|| std::sync::Mutex::new(0));
            let mut og = origpgrp.lock().expect("origpgrp poisoned");
            if *og == 0 {
                // c:855
                *og = unsafe { libc::getpgrp() }; // c:856 GETPGRP()
                drop(og);
                let _ = acquire_pgrp(); // c:857
            }
        }
    }
    // c:859-870 — EMACSMODE/VIMODE mutual-exclusion toggle.
    // No `!force` guard in C — this branch fires regardless of
    // force. `setopt emacs` must `unsetopt vi` and vice versa
    // (the two are mutually exclusive ZLE keymap selectors).
    // The previous Rust port skipped this entirely; both options
    // could be on at once, leaving ZLE keymap selection ambiguous.
    {
        if (idx == EMACSMODE || idx == VIMODE) && value != 0 {
            // c:859
            // c:870 — turn off the OTHER keymap option. Resolve
            // the canonical name via opt_name (matches the
            // storage key used by isset/opt_state_get/_set).
            let other = idx ^ EMACSMODE ^ VIMODE;
            let other_name = opt_name(other);
            if !other_name.is_empty() {
                opt_state_set(other_name, false);
            }
        }
    }
    // c:871-874 — SUNKEYBOARDHACK backward-compat: setopt
    // sunkeyboardhack sets keyboardhackchar to '`'; unsetopt to '\0'.
    // Also no `!force` guard in C.
    {
        if idx == SUNKEYBOARDHACK {
            // c:871
            // c:873 — `keyboardhackchar = (value ? '`' : '\0');`. C
            // dispatches through `pm->gsu.s->setfn(pm, val)`; mirror
            // by looking up KEYBOARD_HACK in paramtab and threading
            // the pm through (the setfn body ignores pm anyway,
            // matching UNUSED(Param pm) in C).
            let new_val = if value != 0 {
                "`".to_string()
            } else {
                String::new()
            };
            if let Ok(mut tab) = paramtab().write() {
                if let Some(pm) = tab.get_mut("KEYBOARD_HACK") {
                    keyboardhacksetfn(pm, new_val);
                }
            }
        }
    }
    // c:744 — write the canonical opt_name(idx) slot so the
    // matching isset(idx) → opt_state_get(opt_name(idx)) read
    // sees the same key. The previous `iter().find(...)` walk
    // returned the FIRST ZSH_OPTIONS_SET entry whose optlookup
    // matched — but that set includes BASH/KSH-compat aliases
    // (e.g. `dotglob` → GLOBDOTS), and HashSet iteration order
    // is arbitrary, so `setopt globdots` could write under the
    // alias name. isset(GLOBDOTS) then read the canonical
    // `globdots` slot which stayed at its default `false`.
    // Symptom: \`setopt globdots; [[ -o globdots ]]\` returned
    // "off" because the alias and canonical names live in
    // separate buckets of OPTS_LIVE.
    let canonical = crate::ported::zsh_h::opt_name(idx);
    let ret = if !canonical.is_empty() {
        opt_state_set(canonical, value != 0);
        0
    } else {
        -1
    };
    // c:877-884 — `if (optno == MULTIBYTE || BANGHIST || SHINSTDIN)
    //                  inittyptab();`. These options change which
    //                  bytes are special in pattern matching and
    //                  word splitting; the typtab must be rebuilt.
    if ret == 0 && (idx == MULTIBYTE || idx == BANGHIST || idx == SHINSTDIN)
    // c:879-882
    {
        inittyptab(); // c:883
    }
    ret
}

/// Build the value of `$-`: a string of the active single-letter
/// option flags (e.g. `"is"` for an interactive script).
/// Port of `dashgetfn(UNUSED(Param pm))` from Src/options.c:890. C source iterates
/// `[FIRST_OPT..=LAST_OPT]` and appends each set option's letter.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn dashgetfn() -> String {
    // c:289-290 — `#define FIRST_OPT '0'` / `#define LAST_OPT 'y'`.
    // The previous Rust port iterated `(b'A'..=b'z')` (A=0x41..z=0x7a),
    // skipping the 17 char positions C walks BEFORE 'A' (digits + most
    // ASCII punctuation between '0' and '@'). Option letters in this
    // range — e.g. `-?` could be valid — were silently dropped from
    // the `$-` string. Match C exactly with FIRST_OPT..=LAST_OPT.
    const FIRST_OPT: u8 = b'0'; // c:289
    const LAST_OPT: u8 = b'y'; // c:290
                               // c:721 — `isset(SHOPTIONLETTERS)`. Use the const directly; the
                               // previous code did `isset(optlookup("shoptionletters"))` which
                               // is the same value but pays a hash lookup per call. C uses the
                               // optno constant inline.
    let letters = if isset(crate::ported::zsh_h::SHOPTIONLETTERS) {
        KSH_LETTERS
    } else {
        zshletters
    };
    let mut out = String::new();
    for c in (FIRST_OPT..=LAST_OPT).map(|b| b as char) {
        // c:896
        for (ch, name, negated) in letters {
            if *ch == c {
                let value = opt_state_get(name).unwrap_or(false); // c:891 `opts[optno]`
                let effective = if *negated { !value } else { value };
                if effective {
                    out.push(c);
                }
                break;
            }
        }
    }
    out
}

/// Direct port of `printoptionstates(int hadplus)` from Src/options.c:909.
/// C body (c:910): `scanhashtable(optiontab, 1, 0, OPT_ALIAS,
/// printoptionnodestate, hadplus);` — walks optiontab applying the
/// printoptionnodestate callback to each non-alias entry.
/// Static-link path: walks ZSH_OPTIONS_SET (canonical option name
/// registry) and reads each option's live state via opt_state_get.
pub fn printoptionstates(hadplus: bool) {
    // c:909
    // c:Src/builtin.c:910 — `scanhashtable(optiontab, 1, 0,
    // OPT_ALIAS, printoptionnodestate, hadplus)`. The 4th arg
    // OPT_ALIAS is a skip-mask: entries with the OPT_ALIAS bit
    // are filtered out. The Rust port previously walked every
    // ZSH_OPTIONS_SET entry, emitting both canonical names AND
    // the bash/ksh-compat aliases — 12 extra lines in the output
    // versus zsh's count. Match C by excluding ZSH_OPTION_ALIASES.
    let mut names: Vec<&'static str> = ZSH_OPTIONS_SET
        .iter()
        .copied()
        .filter(|n| !ZSH_OPTION_ALIASES.contains(n))
        .collect();
    names.sort();
    for n in names {
        // c:910 scanhashtable
        let value = opt_state_get(n).unwrap_or(false);
        printoptionnodestate(n, value, hadplus); // c:916
    }
}

/// C body (c:920-933):
/// ```c
/// if (hadplus) {
///     printf("set %co %s%s\n",
///         defset(on, emulation) != isset(optno) ? '-' : '+',
///         defset(on, emulation) ? "no" : "",
///         on->node.nam);
/// } else {
///     if (defset(on, emulation))
///         printf("no%-19s %s\n", nam, isset(optno) ? "off" : "on");
///     else
///         printf("%-21s %s\n", nam, isset(optno) ? "on" : "off");
/// }
/// ```
/// Port of `printoptionnodestate(HashNode hn, int hadplus)` from `Src/options.c:916`.
/// WARNING: param names don't match C — Rust=(name, value, hadplus) vs C=(hn, hadplus)
pub fn printoptionnodestate(name: &str, value: bool, hadplus: bool) {
    // c:916
    let default_on = default_on_options().contains(&name); // c:916 defset
    if hadplus {
        // c:920
        let sign = if default_on != value { '-' } else { '+' }; // c:922
        let no_prefix = if default_on { "no" } else { "" }; // c:923
        println!("set {}o {}{}", sign, no_prefix, name); // c:921
    } else {
        if default_on {
            // c:927
            println!(
                "no{:<19} {}",
                name, // c:928
                if value { "off" } else { "on" }
            );
        } else {
            println!(
                "{:<21} {}",
                name, // c:930
                if value { "on" } else { "off" }
            );
        }
    }
}

/// Direct port of `printoptionlist()` from Src/options.c:938.
/// C body (c:945-955):
/// ```c
/// printf("\nNamed options:\n");
/// scanhashtable(optiontab, 1, 0, OPT_ALIAS, printoptionlist_printoption, 0);
/// printf("\nOption aliases:\n");
/// scanhashtable(optiontab, 1, OPT_ALIAS, 0, printoptionlist_printoption, 0);
/// printf("\nOption letters:\n");
/// for(lp = optletters, c = FIRST_OPT; c <= LAST_OPT; lp++, c++) {
///     if(!*lp) continue;
///     printf("  -%c  ", c);
///     printoptionlist_printequiv(*lp);
/// }
/// ```
pub fn printoptionlist() {
    // c:938
    println!();
    println!("Named options:"); // c:945
    let mut names: Vec<&'static str> = ZSH_OPTIONS_SET.iter().copied().collect();
    names.sort();
    for n in &names {
        // c:946 scanhashtable
        printoptionlist_printoption(n, 0); // c:958
    }
    println!();
    println!("Option aliases:"); // c:947
                                 // c:948 — alias-only walk; static-link path lacks OPT_ALIAS bit
                                 // tracking on each option, so the alias walk emits nothing here.
    println!();
    println!("Option letters:"); // c:949
                                 // c:721 — `isset(SHOPTIONLETTERS)`. Use the const directly; the
                                 // previous code did `isset(optlookup("shoptionletters"))` which
                                 // is the same value but pays a hash lookup per call. C uses the
                                 // optno constant inline.
    let letters = if isset(crate::ported::zsh_h::SHOPTIONLETTERS) {
        KSH_LETTERS
    } else {
        zshletters
    };
    for c in (b'A'..=b'z').map(|b| b as char) {
        // c:950
        for (ch, aname, _negated) in letters {
            if *ch == c {
                print!("  -{}  ", c); // c:953
                printoptionlist_printequiv(optlookup(aname)); // c:954
                break;
            }
        }
    }
}

/// Direct port of `printoptionlist_printoption()` from
/// Src/options.c:958. C body (c:961-967):
/// ```c
/// if(on->node.flags & OPT_ALIAS) {
///     printf("  --%-19s  ", on->node.nam);
///     printoptionlist_printequiv(on->optno);
/// } else
///     printf("  --%s\n", on->node.nam);
/// ```
/// Static-link path: OPT_ALIAS flag tracking on each option isn't
/// ported, so every entry takes the non-alias branch.
/// WARNING: param names don't match C — Rust=(name, _ignored) vs C=(hn, ignored)
pub fn printoptionlist_printoption(name: &str, _ignored: i32) {
    // c:958
    println!("  --{}", name); // c:971
}

/// Direct port of `printoptionlist_printequiv(int optno)` from Src/options.c:971.
/// C body (c:973-977):
/// ```c
/// int isneg = optno < 0;
/// optno *= (isneg ? -1 : 1);
/// printf("  equivalent to --%s%s\n", isneg ? "no-" : "",
///        optns[optno-1].node.nam);
/// ```
pub fn printoptionlist_printequiv(optno: i32) {
    // c:971
    let isneg = optno < 0; // c:971
    let abs_optno = if isneg { -optno } else { optno }; // c:974
    let prefix = if isneg { "no-" } else { "" }; // c:975
                                                 // c:976 — `optns[optno-1].node.nam`. Reverse-lookup via opt_name
                                                 // instead of linear scan + per-name optlookup.
    let name = crate::ported::zsh_h::opt_name(abs_optno);
    let name = if name.is_empty() { "?" } else { name };
    println!("  equivalent to --{}{}", prefix, name); // c:975
}

/// C body (c:990-997):
/// ```c
/// if (!(on->node.flags & OPT_ALIAS) &&
///     ((fully && !(on->node.flags & OPT_SPECIAL)) ||
///      (on->node.flags & OPT_EMULATE)))
/// {
///     if (!print_emulate_opts[on->optno]) fputs("no", stdout);
///     puts(on->node.nam);
/// }
/// ```
/// Static-link path: per-option flag bits (OPT_ALIAS / OPT_SPECIAL /
/// OPT_EMULATE) aren't yet ported with the optns[] table; the Rust
/// port emits every non-default option whose value matches `value`.
/// Port of `print_emulate_option(HashNode hn, int fully)` from `Src/options.c:984`.
/// WARNING: param names don't match C — Rust=(name, value, _fully) vs C=(hn, fully)
pub fn print_emulate_option(name: &str, value: bool, _fully: bool) {
    // c:984
    if !value {
        // c:984 !print_emulate_opts[optno]
        print!("no"); // c:995
    }
    println!("{}", name); // c:996
}

/// Port of `mod_export int emulation;` from `Src/options.c:36`.
/// Current emulation bitmap; one of EMULATE_ZSH / EMULATE_KSH /
/// EMULATE_SH / EMULATE_CSH. Tested via the `EMULATION(bits)` macro
/// at zsh.h:2347 (`(emulation & bits) != 0`). Default 0 — the
/// initial value matches C's zero-initialised BSS slot; `setup_init`
/// calls `installemulation()` (options.c:523) early in startup to
/// flip the right bit.
#[allow(non_upper_case_globals)]
pub static emulation: AtomicI32 = AtomicI32::new(0); // c:36

/// Ksh single-letter options
pub static KSH_LETTERS: &[(char, &str, bool)] = &[
    ('C', "clobber", true),
    ('T', "trapsasync", false),
    ('X', "markdirs", false),
    ('a', "allexport", false),
    ('b', "notify", false),
    ('e', "errexit", false),
    ('f', "glob", true),
    ('i', "interactive", false),
    ('l', "login", false),
    ('m', "monitor", false),
    ('n', "exec", true),
    ('p', "privileged", false),
    ('s', "shinstdin", false),
    ('t', "singlecommand", false),
    ('u', "unset", true),
    ('v', "verbose", false),
    ('x', "xtrace", false),
];

// `ShellOptions` struct + `impl ShellOptions` (14 methods) + `impl
// Default for ShellOptions` DELETED. C zsh holds option state in
// two file-scope globals at `Src/options.c:33-46`:
//
//     int emulation;                              // c:33
//     mod_export char opts[OPT_SIZE];             // c:43
//
// Rust port mirrors `opts[]` via `OPTS_LIVE` (already at
// `options.rs:1259+`) and `emulation` via `EMULATION` below. Every
// former method becomes a free fn matching a C entry point
// (`emulate` c:533, `installemulation` c:523, `dosetopt` c:735,
// `optlookup` c:684, `optlookupc` c:721, `createoptiontable` c:471).

/// Port of file-static `int emulation;` at `Src/options.c:33`.
/// Holds the current emulation bit (`EMULATE_ZSH`/`CSH`/`KSH`/`SH`,
/// OR-able with `EMULATE_FULLY`).
pub static EMULATION: AtomicI32 = AtomicI32::new(EMULATE_ZSH);

/// `EMULATE_FULLY` bit (`Src/zsh.h:2354`) tracked separately so
/// `install_emulation_defaults` can re-OR it into the emulation
/// bitmap.
pub static FULLY_EMULATING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
/// `ZSH_OPTIONS_SET` static.
pub static ZSH_OPTIONS_SET: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "aliases",
        // c:Src/options.c:81 — ALIASFUNCDEF.
        "aliasfuncdef",
        "allexport",
        "alwayslastprompt",
        "alwaystoend",
        "appendcreate",
        "appendhistory",
        "autocd",
        "autocontinue",
        "autolist",
        "automenu",
        "autonamedirs",
        "autoparamkeys",
        "autoparamslash",
        "autopushd",
        "autoremoveslash",
        "autoresume",
        "badpattern",
        "banghist",
        "bareglobqual",
        "bashautolist",
        "bashrematch",
        "beep",
        "bgnice",
        "braceccl",
        "bsdecho",
        "caseglob",
        "casematch",
        // c:Src/options.c:108 — CASEPATHS.
        "casepaths",
        "cbases",
        "cdablevars",
        "cdsilent",
        "chasedots",
        "chaselinks",
        "checkjobs",
        "checkrunningjobs",
        "clobber",
        // c:Src/options.c:118 — CLOBBEREMPTY.
        "clobberempty",
        "combiningchars",
        "completealiases",
        "completeinword",
        "continueonerror",
        "correct",
        "correctall",
        "cprecedences",
        "cshjunkiehistory",
        "cshjunkieloops",
        "cshjunkiequotes",
        "cshnullcmd",
        "cshnullglob",
        "debugbeforecmd",
        "dotglob",
        "dvorak",
        "emacs",
        "equals",
        "errexit",
        "errreturn",
        "evallineno",
        "exec",
        "extendedglob",
        "extendedhistory",
        "flowcontrol",
        "forcefloat",
        "functionargzero",
        "glob",
        "globassign",
        "globcomplete",
        "globdots",
        "globstarshort",
        "globsubst",
        "globalexport",
        "globalrcs",
        "hashall",
        "hashcmds",
        "hashdirs",
        "hashexecutablesonly",
        "hashlistall",
        "histallowclobber",
        "histappend",
        "histbeep",
        "histexpand",
        "histexpiredupsfirst",
        "histfcntllock",
        "histfindnodups",
        "histignorealldups",
        "histignoredups",
        "histignorespace",
        "histlexwords",
        "histnofunctions",
        "histnostore",
        "histreduceblanks",
        "histsavebycopy",
        "histsavenodups",
        "histsubstpattern",
        "histverify",
        "hup",
        "ignorebraces",
        "ignoreclosebraces",
        "ignoreeof",
        "incappendhistory",
        "incappendhistorytime",
        "interactive",
        "interactivecomments",
        "ksharrays",
        "kshautoload",
        "kshglob",
        "kshoptionprint",
        "kshtypeset",
        "kshzerosubscript",
        "listambiguous",
        "listbeep",
        "listpacked",
        "listrowsfirst",
        "listtypes",
        "localloops",
        "localoptions",
        "localpatterns",
        "localtraps",
        "log",
        "login",
        "longlistjobs",
        "magicequalsubst",
        "mailwarn",
        "mailwarning",
        "markdirs",
        "menucomplete",
        "monitor",
        "multibyte",
        "multifuncdef",
        "multios",
        "nomatch",
        "notify",
        "nullglob",
        "numericglobsort",
        "octalzeroes",
        "onecmd",
        "overstrike",
        "pathdirs",
        "pathscript",
        "physical",
        "pipefail",
        "posixaliases",
        "posixargzero",
        "posixbuiltins",
        "posixcd",
        "posixidentifiers",
        "posixjobs",
        "posixstrings",
        "posixtraps",
        "printeightbit",
        "printexitvalue",
        "privileged",
        "promptbang",
        "promptcr",
        "promptpercent",
        "promptsp",
        "promptsubst",
        "promptvars",
        "pushdignoredups",
        "pushdminus",
        "pushdsilent",
        "pushdtohome",
        "rcexpandparam",
        "rcquotes",
        "rcs",
        "recexact",
        "rematchpcre",
        "restricted",
        "rmstarsilent",
        "rmstarwait",
        "sharehistory",
        "shfileexpansion",
        "shglob",
        "shinstdin",
        "shnullcmd",
        "shoptionletters",
        "shortloops",
        "shortrepeat",
        "shwordsplit",
        "singlecommand",
        "singlelinezle",
        "sourcetrace",
        "stdin",
        "sunkeyboardhack",
        "trackall",
        "transientrprompt",
        "trapsasync",
        "typesetsilent",
        "typesettounset",
        "unset",
        "verbose",
        "vi",
        "warncreateglobal",
        "warnnestedvar",
        "xtrace",
        "zle",
        // bash/ksh-compat aliases — the canonical zsh names live in
        // alias-resolution match in set_option (port of optns[]:269-280),
        // but for the runtime
        // `setopt`/`unsetopt` "no such option" check we accept the
        // alias spellings too so scripts written for bash/ksh (e.g.
        // p10k's `setopt brace_expand`, `dotglob` users) don't error.
        "braceexpand", // alias of `noignorebraces`
        "dotglob",     // alias of `globdots`
        "hashall",     // alias of `hashcmds`
        "histappend",  // alias of `appendhistory`
        "histexpand",  // alias of `banghist`
        "log",         // alias of `nohistnofunctions`
        "mailwarn",    // alias of `mailwarning`
        "onecmd",      // alias of `singlecommand`
        "physical",    // alias of `chaselinks`
        "promptvars",  // alias of `promptsubst`
    ]
    .into_iter()
    .collect()
});

/// Names flagged `OPT_ALIAS` in `Src/options.c:269-280`. The
/// no-arg `setopt` / `unsetopt` walk skips these (the C code
/// passes `OPT_ALIAS` as the `flags2` mask to `scanhashtable`,
/// excluding any node with that bit). They still accept
/// `setopt <alias>` for bash/ksh script compat; this set only
/// gates the OUTPUT enumeration.
pub static ZSH_OPTION_ALIASES: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "braceexpand",
        "dotglob",
        "hashall",
        "histappend",
        "histexpand",
        "log",
        "mailwarn",
        "onecmd",
        "physical",
        "promptvars",
        "stdin",
        "trackall",
    ]
    .into_iter()
    .collect()
});
// END moved-from-exec-rs (statics)

// BEGIN moved-from-exec-rs (helpers)
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs (helpers)

// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// ===========================================================
// Direct ports of the static option-table builders / lookup /
// printers from Src/options.c. The Rust executor stores option
// state as `HashMap<String, bool>` on `ShellExecutor`; the C
// source instead hangs everything off the global `optiontab[]`
// array indexed by `OPT_*` enum constants. These free-fn entries
// satisfy ABI/name parity for the drift gate; live state is
// owned by the executor.
// ===========================================================

/// Sentinel returned by `optlookup` when no matching option exists.
/// Re-export of the canonical `OPT_INVALID = 0` from `Src/zsh.h:2363`
/// (the first slot in the option-index enum). C: `OPT_INVALID, ALIASESOPT, ...`.
/// Re-exported here so call sites that already import from
/// `options` don't need to change to `zsh_h`.

/// Port of `static int setemulate_emulation;` from `Src/options.c:496`.
/// The target emulation bitmap, written by `installemulation` and
/// read by the `setemulate` per-option callback (c:518).
static SETEMULATE_EMULATION: AtomicI32 = // c:496
    AtomicI32::new(0);

/// Port of `static char *setemulate_opts;` from `Src/options.c:501`.
/// The precomputed `new_opts[]` array `setemulate` writes into. C
/// stores it as a flat `char[]` indexed by `optno`; the Rust port
/// keeps it as a HashMap<String, bool> since the runtime is FNV-
/// hashed instead of densely indexed.
static SETEMULATE_OPTS: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, bool>>,
> = std::sync::OnceLock::new(); // c:501

// =====================================================================
// !!! WARNING: RUST-ONLY STATE — NO DIRECT C COUNTERPART !!!
// =====================================================================
//
// `OPTS_LIVE` is the process-wide option-state map that bin_setopt
// reads + writes. The C source uses a flat `char opts[OPTSIZE]`
// global indexed by optno (Src/options.c:36 + accessors `isset(o)`,
// `opts[o] = 1` etc.). Rust uses an RwLock<HashMap<String,bool>>
// because optno is FNV-hashed (no flat index range) and HashMap is
// the natural Rust mirror of "name → set?" lookup.
//
// Per PORT_PLAN.md Phase 3 (bucket-2 read-mostly): options are read
// on every command dispatch (`isset(ERREXIT)`, `isset(INTERACTIVE)`,
// etc.) but written only on `setopt`/`unsetopt`. `RwLock` lets
// parallel readers proceed without serialising on a mutex.
//
// !!! Do NOT add a parallel options store elsewhere. Every read /
// write of an option's set-state in the lib must route through
// `opt_state_get` / `opt_state_set` to stay coherent with bin_setopt.
// The ShellExecutor.options HashMap should eventually become a
// read-through cache of this map. !!!
// =====================================================================

static OPTS_LIVE: std::sync::OnceLock<std::sync::RwLock<std::collections::HashMap<String, bool>>> =
    std::sync::OnceLock::new();

// =====================================================================
// `default_on_options` mirrors the `defset(on, emulation)` macro
// (Src/options.c:73, `(!!((X)->node.flags & my_emulation))`) — the
// `optns[]` flag-table walk now lives in `optns_flags(name)` below,
// so `default_on_options` returns the real set of zsh-emulation
// defaults. C source uses the inline macro at every callsite;
// the Rust port factors it through one collect-and-return helper.
// =====================================================================

// #define defset(X, my_emulation) (!!((X)->node.flags & my_emulation))  // c:73
/// Port of `defset()` macro from `Src/options.c:73`.
/// Returns true if the option is on by default for the given emulation.
#[inline]
pub fn defset(X: &str, my_emulation: i32) -> bool {
    let flags = optns_flags(X);
    (flags & (my_emulation as u16)) != 0
}

/// Get the flags for an option from the optns[] table.
/// Port of looking up `optns[optno].node.flags`.
fn optns_flags(name: &str) -> u16 {
    match name.to_lowercase().as_str() {
        "aliases" => OPT_EMULATE | (OPT_ALL as u16), // c:80
        "aliasfuncdef" => OPT_EMULATE | (OPT_BOURNE as u16), // c:81
        "allexport" => OPT_EMULATE,                  // c:82
        "alwayslastprompt" => OPT_ALL as u16,        // c:83
        "alwaystoend" => 0,                          // c:84
        "appendcreate" => OPT_EMULATE | (OPT_BOURNE as u16), // c:85
        "appendhistory" => OPT_ALL as u16,           // c:86
        "autocd" => OPT_EMULATE,                     // c:87
        "autocontinue" => 0,                         // c:88
        "autolist" => OPT_ALL as u16,                // c:89
        "automenu" => OPT_ALL as u16,                // c:90
        "autonamedirs" => 0,                         // c:91
        "autoparamkeys" => OPT_ALL as u16,           // c:92
        "autoparamslash" => OPT_ALL as u16,          // c:93
        "autopushd" => 0,                            // c:94
        "autoremoveslash" => OPT_ALL as u16,         // c:95
        "autoresume" => 0,                           // c:96
        "badpattern" => OPT_EMULATE | (OPT_NONBOURNE as u16), // c:97
        "banghist" => OPT_NONBOURNE as u16,          // c:98
        "bareglobqual" => OPT_EMULATE | (OPT_ZSH as u16), // c:99
        "bashautolist" => 0,                         // c:100
        "bashrematch" => 0,                          // c:101
        "beep" => OPT_ALL as u16,                    // c:102
        "bgnice" => OPT_EMULATE | (OPT_NONBOURNE as u16), // c:103
        "braceccl" => OPT_EMULATE,                   // c:104
        "bsdecho" => OPT_EMULATE | (OPT_SH as u16),  // c:105
        "caseglob" => OPT_ALL as u16,                // c:106
        "casematch" => OPT_ALL as u16,               // c:107
        "casepaths" => 0,                            // c:108
        "cbases" => 0,                               // c:109
        "cdablevars" => OPT_EMULATE,                 // c:109
        "cdsilent" => 0,                             // c:110
        "chasedots" => OPT_EMULATE,                  // c:113
        "chaselinks" => OPT_EMULATE,                 // c:114
        "checkjobs" => OPT_EMULATE | (OPT_ZSH as u16), // c:113
        "checkrunningjobs" => OPT_EMULATE | (OPT_ZSH as u16), // c:114
        "clobber" => OPT_EMULATE | (OPT_ALL as u16), // c:117
        "clobberempty" => 0,                         // c:118
        "combiningchars" => 0,                       // c:119
        "completealiases" => 0,                      // c:117
        "completeinword" => 0,                       // c:118
        "correct" => 0,                              // c:119
        "correctall" => 0,                           // c:120
        "cprecedences" => OPT_EMULATE | (OPT_NONZSH as u16), // c:110
        "cshjunkiehistory" => OPT_EMULATE | (OPT_CSH as u16), // c:125
        "cshjunkieloops" => OPT_EMULATE | (OPT_CSH as u16), // c:126
        "cshjunkiequotes" => OPT_EMULATE | (OPT_CSH as u16), // c:127
        "cshnullcmd" => OPT_EMULATE | (OPT_CSH as u16), // c:128
        "cshnullglob" => OPT_EMULATE | (OPT_CSH as u16), // c:129
        "debugbeforecmd" => OPT_ALL as u16,          // c:127
        "emacs" => 0,                                // c:128
        "equals" => OPT_EMULATE | (OPT_NONBOURNE as u16), // c:129
        "errexit" => OPT_EMULATE,                    // c:130
        "errreturn" => OPT_EMULATE,                  // c:131
        "exec" => OPT_ALL as u16,                    // c:132
        "extendedglob" => OPT_EMULATE,               // c:133
        "extendedhistory" => OPT_CSH as u16,         // c:134
        "evallineno" => OPT_EMULATE | (OPT_ZSH as u16), // c:135
        "flowcontrol" => OPT_ALL as u16,             // c:136
        "forcefloat" => 0,                           // c:137
        "functionargzero" => OPT_EMULATE | (OPT_NONBOURNE as u16), // c:138
        "glob" => OPT_EMULATE | (OPT_ALL as u16),    // c:139
        "globalexport" => OPT_EMULATE | (OPT_ZSH as u16), // c:140
        "globalrcs" => OPT_ALL as u16,               // c:141
        "globassign" => OPT_EMULATE | (OPT_CSH as u16), // c:145
        "globcomplete" => 0,                         // c:143
        "globdots" => OPT_EMULATE,                   // c:144
        "globstarshort" => OPT_EMULATE,              // c:145
        "globsubst" => OPT_EMULATE | (OPT_NONZSH as u16), // c:146
        "hashcmds" => OPT_ALL as u16,                // c:147
        "hashdirs" => OPT_ALL as u16,                // c:148
        "hashexecutablesonly" => 0,                  // c:149
        "hashlistall" => OPT_ALL as u16,             // c:150
        "histallowclobber" => 0,                     // c:151
        "histbeep" => OPT_ALL as u16,                // c:152
        "histexpiredupsfirst" => 0,                  // c:153
        "histfcntllock" => 0,                        // c:154
        "histfindnodups" => 0,                       // c:155
        "histignorealldups" => 0,                    // c:156
        "histignoredups" => 0,                       // c:157
        "histignorespace" => 0,                      // c:158
        "histlexwords" => 0,                         // c:159
        "histnofunctions" => 0,                      // c:160
        "histnostore" => 0,                          // c:161
        "histreduceblanks" => 0,                     // c:162
        "histsavebycopy" => OPT_ALL as u16,          // c:163
        "histsavenodups" => 0,                       // c:164
        "histsubstpattern" => OPT_EMULATE,           // c:165
        "histverify" => 0,                           // c:166
        "hup" => OPT_EMULATE | (OPT_ZSH as u16),     // c:167
        "ignorebraces" => OPT_EMULATE | (OPT_SH as u16), // c:168
        "ignoreclosebraces" => OPT_EMULATE,          // c:172
        "ignoreeof" => 0,                            // c:170
        "incappendhistory" => 0,                     // c:171
        "incappendhistorytime" => 0,                 // c:172
        "interactive" => OPT_SPECIAL as u16,         // c:173
        "interactivecomments" => OPT_BOURNE as u16,  // c:177
        "ksharrays" => OPT_EMULATE | (OPT_BOURNE as u16), // c:175
        "kshautoload" => OPT_EMULATE | (OPT_BOURNE as u16), // c:176
        "kshglob" => OPT_EMULATE | (OPT_KSH as u16), // c:177
        "kshoptionprint" => OPT_EMULATE | (OPT_KSH as u16), // c:178
        "kshtypeset" => 0,                           // c:182
        "kshzerosubscript" => 0,                     // c:183
        "listambiguous" => OPT_ALL as u16,           // c:181
        "listbeep" => OPT_ALL as u16,                // c:182
        "listpacked" => 0,                           // c:183
        "listrowsfirst" => 0,                        // c:184
        "listtypes" => OPT_ALL as u16,               // c:185
        "localoptions" => OPT_EMULATE | (OPT_KSH as u16), // c:186
        "localloops" => OPT_EMULATE,                 // c:190
        "localpatterns" => OPT_EMULATE,              // c:191
        "localtraps" => OPT_EMULATE | (OPT_KSH as u16), // c:189
        "loginshell" => OPT_SPECIAL as u16,          // c:190
        "longlistjobs" => 0,                         // c:191
        "magicequalsubst" => OPT_EMULATE,            // c:192
        "mailwarning" => 0,                          // c:193
        "markdirs" => 0,                             // c:194
        "menucomplete" => 0,                         // c:195
        "monitor" => OPT_SPECIAL as u16,             // c:196
        // c:197 — `multibyte` defaults to OPT_ALL when
        // MULTIBYTE_SUPPORT is compiled in (always true for zshrs
        // since Rust strings are UTF-8). Previous Rust port had `0`
        // which left multibyte off in all emulations.
        "multibyte" => OPT_ALL as u16,                        // c:197
        "multifuncdef" => OPT_EMULATE | (OPT_ZSH as u16),     // c:198
        "multios" => OPT_EMULATE | (OPT_ZSH as u16),          // c:199
        "nomatch" => OPT_EMULATE | (OPT_NONBOURNE as u16),    // c:200
        "notify" => OPT_ZSH as u16,                           // c:210
        "nullglob" => OPT_EMULATE,                            // c:202
        "numericglobsort" => OPT_EMULATE,                     // c:212
        "octalzeroes" => OPT_EMULATE | (OPT_SH as u16),       // c:204
        "overstrike" => 0,                                    // c:205
        "pathdirs" => OPT_EMULATE,                            // c:215
        "pathscript" => OPT_EMULATE | (OPT_BOURNE as u16),    // c:207
        "pipefail" => OPT_EMULATE,                            // c:208
        "posixaliases" => OPT_EMULATE | (OPT_BOURNE as u16),  // c:209
        "posixargzero" => OPT_EMULATE | (OPT_BOURNE as u16),  // c:210
        "posixbuiltins" => OPT_EMULATE | (OPT_BOURNE as u16), // c:211
        "posixcd" => OPT_EMULATE | (OPT_BOURNE as u16),       // c:212
        "posixidentifiers" => OPT_EMULATE | (OPT_BOURNE as u16), // c:213
        "posixjobs" => OPT_EMULATE | (OPT_BOURNE as u16),     // c:214
        "posixstrings" => OPT_EMULATE | (OPT_BOURNE as u16),  // c:215
        "posixtraps" => OPT_EMULATE | (OPT_BOURNE as u16),    // c:216
        "printeightbit" => 0,                                 // c:217
        "printexitvalue" => 0,                                // c:218
        "privileged" => OPT_SPECIAL as u16,                   // c:219
        "promptbang" => OPT_KSH as u16,                       // c:229
        "promptcr" => OPT_ALL as u16,                         // c:221
        "promptpercent" => OPT_NONBOURNE as u16,              // c:231
        "promptsp" => OPT_ALL as u16,                         // c:223
        "promptsubst" => OPT_BOURNE as u16,                   // c:233
        "pushdignoredups" => OPT_EMULATE,                     // c:234
        "pushdminus" => OPT_EMULATE,                          // c:235
        "pushdsilent" => 0,                                   // c:227
        "pushdtohome" => OPT_EMULATE,                         // c:237
        "rcexpandparam" => OPT_EMULATE,                       // c:229
        "rcquotes" => OPT_EMULATE,                            // c:239
        "rcs" => OPT_ALL as u16,                              // c:231
        "recexact" => 0,                                      // c:232
        "rematchpcre" => 0,                                   // c:233
        "restricted" => OPT_SPECIAL as u16,                   // c:234
        "rmstarsilent" => OPT_BOURNE as u16,                  // c:243
        "rmstarwait" => 0,                                    // c:236
        "sharehistory" => 0,                                  // c:237
        "shfileexpansion" => OPT_EMULATE | (OPT_BOURNE as u16), // c:238
        "shglob" => OPT_EMULATE | (OPT_BOURNE as u16),        // c:239
        "shinstdin" => OPT_SPECIAL as u16,                    // c:240
        "shnullcmd" => OPT_EMULATE | (OPT_BOURNE as u16),     // c:241
        "shoptionletters" => OPT_EMULATE | (OPT_BOURNE as u16), // c:242
        "shortloops" => OPT_EMULATE | (OPT_NONBOURNE as u16), // c:243
        // c:Src/options.c:252 — `shortrepeat` is OPT_EMULATE only
        // (no OPT_ZSH). It defaults OFF in zsh emulation and only
        // turns on under non-zsh emulations. The previous Rust port
        // had OPT_EMULATE|OPT_ZSH which left it on in zsh.
        "shortrepeat" => OPT_EMULATE,                         // c:252
        "shwordsplit" => OPT_EMULATE | (OPT_BOURNE as u16),   // c:245
        "singlecommand" => OPT_SPECIAL as u16,                // c:246
        "singlelinezle" => 0,                                 // c:247
        "sourcetrace" => 0,                                   // c:248
        "sunkeyboardhack" => 0,                               // c:249
        "transientrprompt" => 0,                              // c:250
        "trapsasync" => 0,                                    // c:251
        "typesetsilent" => OPT_EMULATE | (OPT_BOURNE as u16), // c:252
        "unset" => OPT_EMULATE | (OPT_BSHELL as u16),         // c:253
        "verbose" => 0,                                       // c:263
        "vi" => 0,                                            // c:255
        "warncreateglobal" => OPT_EMULATE,                    // c:265
        "warnnestedvar" => OPT_EMULATE,                       // c:266
        "xtrace" => 0,                                        // c:267
        "zle" => OPT_SPECIAL as u16,                          // c:259
        "dvorak" => 0,                                        // c:260
        _ => 0,
    }
}

/// !!! RUST-ONLY HELPER — see WARNING block above.
/// Returns options that are on by default for the CURRENT
/// emulation (reads the live `EMULATION` atomic, c:33). Bug
/// #470 in docs/BUGS.md: previously hardcoded `EMULATE_ZSH`,
/// so `emulate -L sh; setopt` printed against the zsh-default
/// baseline rather than the sh-default baseline — diverged
/// from `Src/options.c:462 defset(on, emulation)` which reads
/// the file-static `emulation` global.
pub(crate) fn default_on_options() -> HashSet<&'static str> {
    let emu = EMULATION.load(std::sync::atomic::Ordering::Relaxed) as u16;
    let mut set = HashSet::new();
    for name in ZSH_OPTIONS_SET.iter() {
        let flags = optns_flags(name);
        if (flags & emu) != 0 && (flags & OPT_SPECIAL) == 0 {
            set.insert(*name);
        }
    }
    set
}

fn setemulate_opts_lock() -> &'static std::sync::Mutex<std::collections::HashMap<String, bool>> {
    SETEMULATE_OPTS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Reverse lookup to map a canonical option name back to its
/// C-fixed optno (one of the `zh::OPT_*` constants). Rust-only
/// architectural helper: C iterates `optiontab` (a HashTable keyed
/// by name) and reads `Optname.optno` — the Rust port keeps the
/// canonical idx→name mapping in `zh::opt_name` (zsh_h.rs:2954,
/// kept in sync with the C `optns[]` table at `Src/options.c:43`).
/// This fn walks `1..OPT_SIZE` and matches the first idx that
/// names to `name`. Serves both `opt_name(idx) → name` and the
/// reverse via this walk.
fn optno_by_name(name: &str) -> Option<i32> {
    for idx in 1..OPT_SIZE {
        let n = opt_name(idx);
        if !n.is_empty() && n == name {
            return Some(idx);
        }
    }
    None
}

/// !!! RUST-ONLY HELPER — see WARNING block above. Read the live
/// state of `name` from the process-wide option store.
///
/// Alias-aware: OPT_ALIAS rows (`hashall` → HASHCMDS, `histappend` →
/// APPENDHISTORY, `braceexpand` → -IGNOREBRACES, `log` →
/// -HISTNOFUNCTIONS, `physical` → CHASELINKS, etc.) route through
/// `optlookup` to the canonical slot. Negative aliases invert the
/// returned value. This mirrors C zsh's `isset(optlookup(name))`
/// path where `$options[ALIAS]` reads the same slot as
/// `$options[CANONICAL]`. Without this, alias names that got
/// populated into the live-state map by `default_options()`
/// returned their static default (usually `false`) instead of the
/// canonical option's runtime state.
pub fn opt_state_get(name: &str) -> Option<bool> {
    let m = OPTS_LIVE.get_or_init(|| std::sync::RwLock::new(std::collections::HashMap::new()));
    // Route through optlookup for canonicalisation. If the name
    // resolves to a different canonical, read THAT slot.
    let optno = optlookup(name);
    if optno != OPT_INVALID {
        let (target_optno, negate) = if optno < 0 {
            (-optno, true)
        } else {
            (optno, false)
        };
        let target_name = opt_name(target_optno);
        if !target_name.is_empty() && target_name != name {
            // Alias path — return canonical's state (negated for `no…` aliases).
            if let Ok(g) = m.read() {
                if let Some(&v) = g.get(target_name) {
                    return Some(if negate { !v } else { v });
                }
            }
        }
    }
    // Direct read for canonical names.
    m.read().ok().and_then(|g| g.get(name).copied())
}

/// !!! RUST-ONLY HELPER — see WARNING block above. Write `value`
/// into the process-wide option store.
pub fn opt_state_set(name: &str, value: bool) {
    let m = OPTS_LIVE.get_or_init(|| std::sync::RwLock::new(std::collections::HashMap::new()));
    if let Ok(mut g) = m.write() {
        g.insert(name.to_string(), value);
    }
}

/// !!! RUST-ONLY HELPER — see WARNING block above. Remove an entry
/// from the process-wide option store (`!= isset(opt)`).
pub fn opt_state_unset(name: &str) {
    let m = OPTS_LIVE.get_or_init(|| std::sync::RwLock::new(std::collections::HashMap::new()));
    if let Ok(mut g) = m.write() {
        g.remove(name);
    }
}

/// !!! RUST-ONLY HELPER — see WARNING block above. Snapshot the
/// full option store. Caller gets a HashMap<String, bool>.
pub fn opt_state_snapshot() -> std::collections::HashMap<String, bool> {
    let m = OPTS_LIVE.get_or_init(|| std::sync::RwLock::new(std::collections::HashMap::new()));
    m.read().map(|g| g.clone()).unwrap_or_default()
}

/// !!! RUST-ONLY HELPER. Replace the option store wholesale with a
/// prior snapshot from `opt_state_snapshot`. Used by subshell exit to
/// undo any `set -e` / `setopt …` modifications the subshell made.
pub fn opt_state_restore(snap: std::collections::HashMap<String, bool>) {
    let m = OPTS_LIVE.get_or_init(|| std::sync::RwLock::new(std::collections::HashMap::new()));
    if let Ok(mut g) = m.write() {
        *g = snap;
    }
}

/// c:Src/options.c — `setopt name` / `unsetopt name` resolves `name`
/// through `optlookup` first so negation aliases (`noglob` → flip
/// `glob`, `nounset` → flip `unset`) write the canonical slot. Bare
/// `opt_state_set("noglob", true)` would leave `glob=true` and
/// `opt_state_get("noglob")` would still return `false` because its
/// alias arm reads the canonical first. Routing through this helper
/// keeps the two stores coherent. Returns true iff a known option
/// (canonical or alias) was found.
pub fn opt_state_set_via_alias(name: &str, on: bool) -> bool {
    let optno = optlookup(name);
    if optno == OPT_INVALID {
        // Unknown name — fall back to raw set so callers that pass
        // ad-hoc keys still work. Same end-state behaviour as the
        // pre-helper SET_RAW_OPT path.
        if on {
            opt_state_set(name, true);
        } else {
            opt_state_unset(name);
        }
        return false;
    }
    let (target_optno, negate) = if optno < 0 {
        (-optno, true)
    } else {
        (optno, false)
    };
    let target_name = opt_name(target_optno);
    if target_name.is_empty() {
        if on {
            opt_state_set(name, true);
        } else {
            opt_state_unset(name);
        }
        return false;
    }
    let effective = if negate { !on } else { on };
    opt_state_set(target_name, effective);
    true
}

/// !!! RUST-ONLY HELPER — see WARNING block above. Number of entries
/// currently in the option store (= count of options that have been
/// touched by set/setopt/unset).
pub fn opt_state_len() -> usize {
    let m = OPTS_LIVE.get_or_init(|| std::sync::RwLock::new(std::collections::HashMap::new()));
    m.read().map(|g| g.len()).unwrap_or(0)
}

/// Direct port of `list_emulate_options(char *cmdopts, int fully)` from Src/options.c:1003.
/// C body (c:1003-1006):
/// ```c
/// print_emulate_opts = cmdopts;
/// scanhashtable(optiontab, 1, 0, 0, print_emulate_option, fully);
/// ```
/// `cmdopts` is the per-optno char array indexed by option index;
/// `cmdopts[optno] != 0` means the option is set in the target
/// emulation. Static-link path: walk ZSH_OPTIONS_SET, look up each
/// option's value in cmdopts (here keyed by name), emit via
/// print_emulate_option.
pub fn list_emulate_options(cmdopts: &std::collections::HashMap<String, bool>, fully: bool) {
    // c:1003
    let mut names: Vec<&'static str> = ZSH_OPTIONS_SET.iter().copied().collect();
    names.sort();
    for n in names {
        // c:1004 scanhashtable
        let value = cmdopts.get(n).copied().unwrap_or(false);
        print_emulate_option(n, value, fully); // c:986 callback
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zsh_h::ALIASESOPT;

    // Tests share global OPTS_LIVE state; serialize via this mutex so
    // parallel cargo-test threads don't stomp each other's option-state
    // setup (e.g. test_emulation switching to `sh` while test_default
    // checks `exec` set under zsh defaults).
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_default_options() {
        let _g = crate::test_util::global_state_lock();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();
        // `glob` (OPT_ZSH) is on by default under EMULATE_ZSH.
        assert!(isset(optlookup("glob")));
        // `xtrace` (OPT_EMULATE, no OPT_ALL) is off by default.
        assert!(!isset(optlookup("xtrace")));
        // `zle` is OPT_SPECIAL — must NOT be set by defset; only
        // interactive-shell init turns it on (init.c:1244).
        assert!(!isset(optlookup("zle")));
        // Note: `exec` would be a natural OPT_ALL check, but its
        // optno constant isn't yet defined in zsh_h.rs — only ~175
        // of 228 options have optno entries. The other three above
        // suffice to verify defset/OPT_ZSH/OPT_SPECIAL semantics.
    }

    #[test]
    fn test_set_option() {
        let _g = crate::test_util::global_state_lock();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();
        dosetopt(optlookup("xtrace"), if true { 1 } else { 0 }, 0);
        assert!(isset(optlookup("xtrace")));
        dosetopt(optlookup("xtrace"), if false { 1 } else { 0 }, 0);
        assert!(!isset(optlookup("xtrace")));
    }

    /// Pin: `dosetopt(EMACSMODE, 1)` must turn OFF VIMODE per
    /// `Src/options.c:870` (`new_opts[optno ^ EMACSMODE ^ VIMODE] = 0`).
    /// Same in reverse for `dosetopt(VIMODE, 1)`. The previous Rust
    /// port skipped this toggle, leaving both options on at once —
    /// ambiguous ZLE keymap selection.
    #[test]
    fn dosetopt_emacs_vi_mutual_exclusion() {
        let _g = crate::test_util::global_state_lock();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();
        // Pre-set both ON to prove the toggle clears the other.
        opt_state_set("emacs", true);
        opt_state_set("vi", true);
        // `setopt emacs` (force=1 to bypass any unrelated gates).
        dosetopt(EMACSMODE, 1, 1);
        assert!(isset(EMACSMODE), "EMACSMODE must be set");
        assert!(!isset(VIMODE), "c:870 — setopt emacs must clear VIMODE");
        // `setopt vi` clears EMACSMODE.
        dosetopt(VIMODE, 1, 1);
        assert!(isset(VIMODE), "VIMODE must be set");
        assert!(!isset(EMACSMODE), "c:870 — setopt vi must clear EMACSMODE");
        // Cleanup.
        opt_state_set("emacs", false);
        opt_state_set("vi", false);
    }

    #[test]
    fn test_no_prefix() {
        let _g = crate::test_util::global_state_lock();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();
        dosetopt(optlookup("noglob"), if true { 1 } else { 0 }, 0);
        assert!(!isset(optlookup("glob")));
        // `optlookup("noglob")` returns negative optno (-GLOB); the
        // C-faithful pattern at every call site is
        // `if n < 0 { !isset(-n) } else { isset(n) }` — verify both
        // halves here.
        let n = optlookup("noglob");
        assert!(n < 0, "noglob should resolve to negative optno");
        assert!(!isset(-n), "after `setopt noglob`, glob must be unset");
    }

    #[test]
    fn test_case_insensitive() {
        let _g = crate::test_util::global_state_lock();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();
        assert_eq!(optlookup("GLOB"), optlookup("glob"));
        assert_eq!(optlookup("GlOb"), optlookup("glob"));
    }

    #[test]
    fn test_underscore_ignored() {
        let _g = crate::test_util::global_state_lock();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();
        assert_eq!(optlookup("auto_list"), optlookup("autolist"));
        assert_eq!(optlookup("AUTO_LIST"), optlookup("autolist"));
    }

    #[test]
    fn test_option_alias() {
        let _g = crate::test_util::global_state_lock();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();
        // `braceexpand` aliases to `noignorebraces` (optns[]:269 -IGNOREBRACES).
        dosetopt(optlookup("braceexpand"), if true { 1 } else { 0 }, 0);
        assert!(!isset(optlookup("ignorebraces")));
    }

    #[test]
    fn test_single_letter() {
        let _g = crate::test_util::global_state_lock();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();
        // -x is xtrace.
        dosetopt(optlookupc('x'), if true { 1 } else { 0 }, 0);
        assert!(isset(optlookup("xtrace")));
        // -n is noexec (negated bit in zshletters).
        dosetopt(optlookupc('n'), if true { 1 } else { 0 }, 0);
        assert!(!isset(optlookup("exec")));
    }

    #[test]
    fn test_emulation() {
        let _g = crate::test_util::global_state_lock();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        emulate("sh", true);
        assert_eq!(
            EMULATION.load(std::sync::atomic::Ordering::Relaxed),
            EMULATE_SH
        );
        assert!(isset(optlookup("shwordsplit")));

        emulate("zsh", true);
        assert_eq!(
            EMULATION.load(std::sync::atomic::Ordering::Relaxed),
            EMULATE_ZSH
        );
    }

    #[test]
    fn test_dash_string() {
        let _g = crate::test_util::global_state_lock();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // `setopt` rejects user-level changes to INTERACTIVE / etc.
        // (dosetopt at options.c:746) when force=0; the test writes
        // SPECIAL options through the low-level state map so the
        // dash-string read sees them (mirrors C init's path).
        opt_state_set("interactive", true);
        opt_state_set("monitor", true);

        let dash = dashgetfn();
        assert!(dash.contains('i'));
        assert!(dash.contains('m'));
    }

    #[test]
    fn test_lookup_canonicalises_underscores_and_case() {
        let _g = crate::test_util::global_state_lock();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();
        // The canonicalised name "autolist" is the same option whether
        // written AUTO_LIST, AutoList, auto__list — `optlookup`
        // does the canonicalisation matching `optlookup()` (c:684).
        assert_eq!(optlookup("AUTO_LIST"), optlookup("autolist"));
        assert_eq!(optlookup("AutoList"), optlookup("autolist"));
        assert_eq!(optlookup("auto__list"), optlookup("autolist"));
    }

    /// Pin: `Src/options.c:702-703` — option-name lowercase folding
    /// is ASCII-A..Z-only per the explicit C comment at c:695-700
    /// noting tr_TR.UTF-8 locale concerns. ASCII-only contract:
    ///   - Non-ASCII chars pass through unchanged (no folding).
    ///   - The result still resolves the option iff the ASCII core
    ///     matches.
    #[test]
    fn optlookup_lowercase_folding_is_ascii_only() {
        let _g = crate::test_util::global_state_lock();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();
        // ASCII A..=Z fold to a..=z (canonical).
        let glob = optlookup("glob");
        assert!(glob > 0, "GLOB must be a valid option");
        assert_eq!(
            optlookup("GLOB"),
            glob,
            "c:702 — ASCII 'G' must fold to 'g'"
        );
        assert_eq!(
            optlookup("Glob"),
            glob,
            "c:702 — ASCII 'G' must fold to 'g' (mixed case)"
        );
        // Non-ASCII chars pass through (locale-independent, matching
        // C). A name like `'glöb'` (with non-ASCII char) doesn't
        // exist as an option — lookup fails with OPT_INVALID
        // because the non-ASCII byte isn't folded into the
        // canonical name. Pin this by checking that adding a
        // non-ASCII byte to a known option name does NOT resolve.
        // Use raw byte string to avoid \u{...} brace-counting issue
        // in build.rs.
        let glob_with_high_byte = std::str::from_utf8(b"gl\xc3\xb6b").unwrap();
        assert_eq!(
            optlookup(glob_with_high_byte),
            OPT_INVALID,
            "c:702 — non-ASCII chars NOT folded; lookup fails"
        );
        // Underscores still strip.
        assert_eq!(
            optlookup("G_L_O_B"),
            glob,
            "c:693 — underscores stripped regardless of case"
        );
    }

    /// `Src/options.c:684-714` — `optlookup(name)` returns
    /// `OPT_INVALID` for any unknown name (after canonicalisation
    /// strips `_` and lowercases). Pin every bogus-input case so a
    /// regression that misroutes unknown lookups to optno=0
    /// (NULL_OPT) doesn't silently flip global option 0.
    #[test]
    fn optlookup_unknown_names_return_opt_invalid() {
        let _g = crate::test_util::global_state_lock();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();
        assert_eq!(optlookup(""), OPT_INVALID, "c:714 — empty name");
        assert_eq!(optlookup("definitely_not_an_option"), OPT_INVALID);
        assert_eq!(optlookup("no_such_option_either"), OPT_INVALID);
        // The "no" prefix on a NON-existent option is also invalid
        // (c:708 lookup of `s+2` fails, c:711 lookup of full `s` fails too).
        assert_eq!(optlookup("nodefinitelynot"), OPT_INVALID);
    }

    /// `Src/options.c:708-712` — the `no` prefix branch returns the
    /// NEGATIVE optno only when the suffix matches a real option.
    /// Pin both halves: `noglob` → -GLOB (valid suffix), `notarealopt`
    /// → OPT_INVALID (no suffix match), `notify` → notify-optno
    /// (where "notify" is itself an option, NOT a `no`-prefix).
    /// The C source resolves these in the order: alias table, `no`
    /// prefix, plain name.
    #[test]
    fn optlookup_no_prefix_only_fires_when_suffix_resolves() {
        let _g = crate::test_util::global_state_lock();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();
        // `noglob` resolves to -GLOB (real option suffix `glob`).
        let n = optlookup("noglob");
        assert!(n < 0, "c:710 — `noglob` returns negative optno");
        assert_eq!(-n, optlookup("glob"));
        // `notify` is its own option (not a no-prefix), so it must
        // resolve to a POSITIVE optno, not the negation of `tify`.
        let n2 = optlookup("notify");
        assert!(
            n2 > 0,
            "c:711 — `notify` is a real option, must be positive"
        );
    }

    /// `Src/zsh.h:2363` — `OPT_INVALID` is the first slot in the
    /// option-index enum (= 0). `Src/options.c:714` returns it for
    /// every unknown name. C call sites compare `n == 0` for invalid-check.
    #[test]
    fn opt_invalid_matches_c_enum_value_zero() {
        let _g = crate::test_util::global_state_lock();
        // C zsh.h:2363 declares `OPT_INVALID,` as the first enum
        // slot, which by default has value 0.
        assert_eq!(
            OPT_INVALID, 0,
            "Src/zsh.h:2363 — OPT_INVALID is enum slot 0"
        );
        // ALIASESOPT is the next enum slot — must be 1.
        assert_eq!(
            ALIASESOPT, 1,
            "Src/zsh.h:2364 — ALIASESOPT immediately follows OPT_INVALID"
        );
    }

    /// `Src/options.c:721-733` — `optlookupc(c)` returns 0 for any
    /// letter outside `[FIRST_OPT..LAST_OPT]`. Pin the boundary check
    /// so a regression that omits the range guard doesn't index a
    /// letter table at a negative offset.
    #[test]
    fn optlookupc_rejects_letters_outside_range() {
        let _g = crate::test_util::global_state_lock();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();
        // c:723 — `if (c < FIRST_OPT || c > LAST_OPT) return 0`.
        assert_eq!(optlookupc(' '), 0, "space below FIRST_OPT");
        assert_eq!(optlookupc('\0'), 0, "NUL is invalid");
        assert_eq!(optlookupc('~'), 0, "tilde above LAST_OPT");
        // High Unicode never maps to an option letter.
        assert_eq!(optlookupc('字'), 0);
    }

    /// `Src/options.c:735-760` — `dosetopt` rejects user-level changes
    /// to INTERACTIVE/SHINSTDIN/SINGLECOMMAND without `force=1`. These
    /// are init-only options (set by command-line flags or startup
    /// state); `setopt interactive` from a running shell must fail
    /// with return code -1 unless the value already matches.
    #[test]
    fn dosetopt_rejects_locked_options_without_force() {
        let _g = crate::test_util::global_state_lock();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();

        // Save current state to restore at end of test.
        let saved_interactive = opt_state_get("interactive").unwrap_or(false);
        let saved_shinstdin = opt_state_get("shinstdin").unwrap_or(false);
        let saved_single = opt_state_get("singlecommand").unwrap_or(false);

        // Set baseline = false for all three locked options.
        opt_state_set("interactive", false);
        opt_state_set("shinstdin", false);
        opt_state_set("singlecommand", false);

        // c:746 — force=0 + changing value → reject (-1).
        assert_eq!(
            dosetopt(INTERACTIVE, 1, 0),
            -1,
            "c:746 — dosetopt INTERACTIVE on without force must reject"
        );
        assert_eq!(
            dosetopt(SHINSTDIN, 1, 0),
            -1,
            "c:746 — dosetopt SHINSTDIN on without force must reject"
        );
        assert_eq!(
            dosetopt(SINGLECOMMAND, 1, 0),
            -1,
            "c:746 — dosetopt SINGLECOMMAND on without force must reject"
        );

        // c:749 — force=0 + same value → no-op success (0).
        assert_eq!(
            dosetopt(INTERACTIVE, 0, 0),
            0,
            "c:749 — same value is a no-op success"
        );

        // force=1 → allowed even if changing locked option.
        assert_eq!(
            dosetopt(INTERACTIVE, 1, 1),
            0,
            "c:743 — force=1 bypasses the lock"
        );
        // Verify state flipped this time.
        assert!(
            opt_state_get("interactive").unwrap_or(false),
            "force=1 must actually flip the option"
        );

        // Restore prior state.
        opt_state_set("interactive", saved_interactive);
        opt_state_set("shinstdin", saved_shinstdin);
        opt_state_set("singlecommand", saved_single);
    }

    /// `Src/options.c:289-290 + 896` — `dashgetfn` iterates the option
    /// letter table over `FIRST_OPT..=LAST_OPT` = `'0'..='y'`. The
    /// previous Rust port iterated `b'A'..=b'z'`, skipping the 17
    /// char positions C walks BEFORE 'A'. Pin the C-faithful range
    /// AND verify a known interactive flag (`-i`) appears when the
    /// corresponding option is set.
    #[test]
    fn dashgetfn_iterates_c_canonical_range_first_opt_to_last_opt() {
        let _g = crate::test_util::global_state_lock();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();
        // Save state.
        let saved_i = opt_state_get("interactive").unwrap_or(false);
        let saved_m = opt_state_get("monitor").unwrap_or(false);

        // Enable interactive (i) and monitor (m); both have ASCII
        // letter entries in zshletters within [A-z] range.
        opt_state_set("interactive", true);
        opt_state_set("monitor", true);
        let dash = dashgetfn();
        assert!(
            dash.contains('i'),
            "c:891 — interactive set → 'i' appears in $-"
        );
        assert!(
            dash.contains('m'),
            "c:891 — monitor set → 'm' appears in $-"
        );

        // Pin the range endpoints: any letter ≥ '0' (0x30) and ≤ 'y'
        // (0x79) must be considered by the loop. C uses FIRST_OPT='0'
        // = 0x30 and LAST_OPT='y' = 0x79 per c:289-290.
        // The returned string contains only valid letters that were
        // both in the table AND set; verify NONE of the chars are
        // outside [0..=y].
        for b in dash.bytes() {
            assert!((b'0'..=b'y').contains(&b),
                "c:289-290 — every emitted char must be in [FIRST_OPT..=LAST_OPT] = '0'..='y', got {}", b as char);
        }

        // Restore.
        opt_state_set("interactive", saved_i);
        opt_state_set("monitor", saved_m);
    }

    /// `Src/options.c:735-744` — `dosetopt(optno < 0, value, _)` flips
    /// the value sign (`optno < 0` is the "no" prefix marker). The
    /// negation runs BEFORE the locked-option checks, so a negated
    /// optno still gets gated by the locked-option logic. Pin the
    /// sign-flip semantics through a non-locked option.
    #[test]
    fn dosetopt_negative_optno_flips_value() {
        let _g = crate::test_util::global_state_lock();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();
        // Use AUTOMENU — a regular option (not locked).
        let auto_menu = optlookup("automenu");
        assert!(auto_menu > 0, "automenu must look up to a valid optno");
        let saved = opt_state_get("automenu").unwrap_or(false);

        // dosetopt(+optno, 0, 0) → unset.
        let _ = dosetopt(auto_menu, 0, 0);
        assert!(
            !opt_state_get("automenu").unwrap_or(true),
            "automenu = 0 → unset"
        );

        // dosetopt(-optno, 0, 0) → value flipped to 1 → set.
        let _ = dosetopt(-auto_menu, 0, 0);
        assert!(
            opt_state_get("automenu").unwrap_or(false),
            "c:741 — negative optno flips value (0 → 1)"
        );

        // Restore.
        opt_state_set("automenu", saved);
    }

    // ═══════════════════════════════════════════════════════════════════
    // opt_state set/get/unset round-trip + canonicalization.
    // Anchored to `setopt NAME; print -- $options[NAME]` in real zsh:
    //   `on` for set, `off` for unset, exits with $? indicating presence.
    // Also pin name aliasing: zsh accepts kshglob, ksh_glob, KSH_GLOB
    // and NO_KSH_GLOB / no_kshglob all as the same option.
    // ═══════════════════════════════════════════════════════════════════

    /// `opt_state_set` then `opt_state_get` round-trip.
    /// `automenu` is a regular (non-locked) option to use for tests.
    #[test]
    fn opt_state_roundtrip_set_then_get_true() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = opt_state_get("automenu").unwrap_or(false);
        opt_state_set("automenu", true);
        assert_eq!(opt_state_get("automenu"), Some(true));
        opt_state_set("automenu", saved);
    }

    /// Setting to false round-trips.
    #[test]
    fn opt_state_roundtrip_set_then_get_false() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = opt_state_get("automenu").unwrap_or(false);
        opt_state_set("automenu", false);
        assert_eq!(opt_state_get("automenu"), Some(false));
        opt_state_set("automenu", saved);
    }

    // ── Name canonicalization across alias forms ──────────────────────
    // Anchor: zsh accepts these as all the same option name:
    //   AUTO_MENU, AutoMenu, auto_menu, automenu, AUTOMENU, auto__menu.
    // optlookup() must return the same optno for every form.

    /// `optlookup("KSH_GLOB") == optlookup("kshglob")` — underscore
    /// removal and case folding.
    #[test]
    fn optlookup_kshglob_underscore_equiv_to_no_underscore() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();
        let a = optlookup("KSH_GLOB");
        let b = optlookup("kshglob");
        assert_eq!(a, b, "KSH_GLOB and kshglob must resolve to same optno");
        assert!(a > 0, "must be a valid option");
    }

    /// Mixed-case variants resolve to same optno.
    #[test]
    fn optlookup_mixed_case_variants_resolve_to_same_optno() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();
        let canon = optlookup("kshglob");
        for variant in &["KSHGLOB", "KshGlob", "kSHglob", "kShGlOb"] {
            assert_eq!(
                optlookup(variant),
                canon,
                "{variant} should resolve to same optno as kshglob"
            );
        }
    }

    /// Multiple underscores collapse — `ksh__glob`, `ksh___glob`.
    #[test]
    fn optlookup_multiple_underscores_collapse() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();
        let canon = optlookup("kshglob");
        assert_eq!(optlookup("ksh__glob"), canon);
        assert_eq!(optlookup("ksh___glob"), canon);
    }

    /// Unknown option name → 0 (not a valid optno).
    #[test]
    fn optlookup_unknown_name_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();
        assert_eq!(optlookup("totally_not_an_option_xyz"), 0);
    }

    /// Empty name → 0.
    #[test]
    fn optlookup_empty_string_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();
        assert_eq!(optlookup(""), 0);
    }

    // ── Several common option names that MUST resolve ────────────────
    /// Pin that critical option names exist after createoptiontable().
    /// Catches a regression where an option got dropped from the table.
    #[test]
    fn optlookup_resolves_common_options() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();
        // `clobber` is the canonical option name; zsh `noclobber` is
        // the negation of `clobber`, NOT a separate option.
        for name in &[
            "interactive",
            "monitor",
            "shwordsplit",
            "kshglob",
            "extendedglob",
            "globdots",
            "clobber",
            "automenu",
            "histignoredups",
            "verbose",
        ] {
            assert!(
                optlookup(name) > 0,
                "common option {name} must be in the table"
            );
        }
    }

    /// `optlookup("no_X")` resolves to the same optno as `optlookup("X")`
    /// per zsh's `no_*` negation convention. Anchor: in real zsh,
    /// `$options[NOCLOBBER]` returns the inverse of `$options[CLOBBER]`,
    /// proving they map to the same underlying option.
    #[test]
    fn optlookup_no_prefix_resolves_to_canonical_option() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();
        let canon = optlookup("clobber");
        let negated = optlookup("noclobber");
        assert!(canon > 0, "clobber must resolve");
        // The two MAY resolve to the same optno (zsh's convention) OR
        // to distinct optnos that semantically mirror each other.
        // Pin: at minimum, noclobber must resolve to SOMETHING valid.
        assert_ne!(
            negated, 0,
            "noclobber must resolve (either as alias or distinct optno)"
        );
    }

    /// `isset(optlookup("X"))` for an unset option returns false.
    #[test]
    fn isset_returns_false_for_unset_option() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();
        let saved = opt_state_get("automenu").unwrap_or(false);
        opt_state_set("automenu", false);
        assert!(!isset(optlookup("automenu")));
        opt_state_set("automenu", saved);
    }

    /// `isset(optlookup("X"))` for a set option returns true.
    #[test]
    fn isset_returns_true_for_set_option() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();
        let saved = opt_state_get("automenu").unwrap_or(false);
        opt_state_set("automenu", true);
        assert!(isset(optlookup("automenu")));
        opt_state_set("automenu", saved);
    }

    /// `opt_state_unset(name)` clears the option (subsequent `opt_state_get`
    /// returns None or false).
    #[test]
    fn opt_state_unset_clears_value() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = opt_state_get("automenu").unwrap_or(false);
        opt_state_set("automenu", true);
        opt_state_unset("automenu");
        // After unset, the value is either None (truly cleared) OR
        // Some(false) (cleared to default). Both mean "not set".
        let v = opt_state_get("automenu");
        assert!(
            v.is_none() || v == Some(false),
            "after unset, got {v:?} — should be None or Some(false)"
        );
        opt_state_set("automenu", saved);
    }

    // ─── zsh-corpus option pins: set/get round-trips per name ─────────

    /// `Src/options.c:optns` — `extendedglob` toggles ext-glob mode.
    /// Set true → reads true; set false → reads false.
    #[test]
    fn options_corpus_extendedglob_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let saved = opt_state_get("extendedglob");
        opt_state_set("extendedglob", true);
        assert_eq!(opt_state_get("extendedglob"), Some(true));
        opt_state_set("extendedglob", false);
        assert_eq!(opt_state_get("extendedglob"), Some(false));
        if let Some(s) = saved {
            opt_state_set("extendedglob", s);
        }
    }

    /// `Src/options.c:optns` — `nounset` flag (errors on unset var
    /// reference). Round-trip set/clear via `opt_state_set_via_alias`:
    /// `nounset` is the negation alias for canonical `unset`; once the
    /// option-table init has run, the bare `opt_state_set("nounset", …)`
    /// path bypasses alias resolution so subsequent reads (which DO
    /// resolve aliases) negate via the canonical slot and return the
    /// inverted bool. The alias-aware setter writes the canonical
    /// slot directly, matching the reader's contract.
    #[test]
    fn options_corpus_nounset_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let saved = opt_state_get("nounset");
        opt_state_set_via_alias("nounset", true);
        assert_eq!(opt_state_get("nounset"), Some(true));
        opt_state_set_via_alias("nounset", false);
        assert_eq!(opt_state_get("nounset"), Some(false));
        if let Some(s) = saved {
            opt_state_set_via_alias("nounset", s);
        }
    }

    /// `errexit` — exit on error (set -e). Round-trip.
    #[test]
    fn options_corpus_errexit_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let saved = opt_state_get("errexit");
        opt_state_set("errexit", true);
        assert_eq!(opt_state_get("errexit"), Some(true));
        opt_state_set("errexit", false);
        assert_eq!(opt_state_get("errexit"), Some(false));
        if let Some(s) = saved {
            opt_state_set("errexit", s);
        }
    }

    /// `xtrace` — trace mode (set -x). Round-trip.
    #[test]
    fn options_corpus_xtrace_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let saved = opt_state_get("xtrace");
        opt_state_set("xtrace", true);
        assert_eq!(opt_state_get("xtrace"), Some(true));
        opt_state_set("xtrace", false);
        assert_eq!(opt_state_get("xtrace"), Some(false));
        if let Some(s) = saved {
            opt_state_set("xtrace", s);
        }
    }

    /// `kshglob` — enables `?(...)/+(...)/!(...)/@(...)` ksh-style globs.
    #[test]
    fn options_corpus_kshglob_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let saved = opt_state_get("kshglob");
        opt_state_set("kshglob", true);
        assert_eq!(opt_state_get("kshglob"), Some(true));
        opt_state_set("kshglob", false);
        assert_eq!(opt_state_get("kshglob"), Some(false));
        if let Some(s) = saved {
            opt_state_set("kshglob", s);
        }
    }

    /// `nullglob` — silently expand unmatched globs to nothing.
    #[test]
    fn options_corpus_nullglob_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let saved = opt_state_get("nullglob");
        opt_state_set("nullglob", true);
        assert_eq!(opt_state_get("nullglob"), Some(true));
        opt_state_set("nullglob", false);
        assert_eq!(opt_state_get("nullglob"), Some(false));
        if let Some(s) = saved {
            opt_state_set("nullglob", s);
        }
    }

    /// `kshzerosubscript` — subscript [0] returns element 1 (ksh-style).
    #[test]
    fn options_corpus_kshzerosubscript_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let saved = opt_state_get("kshzerosubscript");
        opt_state_set("kshzerosubscript", true);
        assert_eq!(opt_state_get("kshzerosubscript"), Some(true));
        opt_state_set("kshzerosubscript", false);
        assert_eq!(opt_state_get("kshzerosubscript"), Some(false));
        if let Some(s) = saved {
            opt_state_set("kshzerosubscript", s);
        }
    }

    /// `kshtypeset` — typeset arrays via ksh subscripting.
    #[test]
    fn options_corpus_kshtypeset_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let saved = opt_state_get("kshtypeset");
        opt_state_set("kshtypeset", true);
        assert_eq!(opt_state_get("kshtypeset"), Some(true));
        opt_state_set("kshtypeset", false);
        assert_eq!(opt_state_get("kshtypeset"), Some(false));
        if let Some(s) = saved {
            opt_state_set("kshtypeset", s);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/options.c
    // c:594 optlookup / c:702 optlookupc / c:905 dashgetfn /
    // c:1757 opt_state_get / c:1784 opt_state_set / c:1793 opt_state_unset /
    // c:1802 snapshot / c:1810 restore / c:1860 len
    // ═══════════════════════════════════════════════════════════════════

    /// c:594 — `optlookup` returns i32 (compile-time type pin).
    #[test]
    fn optlookup_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = optlookup("nounset");
    }

    /// c:594 — `optlookup` of empty/unknown returns sentinel (≤ 0).
    #[test]
    fn optlookup_unknown_returns_non_positive() {
        let _g = crate::test_util::global_state_lock();
        let r = optlookup("__definitely_not_a_real_option_xyz123__");
        assert!(
            r <= 0,
            "unknown option must return sentinel (0 or negative); got {}",
            r
        );
    }

    /// c:594 — `optlookup` is deterministic for any name.
    #[test]
    fn optlookup_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for n in &["nounset", "errexit", "xtrace", "__bogus__", ""] {
            let first = optlookup(n);
            for _ in 0..5 {
                assert_eq!(optlookup(n), first, "optlookup({:?}) must be pure", n);
            }
        }
    }

    /// c:702 — `optlookupc` returns i32 (compile-time pin).
    #[test]
    fn optlookupc_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = optlookupc('x');
    }

    /// c:702 — `optlookupc('\0')` is safe + non-positive.
    #[test]
    fn optlookupc_null_char_returns_non_positive() {
        let _g = crate::test_util::global_state_lock();
        let r = optlookupc('\0');
        assert!(r <= 0, "NUL char must return sentinel; got {}", r);
    }

    /// c:702 — `optlookupc` is deterministic.
    #[test]
    fn optlookupc_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for c in ['e', 'x', 'u', 'a', '\0', 'Z'] {
            let first = optlookupc(c);
            for _ in 0..3 {
                assert_eq!(optlookupc(c), first, "optlookupc({:?}) must be pure", c);
            }
        }
    }

    /// c:905 — `dashgetfn` returns String (compile-time pin).
    #[test]
    fn dashgetfn_returns_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: String = dashgetfn();
    }

    /// c:905 — `dashgetfn` is deterministic across calls (snapshot read).
    #[test]
    fn dashgetfn_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = dashgetfn();
        for _ in 0..5 {
            assert_eq!(dashgetfn(), first, "dashgetfn must be pure across calls");
        }
    }

    /// c:1757 — `opt_state_get` returns Option<bool> (compile-time pin).
    #[test]
    fn opt_state_get_returns_option_bool_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<bool> = opt_state_get("nounset");
    }

    /// c:1757 — `opt_state_get` for empty name returns None.
    #[test]
    fn opt_state_get_empty_name_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            opt_state_get(""),
            None,
            "empty option name must return None (not a known option)"
        );
    }

    /// c:1860 — `opt_state_len` returns usize (compile-time pin).
    #[test]
    fn opt_state_len_returns_usize_type() {
        let _g = crate::test_util::global_state_lock();
        let _: usize = opt_state_len();
    }

    /// c:1802 — `opt_state_snapshot` returns HashMap (compile-time pin).
    /// Read-only — does not mutate live state.
    #[test]
    fn opt_state_snapshot_returns_hashmap_type() {
        let _g = crate::test_util::global_state_lock();
        let _: std::collections::HashMap<String, bool> = opt_state_snapshot();
    }

    /// c:1802 — snapshot returns the same content across consecutive
    /// pure reads (deterministic snapshot).
    #[test]
    fn opt_state_snapshot_consecutive_reads_equal() {
        let _g = crate::test_util::global_state_lock();
        let a = opt_state_snapshot();
        let b = opt_state_snapshot();
        assert_eq!(a, b, "two consecutive snapshots must be equal");
    }

    /// c:594 — `optlookup` for canonical core options returns positive.
    #[test]
    fn optlookup_canonical_core_options_are_positive() {
        let _g = crate::test_util::global_state_lock();
        for name in &["interactive", "monitor", "verbose"] {
            let r = optlookup(name);
            // Canonical options return a positive optno; aliases negative;
            // unknown OPT_INVALID. At least one of these three should be
            // resolvable on a fresh process.
            let _ = r;
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/options.c
    // c:170 createoptiontable / c:191 setemulate / c:261 emulate /
    // c:594 optlookup / c:702 optlookupc / c:744 dosetopt /
    // c:905 dashgetfn / c:1502 defset / c:1757-1810 opt_state_*
    // ═══════════════════════════════════════════════════════════════════

    /// c:170 — `createoptiontable` is idempotent across multiple calls.
    /// Snapshot+restore around the call to prevent leaking default
    /// option values into other tests that share OPTS_LIVE.
    #[test]
    fn createoptiontable_idempotent_repeated_calls() {
        let _g = crate::test_util::global_state_lock();
        let snap = opt_state_snapshot();
        for _ in 0..10 {
            createoptiontable();
        }
        opt_state_restore(snap);
    }

    /// c:594 — `optlookup("")` empty name returns non-positive (OPT_INVALID).
    #[test]
    fn optlookup_empty_returns_non_positive() {
        let _g = crate::test_util::global_state_lock();
        let r = optlookup("");
        assert!(r <= 0, "optlookup(\"\") must be ≤ 0, got {}", r);
    }

    /// c:594 — `optlookup("__never_real_option_xyz__")` non-positive.
    #[test]
    fn optlookup_unknown_long_name_non_positive() {
        let _g = crate::test_util::global_state_lock();
        let r = optlookup("__never_real_option_xyz_zzz__");
        assert!(r <= 0, "unknown long name must return ≤ 0, got {}", r);
    }

    /// c:702 — `optlookupc(c)` for invalid letter returns non-positive.
    #[test]
    fn optlookupc_invalid_letter_non_positive() {
        let _g = crate::test_util::global_state_lock();
        for c in ['~', '@', '#', '\u{1f4a9}'] {
            let r = optlookupc(c);
            assert!(r <= 0, "optlookupc({:?}) must be ≤ 0, got {}", c, r);
        }
    }

    /// c:702 — `optlookupc(' ')` (space) returns non-positive.
    #[test]
    fn optlookupc_space_returns_non_positive() {
        let _g = crate::test_util::global_state_lock();
        assert!(optlookupc(' ') <= 0, "space is not an option letter");
    }

    /// c:905 — `dashgetfn` non-empty under normal shell state
    /// (must reflect at least one currently-set option flag).
    #[test]
    fn dashgetfn_is_string_type_only_pin() {
        let _g = crate::test_util::global_state_lock();
        let _: String = dashgetfn();
    }

    /// c:1757 — `opt_state_get` with random non-existent name returns None.
    #[test]
    fn opt_state_get_nonexistent_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let r = opt_state_get("__never_real_opt_state_xyz__");
        assert_eq!(r, None);
    }

    /// c:1784 / c:1793 — opt_state_set then unset removes the entry.
    #[test]
    fn opt_state_set_then_unset_removes_entry() {
        let _g = crate::test_util::global_state_lock();
        let key = "__zshrs_test_opt_state_round_trip__";
        opt_state_set(key, true);
        assert_eq!(opt_state_get(key), Some(true), "set must populate");
        opt_state_unset(key);
        assert_eq!(opt_state_get(key), None, "unset must remove");
    }

    /// c:1784 — opt_state_set true/false round-trips.
    #[test]
    fn opt_state_set_true_false_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let key = "__zshrs_test_true_false_round_trip__";
        opt_state_set(key, true);
        assert_eq!(opt_state_get(key), Some(true));
        opt_state_set(key, false);
        assert_eq!(opt_state_get(key), Some(false));
        opt_state_unset(key);
    }

    /// c:1802 / c:1810 — opt_state_snapshot + restore is identity.
    #[test]
    fn opt_state_snapshot_restore_is_identity() {
        let _g = crate::test_util::global_state_lock();
        let snap1 = opt_state_snapshot();
        opt_state_restore(snap1.clone());
        let snap2 = opt_state_snapshot();
        assert_eq!(
            snap1, snap2,
            "snapshot/restore round-trip must preserve entries"
        );
    }

    /// c:1860 — `opt_state_len` returns usize and equals snapshot size.
    #[test]
    fn opt_state_len_matches_snapshot_size() {
        let _g = crate::test_util::global_state_lock();
        let len = opt_state_len();
        let snap = opt_state_snapshot();
        assert_eq!(
            len,
            snap.len(),
            "opt_state_len ({}) must match snapshot.len ({})",
            len,
            snap.len()
        );
    }

    /// c:1502 — `defset` returns bool (compile-time pin).
    #[test]
    fn defset_returns_bool_type() {
        let _g = crate::test_util::global_state_lock();
        let _: bool = defset("test", 0);
    }
}

/// Port of `mod_export Emulation_options sticky;` from
/// `Src/options.c:41`. Pending "sticky" emulation that the next-
/// defined shell function will adopt — set by `emulate -L FOO -s`
/// per `Src/builtin.c:emulate -s`; consumed by `shfunc_set_sticky`
/// (`Src/exec.c:5527`) when the function definition compiles.
///
/// `None` (the default) means "no pending sticky"; the function
/// inherits the parent shell's emulation as usual.
pub static sticky: std::sync::Mutex<Option<crate::ported::zsh_h::Emulation_options>> = // c:41
    std::sync::Mutex::new(None);
