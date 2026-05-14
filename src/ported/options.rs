//! Shell options for zshrs
//!
//! Direct port from zsh/Src/options.c
//!
//! Manages all shell options including:
//! - Option lookup by name and single-letter
//! - Emulation modes (zsh, ksh, sh, csh)
//! - Option aliases (bash/ksh compatibility)
//! - setopt/unsetopt builtins

use std::collections::HashMap;
use std::sync::atomic::AtomicI32;
use crate::ported::utils::zwarnnam;
use crate::ported::zsh_h::EMULATE_FULLY;
use crate::ported::zsh_h::{APPENDHISTORY, BANGHIST, CHASELINKS, GLOBDOTS, HASHCMDS, HISTNOFUNCTIONS, IGNOREBRACES, MAILWARNING, PROMPTSUBST, SHINSTDIN, SINGLECOMMAND, };
use crate::ported::zsh_h::OPT_SIZE;
use crate::ported::zsh_h as zh;

/// Emulation flags for option defaults
// `#define OPT_X EMULATE_X` (options.c:55-58) — the option-default
// bits ARE the emulation bits. Direct mirror of the C macros.
const OPT_CSH: u8 = crate::ported::zsh_h::EMULATE_CSH as u8;                 // c:55
const OPT_KSH: u8 = crate::ported::zsh_h::EMULATE_KSH as u8;                 // c:56
const OPT_SH:  u8 = crate::ported::zsh_h::EMULATE_SH  as u8;                 // c:57
const OPT_ZSH: u8 = crate::ported::zsh_h::EMULATE_ZSH as u8;                 // c:58
const OPT_ALL: u8 = OPT_CSH | OPT_KSH | OPT_SH | OPT_ZSH;                    // c:60
const OPT_BOURNE: u8 = OPT_KSH | OPT_SH;                                     // c:61
const OPT_BSHELL: u8 = OPT_KSH | OPT_SH | OPT_ZSH;                           // c:62
const OPT_NONBOURNE: u8 = OPT_ALL & !OPT_BOURNE;                             // c:63
const OPT_NONZSH: u8 = OPT_ALL & !OPT_ZSH;                                   // c:64

/// Option flags
// option is relevant to emulation                                          // c:66
const OPT_EMULATE: u16 = crate::ported::zsh_h::EMULATE_UNUSED as u16;        // c:67
// option should never be set by emulate()                                  // c:68
const OPT_SPECIAL: u16 = (crate::ported::zsh_h::EMULATE_UNUSED << 1) as u16; // c:69
// option is an alias to an other option                                    // c:70
const OPT_ALIAS: u16 = (crate::ported::zsh_h::EMULATE_UNUSED << 2) as u16;   // c:71

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
pub fn printoptionnode(hn: &str, set: bool) {                                // c:450
    let on = opt_state_get(hn).unwrap_or(false);                             // c:450 isset(optno)
    let default_on = default_on_options().contains(&hn);                     // c:455 defset(on, emulation)
    let kshprint = opt_state_get("kshoptionprint").unwrap_or(false);         // c:456 isset(KSHOPTIONPRINT)
    if kshprint {                                                            // c:456
        if default_on {                                                      // c:457
            println!("no{:<19} {}", hn, if on { "off" } else { "on" });      // c:458
        } else {
            println!("{:<21} {}", hn, if on { "on" } else { "off" });        // c:460
        }
    } else if set == (on ^ default_on) {                                     // c:462
        if set ^ on {                                                        // c:463
            print!("no");                                                    // c:464
        }
        println!("{}", hn);                                                  // c:465
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
pub fn createoptiontable() {                                                 // c:471
    let zsh_emu = EMULATE_ZSH;
    for name in ZSH_OPTIONS_SET.iter() {                                     // c:46 opts[optno] = defset(...)
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
pub fn setemulate(name: &str, fully: i32) {                                  // c:507
    let flags = optns_flags(name);                                           // c:507
    // c:515-517 — emulation-relevant filter.
    let is_alias = (flags & OPT_ALIAS) != 0;
    let is_special = (flags & OPT_SPECIAL) != 0;
    let is_emulate = (flags & OPT_EMULATE) != 0;
    if is_alias {
        return;
    }
    if !((fully != 0 && !is_special) || is_emulate) {                        // c:516-517
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
pub fn installemulation(new_emulation: i32,
                        new_opts: &mut std::collections::HashMap<String, bool>) { // c:523
    // c:525 — `setemulate_emulation = new_emulation;`
    SETEMULATE_EMULATION
        .store(new_emulation, std::sync::atomic::Ordering::Relaxed);         // c:525
    // c:526 — `setemulate_opts = new_opts;`. We can't alias the
    // caller's HashMap directly, so the per-option callback writes
    // into our module-static and we splice it back into `new_opts`.
    if let Ok(mut tab) = setemulate_opts_lock().lock() {
        tab.clear();
    }
    // c:527-528 — scanhashtable(optiontab, ..., setemulate, fully).
    let fully = if (new_emulation & EMULATE_FULLY) != 0 { 1 } else { 0 };    // c:528
    for name in ZSH_OPTIONS_SET.iter() {
        setemulate(name, fully);                                             // c:527
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
pub fn emulate(mode: &str, fully: bool) {                                    // c:533
    let ch = mode.chars().next().unwrap_or('z');
    let ch = if ch == 'r' { mode.chars().nth(1).unwrap_or('z') } else { ch };
    let new_emu = match ch {
        'c' => crate::ported::zsh_h::EMULATE_CSH,
        'k' => crate::ported::zsh_h::EMULATE_KSH,
        's' | 'b' => crate::ported::zsh_h::EMULATE_SH,
        _ => crate::ported::zsh_h::EMULATE_ZSH,
    };
    EMULATION.store(new_emu, std::sync::atomic::Ordering::Relaxed);
    FULLY_EMULATING.store(fully, std::sync::atomic::Ordering::Relaxed);

    // c:551-572 — body of `installemulation()` + exec.c:5933-5938
    // OPT_SPECIAL-skip walk, inlined here per the C source.
    let mut emu = new_emu;
    if fully {
        emu |= crate::ported::zsh_h::EMULATE_FULLY;                          // c:551
    }
    let mut new_opts: std::collections::HashMap<String, bool> =
        std::collections::HashMap::new();
    installemulation(emu, &mut new_opts);                                    // c:552
    for (k, v) in &new_opts {
        if (optns_flags(k) & OPT_SPECIAL) == 0 {                             // exec.c:5933-5938
            opt_state_set(k, *v);
        }
    }
    if new_emu == crate::ported::zsh_h::EMULATE_ZSH {
        // c:46 — `opts[optno] = defset(...)` walk for zsh defaults.
        for name in ZSH_OPTIONS_SET.iter() {
            opt_state_set(name, defset(name, EMULATE_ZSH));
        }
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

// ===========================================================
// Static + helpers moved verbatim from src/ported/exec.rs.
// These are the C options.c port-of-record (canonical option
// name list, default values, normalization, pattern matching,
// emulation-mode option deltas, and the option-printing
// helpers). Their C counterparts all live in
// src/zsh/Src/options.c (`optns[]` table, `defset()`,
// `installemulation()`, `printoptions()`).
// ===========================================================

// BEGIN moved-from-exec-rs (statics)
use std::collections::HashSet;
use std::sync::LazyLock;
use crate::zsh_h::{isset, EMULATE_ZSH};

/// `setopt OPT` builtin per-arg dispatch.
/// Port of `setoption(HashNode hn, int value)` from Src/options.c:573 — the inner loop
/// of `bin_setopt`. Returns 0 on success, -1 on bad option name.
pub fn setoption(hn: &str, value: i32) -> i32 {
    // C: `opts[optno] = value;` — the C source writes the option's
    // live state into the `opts[]` array. The Rust port stores it
    // in OPTS_LIVE via `opt_state_set` (the same global the
    // `optlookup("hn")>0` and `isset(OPT)` paths read).
    opt_state_set(hn, value != 0);                                         // c:735+ dosetopt body
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
pub fn bin_setopt(nam: &str, args: &[String],                                // c:580
                  _ops: &crate::ported::zsh_h::options, isun: i32) -> i32 {
    let mut retval = 0i32;
    let mut match_glob = false;                                              // c:582
    let mut idx = 0usize;

    if args.is_empty() {                                                     // c:586
        // c:587 — scanhashtable(optiontab, 1, 0, OPT_ALIAS,
        // optiontab->printnode, !isun): walk every option in the
        // table and emit each one whose current state matches !isun.
        let want_set = isun == 0;
        let mut names: Vec<String> = ZSH_OPTIONS_SET.iter()
            .map(|s| s.to_string()).collect();
        names.sort();
        for n in names {
            let on = opt_state_get(&n).unwrap_or(false);
            if on == want_set {
                printoptionnode(&n, want_set);                               // c:587 printnode
            }
        }
        return 0;                                                            // c:589
    }

    // c:592-636 — leading `-`/`+` flag parse loop.
    'outer: while idx < args.len()
        && (args[idx].starts_with('-') || args[idx].starts_with('+'))
    {
        let leading = args[idx].as_bytes()[0];                               // c:594
        let action: i32 = ((leading == b'-') as i32) ^ isun;                 // c:594
        if args[idx].len() == 1 {                                            // c:596 args[0][1] empty
            // c:597 — `*args = "--";` then fall through to the
            // inner while which immediately matches `-` and breaks
            // into doneoptions. Equivalent: skip past this arg and
            // exit the outer loop.
            idx += 1;
            break 'outer;
        }
        let body_bytes = args[idx].as_bytes()[1..].to_vec();                 // c:599 *++*args
        let mut k = 0usize;
        while k < body_bytes.len() {                                         // c:599
            let mut c = body_bytes[k];
            // c:600-601 — `if(**args == Meta) *++*args ^= 32;` —
            // unmeta the next byte before reading.
            if c == crate::ported::zsh_h::META as u8 {                       // c:600
                k += 1;
                if k < body_bytes.len() { c = body_bytes[k] ^ 32; }          // c:601
                else { break; }
            }
            if c == b'-' {                                                   // c:603 pseudo `--`
                idx += 1;                                                    // c:604
                break 'outer;                                                // c:605 goto doneoptions
            } else if c == b'o' {                                            // c:606
                // c:607-608 — if more chars after 'o', use them as the
                // option name; otherwise advance to next arg.
                let oarg: String = if k + 1 < body_bytes.len() {             // c:607
                    String::from_utf8_lossy(&body_bytes[k + 1..]).into_owned()
                } else {
                    idx += 1;                                                // c:608
                    if idx >= args.len() {                                   // c:609 !*args
                        zwarnnam(nam, "string expected after -o");           // c:610
                        return 1;                                            // c:612
                    }
                    args[idx].clone()
                };
                let optno = optlookup(&oarg);                                // c:614
                if optno == 0 {                                              // c:614
                    zwarnnam(nam,                                            // c:615
                        &format!("no such option: {}", oarg));
                    retval |= 1;
                } else if dosetopt(optno, action, 0) != 0 {                  // c:617
                    zwarnnam(nam,                                            // c:618
                        &format!("can't change option: {}", oarg));
                    retval |= 1;
                }
                break;                                                       // c:622 break inner
            } else if c == b'm' {                                            // c:624
                match_glob = true;                                           // c:625
            } else {                                                         // c:626
                let optno = optlookupc(c as char);                           // c:627
                if optno == 0 {                                              // c:627
                    zwarnnam(nam, &format!("bad option: -{}", c as char));   // c:628
                    retval |= 1;
                } else if dosetopt(optno, action, 0) != 0 {                  // c:630
                    zwarnnam(nam,                                            // c:631
                        &format!("can't change option: -{}", c as char));
                    retval |= 1;
                }
            }
            k += 1;
        }
        idx += 1;                                                            // c:636 args++
    }

    // c:638 — doneoptions: positional args remain.
    if !match_glob {                                                         // c:640
        // c:642-650 — bare option names.
        while idx < args.len() {                                             // c:642
            let oname = args[idx].clone();
            idx += 1;
            let optno = optlookup(&oname);                                   // c:643
            if optno == 0 {                                                  // c:643
                zwarnnam(nam,                                                // c:644
                    &format!("no such option: {}", oname));
                retval |= 1;
            } else {
                let v = (isun == 0) as i32;                                  // c:646 !isun
                if dosetopt(optno, v, 0) != 0 {                              // c:646
                    zwarnnam(nam,                                            // c:647
                        &format!("can't change option: {}", oname));
                    retval |= 1;
                }
            }
        }
    } else {                                                                 // c:653
        // c:655-678 — globbing branch.
        while idx < args.len() {                                             // c:655
            let raw = args[idx].clone();
            idx += 1;
            // c:660-666 — `s = dupstring(*args);` then walk: strip
            // `_`, lowercase A-Z (mirrors optlookup's canonicalisation
            // documented at c:684).
            let normalized: String = raw.chars()
                .filter(|&c| c != '_')
                .map(|c| c.to_ascii_lowercase()).collect();
            // c:670 — patcompile(s, PAT_HEAPDUP, NULL).
            let prog = crate::ported::pattern::patcompile(
                &normalized,
                crate::ported::zsh_h::PAT_HEAPDUP,
                None,
            );
            if prog.is_none() {                                              // c:670
                zwarnnam(nam, &format!("bad pattern: {}", raw));             // c:671
                retval |= 1;
                break;                                                       // c:673
            }
            // c:676 — scanmatchtable(optiontab, pprog, 0, 0, OPT_ALIAS,
            // setoption, !isun): the `setoption` static at c:572 calls
            // `dosetopt(optname->optno, !isun, 0, opts)` on each match.
            let v = (isun == 0) as i32;
            for opt_name in ZSH_OPTIONS_SET.iter() {                         // c:676
                if crate::ported::pattern::patmatch(&normalized, opt_name) {
                    let _ = setoption(opt_name, v);                          // c:572 setoption
                }
            }
        }
    }
    crate::ported::utils::inittyptab();                                                            // c:678
    retval                                                                   // c:679
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
pub fn optlookup(name: &str) -> i32 {                                        // c:684

    // c:689 — `s = t = dupstring(name);`
    // c:691-705 — strip `_` + lowercase.
    let s: String = name.chars()                                             // c:689
        .filter(|&c| c != '_')                                               // c:693-694
        .flat_map(|c| c.to_lowercase())                                      // c:702-703
        .collect();

    // OPT_ALIAS rows from optns[]:269-280 — alias names resolve to
    // their target optno (signed for the alias's negation polarity).
    // C zsh stores these as hash entries in optiontab with
    // `optno = -target` (negative) for the `-PREFIX` rows.
    let alias_optno: Option<i32> = match s.as_str() {
        "braceexpand" => Some(-IGNOREBRACES),                                // c:269 -IGNOREBRACES
        "dotglob"     => Some(GLOBDOTS),                                     // c:270 GLOBDOTS
        "hashall"     => Some(HASHCMDS),                                     // c:271 HASHCMDS
        "histappend"  => Some(APPENDHISTORY),                                // c:272 APPENDHISTORY
        "histexpand"  => Some(BANGHIST),                                     // c:273 BANGHIST
        "log"         => Some(-HISTNOFUNCTIONS),                             // c:274 -HISTNOFUNCTIONS
        "mailwarn"    => Some(MAILWARNING),                                  // c:275 MAILWARNING
        "onecmd"      => Some(SINGLECOMMAND),                                // c:276 SINGLECOMMAND
        "physical"    => Some(CHASELINKS),                                   // c:277 CHASELINKS
        "promptvars"  => Some(PROMPTSUBST),                                  // c:278 PROMPTSUBST
        "stdin"       => Some(SHINSTDIN),                                    // c:279 SHINSTDIN
        "trackall"    => Some(HASHCMDS),                                     // c:280 HASHCMDS
        _ => None,
    };
    if let Some(optno) = alias_optno {
        return optno;
    }

    // c:708-712 — `if s[0..2] == "no" && getnode(s+2)` → -optno, else getnode(s).
    if let Some(stripped) = s.strip_prefix("no") {                           // c:708
        if let Some(optno) = optno_by_name(stripped) {                       // c:709
            return -optno;                                                   // c:710
        }
    }
    match optno_by_name(&s) {                                                // c:721
        Some(optno) => optno,                                                // c:721
        None => OPT_INVALID,                                                 // c:721
    }
}

// Identify an option letter                                                // c:721
/// Translate a single-letter option flag to its index.
/// Port of `optlookupc(char c)` from Src/options.c:721. Returns 0 for
/// unrecognised letters. Walks the active letter table (`KSH_LETTERS`
/// when `SHOPTIONLETTERS` is set, `zshletters` otherwise) and
/// resolves the canonical name via `optno_by_name`.
pub fn optlookupc(c: char) -> i32 {                                          // c:721
    let letters = if crate::ported::zsh_h::isset(optlookup("shoptionletters")) {
        KSH_LETTERS
    } else {
        zshletters
    };
    for (ch, name, _negated) in letters {
        if *ch == c {
            return optno_by_name(name).unwrap_or(0);                         // c:725
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
/// WARNING: param names don't match C — Rust=(optno, value, _force) vs C=(optno, value, force, new_opts)
pub fn dosetopt(optno: i32, mut value: i32, _force: i32) -> i32 {            // c:735
    if optno == 0 { return -1; }
    let mut idx = optno;
    if idx < 0 {                                                             // c:739
        idx = -idx;
        value = if value != 0 { 0 } else { 1 };                              // c:741
    }
    // c:744 — locate the option name whose FNV hash matches idx.
    let name = ZSH_OPTIONS_SET.iter().find(|n| optlookup(n) == idx);
    match name {
        Some(n) => { opt_state_set(n, value != 0); 0 }                       // c:760 opts[optno] = value
        None => -1,                                                          // c:758
    }
}

/// Build the value of `$-`: a string of the active single-letter
/// option flags (e.g. `"is"` for an interactive script).
/// Port of `dashgetfn(UNUSED(Param pm))` from Src/options.c:890. C source iterates
/// `[FIRST_OPT..=LAST_OPT]` and appends each set option's letter.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn dashgetfn() -> String {
    // C reads `opts[optno]` for each single-letter option (c:890-895).
    // The Rust port's authoritative store is OPTS_LIVE (the same map
    // `opt_state_get` reads), so route each lookup through it.
    let letters = if crate::ported::zsh_h::isset(optlookup("shoptionletters")) {
        KSH_LETTERS
    } else {
        zshletters
    };
    let mut out = String::new();
    for c in (b'A'..=b'z').map(|b| b as char) {
        for (ch, name, negated) in letters {
            if *ch == c {
                let value = opt_state_get(name).unwrap_or(false);            // c:891 `opts[optno]`
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
pub fn printoptionstates(hadplus: bool) {                                    // c:909
    let mut names: Vec<&'static str> = ZSH_OPTIONS_SET.iter().copied().collect();
    names.sort();
    for n in names {                                                         // c:910 scanhashtable
        let value = opt_state_get(n).unwrap_or(false);
        printoptionnodestate(n, value, hadplus);                             // c:916
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
pub fn printoptionnodestate(name: &str, value: bool, hadplus: bool) {        // c:916
    let default_on = default_on_options().contains(&name);                   // c:916 defset
    if hadplus {                                                             // c:920
        let sign = if default_on != value { '-' } else { '+' };              // c:922
        let no_prefix = if default_on { "no" } else { "" };                  // c:923
        println!("set {}o {}{}", sign, no_prefix, name);                     // c:921
    } else {
        if default_on {                                                      // c:927
            println!("no{:<19} {}", name,                                    // c:928
                if value { "off" } else { "on" });
        } else {
            println!("{:<21} {}", name,                                      // c:930
                if value { "on" } else { "off" });
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
pub fn printoptionlist() {                                                   // c:938
    println!();
    println!("Named options:");                                              // c:945
    let mut names: Vec<&'static str> = ZSH_OPTIONS_SET.iter().copied().collect();
    names.sort();
    for n in &names {                                                        // c:946 scanhashtable
        printoptionlist_printoption(n, 0);                                   // c:958
    }
    println!();
    println!("Option aliases:");                                             // c:947
    // c:948 — alias-only walk; static-link path lacks OPT_ALIAS bit
    // tracking on each option, so the alias walk emits nothing here.
    println!();
    println!("Option letters:");                                             // c:949
    let letters = if crate::ported::zsh_h::isset(optlookup("shoptionletters")) {
        KSH_LETTERS
    } else {
        zshletters
    };
    for c in (b'A'..=b'z').map(|b| b as char) {                              // c:950
        for (ch, aname, _negated) in letters {
            if *ch == c {
                print!("  -{}  ", c);                                        // c:953
                printoptionlist_printequiv(optlookup(aname));                // c:954
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
pub fn printoptionlist_printoption(name: &str, _ignored: i32) {              // c:958
    println!("  --{}", name);                                                // c:971
}

/// Direct port of `printoptionlist_printequiv(int optno)` from Src/options.c:971.
/// C body (c:973-977):
/// ```c
/// int isneg = optno < 0;
/// optno *= (isneg ? -1 : 1);
/// printf("  equivalent to --%s%s\n", isneg ? "no-" : "",
///        optns[optno-1].node.nam);
/// ```
pub fn printoptionlist_printequiv(optno: i32) {                              // c:971
    let isneg = optno < 0;                                                   // c:971
    let abs_optno = if isneg { -optno } else { optno };                      // c:974
    let prefix = if isneg { "no-" } else { "" };                             // c:975
    let name = ZSH_OPTIONS_SET.iter()
        .find(|n| optlookup(n) == abs_optno)
        .copied()
        .unwrap_or("?");                                                     // c:976 optns[optno-1].node.nam
    println!("  equivalent to --{}{}", prefix, name);                        // c:975
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
pub fn print_emulate_option(name: &str, value: bool, _fully: bool) {         // c:984
    if !value {                                                              // c:984 !print_emulate_opts[optno]
        print!("no");                                                        // c:995
    }
    println!("{}", name);                                                    // c:996
}

/// Port of `mod_export int emulation;` from `Src/options.c:36`.
/// Current emulation bitmap; one of EMULATE_ZSH / EMULATE_KSH /
/// EMULATE_SH / EMULATE_CSH. Tested via the `EMULATION(bits)` macro
/// at zsh.h:2347 (`(emulation & bits) != 0`). Default 0 — the
/// initial value matches C's zero-initialised BSS slot; `setup_init`
/// calls `installemulation()` (options.c:523) early in startup to
/// flip the right bit.
#[allow(non_upper_case_globals)]
pub static emulation: AtomicI32 = AtomicI32::new(0);                         // c:36

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
pub static EMULATION: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(crate::ported::zsh_h::EMULATE_ZSH);

/// `EMULATE_FULLY` bit (`Src/zsh.h:2354`) tracked separately so
/// `install_emulation_defaults` can re-OR it into the emulation
/// bitmap.
pub static FULLY_EMULATING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub(crate) static ZSH_OPTIONS_SET: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "aliases",
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
        "cbases",
        "cdablevars",
        "cdsilent",
        "chasedots",
        "chaselinks",
        "checkjobs",
        "checkrunningjobs",
        "clobber",
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
        "braceexpand",   // alias of `noignorebraces`
        "dotglob",       // alias of `globdots`
        "hashall",       // alias of `hashcmds`
        "histappend",    // alias of `appendhistory`
        "histexpand",    // alias of `banghist`
        "log",           // alias of `nohistnofunctions`
        "mailwarn",      // alias of `mailwarning`
        "onecmd",        // alias of `singlecommand`
        "physical",      // alias of `chaselinks`
        "promptvars",    // alias of `promptsubst`
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
/// Mirrors the `OPT_INVALID` enum value the C source returns at
/// Src/options.c:714.
pub const OPT_INVALID: i32 = -10000;

/// Port of `static int setemulate_emulation;` from `Src/options.c:496`.
/// The target emulation bitmap, written by `installemulation` and
/// read by the `setemulate` per-option callback (c:518).
static SETEMULATE_EMULATION: std::sync::atomic::AtomicI32 =                  // c:496
    std::sync::atomic::AtomicI32::new(0);

/// Port of `static char *setemulate_opts;` from `Src/options.c:501`.
/// The precomputed `new_opts[]` array `setemulate` writes into. C
/// stores it as a flat `char[]` indexed by `optno`; the Rust port
/// keeps it as a HashMap<String, bool> since the runtime is FNV-
/// hashed instead of densely indexed.
static SETEMULATE_OPTS: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, bool>>,
> = std::sync::OnceLock::new();                                              // c:501

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

static OPTS_LIVE: std::sync::OnceLock<
    std::sync::RwLock<std::collections::HashMap<String, bool>>> =
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
        "aliases" => OPT_EMULATE | (OPT_ALL as u16),                         // c:80
        "aliasfuncdef" => OPT_EMULATE | (OPT_BOURNE as u16),                  // c:81
        "allexport" => OPT_EMULATE,                                          // c:82
        "alwayslastprompt" => OPT_ALL as u16,                                 // c:83
        "alwaystoend" => 0,                                                  // c:84
        "appendcreate" => OPT_EMULATE | (OPT_BOURNE as u16),                  // c:85
        "appendhistory" => OPT_ALL as u16,                                    // c:86
        "autocd" => OPT_EMULATE,                                             // c:87
        "autocontinue" => 0,                                                 // c:88
        "autolist" => OPT_ALL as u16,                                         // c:89
        "automenu" => OPT_ALL as u16,                                         // c:90
        "autonamedirs" => 0,                                                 // c:91
        "autoparamkeys" => OPT_ALL as u16,                                    // c:92
        "autoparamslash" => OPT_ALL as u16,                                   // c:93
        "autopushd" => 0,                                                    // c:94
        "autoremoveslash" => OPT_ALL as u16,                                  // c:95
        "autoresume" => 0,                                                   // c:96
        "badpattern" => OPT_EMULATE | (OPT_NONBOURNE as u16),                 // c:97
        "banghist" => OPT_NONBOURNE as u16,                                   // c:98
        "bareglobqual" => OPT_EMULATE | (OPT_ZSH as u16),                     // c:99
        "bashautolist" => 0,                                                 // c:100
        "bashrematch" => OPT_EMULATE,                                        // c:101
        "beep" => OPT_ALL as u16,                                             // c:102
        "bgnice" => OPT_EMULATE | (OPT_NONBOURNE as u16),                     // c:103
        "braceccl" => 0,                                                     // c:104
        "bsdecho" => OPT_EMULATE,                                            // c:105
        "caseglob" => OPT_ALL as u16,                                         // c:106
        "casematch" => OPT_ALL as u16,                                        // c:107
        "cbases" => 0,                                                       // c:108
        "cdablevars" => OPT_EMULATE,                                         // c:109
        "cdsilent" => 0,                                                     // c:110
        "chasedots" => 0,                                                    // c:111
        "chaselinks" => 0,                                                   // c:112
        "checkjobs" => OPT_EMULATE | (OPT_ZSH as u16),                        // c:113
        "checkrunningjobs" => OPT_EMULATE | (OPT_ZSH as u16),                 // c:114
        "clobber" => OPT_EMULATE | (OPT_ALL as u16),                          // c:115
        "combiningchars" => 0,                                               // c:116
        "completealiases" => 0,                                              // c:117
        "completeinword" => 0,                                               // c:118
        "correct" => 0,                                                      // c:119
        "correctall" => 0,                                                   // c:120
        "cprecedences" => OPT_EMULATE,                                       // c:121
        "cshjunkiehistory" => OPT_EMULATE,                                   // c:122
        "cshjunkieloops" => OPT_EMULATE,                                     // c:123
        "cshjunkiequotes" => OPT_EMULATE,                                    // c:124
        "cshnullcmd" => OPT_EMULATE,                                         // c:125
        "cshnullglob" => OPT_EMULATE,                                        // c:126
        "debugbeforecmd" => OPT_ALL as u16,                                   // c:127
        "emacs" => 0,                                                        // c:128
        "equals" => OPT_EMULATE | (OPT_NONBOURNE as u16),                     // c:129
        "errexit" => OPT_EMULATE,                                            // c:130
        "errreturn" => OPT_EMULATE,                                          // c:131
        "exec" => OPT_ALL as u16,                                             // c:132
        "extendedglob" => OPT_EMULATE,                                       // c:133
        "extendedhistory" => OPT_CSH as u16,                                  // c:134
        "evallineno" => OPT_EMULATE | (OPT_ZSH as u16),                       // c:135
        "flowcontrol" => OPT_ALL as u16,                                      // c:136
        "forcefloat" => 0,                                                   // c:137
        "functionargzero" => OPT_EMULATE | (OPT_NONBOURNE as u16),            // c:138
        "glob" => OPT_EMULATE | (OPT_ALL as u16),                             // c:139
        "globalexport" => OPT_EMULATE | (OPT_ZSH as u16),                     // c:140
        "globalrcs" => OPT_ALL as u16,                                        // c:141
        "globassign" => OPT_EMULATE,                                         // c:142
        "globcomplete" => 0,                                                 // c:143
        "globdots" => OPT_EMULATE,                                           // c:144
        "globstarshort" => OPT_EMULATE,                                      // c:145
        "globsubst" => OPT_EMULATE | (OPT_NONZSH as u16),                     // c:146
        "hashcmds" => OPT_ALL as u16,                                         // c:147
        "hashdirs" => OPT_ALL as u16,                                         // c:148
        "hashexecutablesonly" => 0,                                          // c:149
        "hashlistall" => OPT_ALL as u16,                                      // c:150
        "histallowclobber" => 0,                                             // c:151
        "histbeep" => OPT_ALL as u16,                                         // c:152
        "histexpiredupsfirst" => 0,                                          // c:153
        "histfcntllock" => 0,                                                // c:154
        "histfindnodups" => 0,                                               // c:155
        "histignorealldups" => 0,                                            // c:156
        "histignoredups" => 0,                                               // c:157
        "histignorespace" => 0,                                              // c:158
        "histlexwords" => 0,                                                 // c:159
        "histnofunctions" => 0,                                              // c:160
        "histnostore" => 0,                                                  // c:161
        "histreduceblanks" => 0,                                             // c:162
        "histsavebycopy" => OPT_ALL as u16,                                   // c:163
        "histsavenodups" => 0,                                               // c:164
        "histsubstpattern" => OPT_EMULATE,                                   // c:165
        "histverify" => 0,                                                   // c:166
        "hup" => OPT_EMULATE | (OPT_ZSH as u16),                              // c:167
        "ignorebraces" => OPT_EMULATE | (OPT_SH as u16),                      // c:168
        "ignoreclosebraces" => 0,                                            // c:169
        "ignoreeof" => 0,                                                    // c:170
        "incappendhistory" => 0,                                             // c:171
        "incappendhistorytime" => 0,                                         // c:172
        "interactive" => OPT_SPECIAL as u16,                                  // c:173
        "interactivecomments" => OPT_EMULATE | (OPT_BOURNE as u16),           // c:174
        "ksharrays" => OPT_EMULATE | (OPT_BOURNE as u16),                     // c:175
        "kshautoload" => OPT_EMULATE | (OPT_BOURNE as u16),                   // c:176
        "kshglob" => OPT_EMULATE | (OPT_KSH as u16),                          // c:177
        "kshoptionprint" => OPT_EMULATE | (OPT_KSH as u16),                   // c:178
        "kshtypeset" => OPT_EMULATE | (OPT_BOURNE as u16),                    // c:179
        "kshzerosubscript" => OPT_EMULATE | (OPT_BOURNE as u16),              // c:180
        "listambiguous" => OPT_ALL as u16,                                    // c:181
        "listbeep" => OPT_ALL as u16,                                         // c:182
        "listpacked" => 0,                                                   // c:183
        "listrowsfirst" => 0,                                                // c:184
        "listtypes" => OPT_ALL as u16,                                        // c:185
        "localoptions" => OPT_EMULATE | (OPT_KSH as u16),                     // c:186
        "localloops" => 0,                                                   // c:187
        "localpatterns" => 0,                                                // c:188
        "localtraps" => OPT_EMULATE | (OPT_KSH as u16),                       // c:189
        "loginshell" => OPT_SPECIAL as u16,                                   // c:190
        "longlistjobs" => 0,                                                 // c:191
        "magicequalsubst" => OPT_EMULATE,                                    // c:192
        "mailwarning" => 0,                                                  // c:193
        "markdirs" => 0,                                                     // c:194
        "menucomplete" => 0,                                                 // c:195
        "monitor" => OPT_SPECIAL as u16,                                      // c:196
        "multibyte" => 0,                                                    // c:197
        "multifuncdef" => OPT_EMULATE | (OPT_ZSH as u16),                     // c:198
        "multios" => OPT_EMULATE | (OPT_ZSH as u16),                          // c:199
        "nomatch" => OPT_EMULATE | (OPT_NONBOURNE as u16),                    // c:200
        "notify" => OPT_EMULATE | (OPT_ZSH as u16),                           // c:201
        "nullglob" => OPT_EMULATE,                                           // c:202
        "numericglobsort" => 0,                                              // c:203
        "octalzeroes" => OPT_EMULATE | (OPT_SH as u16),                       // c:204
        "overstrike" => 0,                                                   // c:205
        "pathdirs" => 0,                                                     // c:206
        "pathscript" => OPT_EMULATE | (OPT_BOURNE as u16),                    // c:207
        "pipefail" => OPT_EMULATE,                                           // c:208
        "posixaliases" => OPT_EMULATE | (OPT_BOURNE as u16),                  // c:209
        "posixargzero" => OPT_EMULATE | (OPT_BOURNE as u16),                  // c:210
        "posixbuiltins" => OPT_EMULATE | (OPT_BOURNE as u16),                 // c:211
        "posixcd" => OPT_EMULATE | (OPT_BOURNE as u16),                       // c:212
        "posixidentifiers" => OPT_EMULATE | (OPT_BOURNE as u16),              // c:213
        "posixjobs" => OPT_EMULATE | (OPT_BOURNE as u16),                     // c:214
        "posixstrings" => OPT_EMULATE | (OPT_BOURNE as u16),                  // c:215
        "posixtraps" => OPT_EMULATE | (OPT_BOURNE as u16),                    // c:216
        "printeightbit" => 0,                                                // c:217
        "printexitvalue" => 0,                                               // c:218
        "privileged" => OPT_SPECIAL as u16,                                   // c:219
        "promptbang" => OPT_EMULATE | (OPT_KSH as u16),                       // c:220
        "promptcr" => OPT_ALL as u16,                                         // c:221
        "promptpercent" => OPT_EMULATE | (OPT_NONBOURNE as u16),              // c:222
        "promptsp" => OPT_ALL as u16,                                         // c:223
        "promptsubst" => OPT_EMULATE | (OPT_BOURNE as u16),                   // c:224
        "pushdignoredups" => 0,                                              // c:225
        "pushdminus" => 0,                                                   // c:226
        "pushdsilent" => 0,                                                  // c:227
        "pushdtohome" => 0,                                                  // c:228
        "rcexpandparam" => OPT_EMULATE,                                      // c:229
        "rcquotes" => 0,                                                     // c:230
        "rcs" => OPT_ALL as u16,                                              // c:231
        "recexact" => 0,                                                     // c:232
        "rematchpcre" => 0,                                                  // c:233
        "restricted" => OPT_SPECIAL as u16,                                   // c:234
        "rmstarsilent" => OPT_EMULATE | (OPT_BOURNE as u16),                  // c:235
        "rmstarwait" => 0,                                                   // c:236
        "sharehistory" => 0,                                                 // c:237
        "shfileexpansion" => OPT_EMULATE | (OPT_BOURNE as u16),               // c:238
        "shglob" => OPT_EMULATE | (OPT_BOURNE as u16),                        // c:239
        "shinstdin" => OPT_SPECIAL as u16,                                    // c:240
        "shnullcmd" => OPT_EMULATE | (OPT_BOURNE as u16),                     // c:241
        "shoptionletters" => OPT_EMULATE | (OPT_BOURNE as u16),               // c:242
        "shortloops" => OPT_EMULATE | (OPT_NONBOURNE as u16),                 // c:243
        "shortrepeat" => OPT_EMULATE | (OPT_ZSH as u16),                      // c:244
        "shwordsplit" => OPT_EMULATE | (OPT_BOURNE as u16),                   // c:245
        "singlecommand" => OPT_SPECIAL as u16,                                // c:246
        "singlelinezle" => 0,                                                // c:247
        "sourcetrace" => 0,                                                  // c:248
        "sunkeyboardhack" => 0,                                              // c:249
        "transientrprompt" => 0,                                             // c:250
        "trapsasync" => 0,                                                   // c:251
        "typesetsilent" => OPT_EMULATE | (OPT_BOURNE as u16),                 // c:252
        "unset" => OPT_EMULATE | (OPT_BSHELL as u16),                         // c:253
        "verbose" => OPT_EMULATE,                                            // c:254
        "vi" => 0,                                                           // c:255
        "warncreateglobal" => 0,                                             // c:256
        "warnnestedvar" => 0,                                                // c:257
        "xtrace" => OPT_EMULATE,                                             // c:258
        "zle" => OPT_SPECIAL as u16,                                          // c:259
        "dvorak" => 0,                                                       // c:260
        _ => 0,
    }
}

/// !!! RUST-ONLY HELPER — see WARNING block above.
/// Returns options that are on by default for zsh emulation.
pub(crate) fn default_on_options() -> std::collections::HashSet<&'static str> {
    // Default-on options have OPT_ZSH bit set in their flags
    let zsh_emu = crate::ported::zsh_h::EMULATE_ZSH as u16;
    let mut set = std::collections::HashSet::new();
    for name in ZSH_OPTIONS_SET.iter() {
        let flags = optns_flags(name);
        if (flags & zsh_emu) != 0 && (flags & OPT_SPECIAL) == 0 {
            set.insert(*name);
        }
    }
    set
}

fn setemulate_opts_lock()
    -> &'static std::sync::Mutex<std::collections::HashMap<String, bool>>
{
    SETEMULATE_OPTS
        .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Reverse lookup `index_to_name` to map a canonical option name
/// back to its C-fixed optno (one of the `zh::OPT_*` constants).
/// Rust-only architectural helper: C iterates `optiontab` (a
/// HashTable keyed by name) and reads `Optname.optno` — the Rust
/// port stores the same mapping inverted in `index_to_name`'s match
/// arms; this fn walks `0..OPT_SIZE` and matches the first idx that
/// names to `name`.
fn optno_by_name(name: &str) -> Option<i32> {
    for idx in 1..OPT_SIZE {
        if index_to_name(idx) == Some(name) {
            return Some(idx);
        }
    }
    None
}

/// !!! RUST-ONLY HELPER — see WARNING block above. Read the live
/// state of `name` from the process-wide option store.
pub fn opt_state_get(name: &str) -> Option<bool> {
    let m = OPTS_LIVE.get_or_init(|| std::sync::RwLock::new(
        std::collections::HashMap::new()));
    m.read().ok().and_then(|g| g.get(name).copied())
}

/// !!! RUST-ONLY HELPER — see WARNING block above. Write `value`
/// into the process-wide option store.
pub fn opt_state_set(name: &str, value: bool) {
    let m = OPTS_LIVE.get_or_init(|| std::sync::RwLock::new(
        std::collections::HashMap::new()));
    if let Ok(mut g) = m.write() {
        g.insert(name.to_string(), value);
    }
}

/// !!! RUST-ONLY HELPER — see WARNING block above. Remove an entry
/// from the process-wide option store (`!= isset(opt)`).
pub fn opt_state_unset(name: &str) {
    let m = OPTS_LIVE.get_or_init(|| std::sync::RwLock::new(
        std::collections::HashMap::new()));
    if let Ok(mut g) = m.write() {
        g.remove(name);
    }
}

/// !!! RUST-ONLY HELPER — see WARNING block above. Snapshot the
/// full option store. Caller gets a HashMap<String, bool>.
pub fn opt_state_snapshot() -> std::collections::HashMap<String, bool> {
    let m = OPTS_LIVE.get_or_init(|| std::sync::RwLock::new(
        std::collections::HashMap::new()));
    m.read().map(|g| g.clone()).unwrap_or_default()
}

/// !!! RUST-ONLY HELPER — see WARNING block above. Number of entries
/// currently in the option store (= count of options that have been
/// touched by set/setopt/unset).
pub fn opt_state_len() -> usize {
    let m = OPTS_LIVE.get_or_init(|| std::sync::RwLock::new(
        std::collections::HashMap::new()));
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
pub fn list_emulate_options(cmdopts: &std::collections::HashMap<String, bool>,
                            fully: bool) {                                   // c:1003
    let mut names: Vec<&'static str> = ZSH_OPTIONS_SET.iter().copied().collect();
    names.sort();
    for n in names {                                                         // c:1004 scanhashtable
        let value = cmdopts.get(n).copied().unwrap_or(false);
        print_emulate_option(n, value, fully);                               // c:986 callback
    }
}

/// Map a canonical zsh option index (the constants in `zsh_h.rs`
/// like `VIMODE = 180`, `POSIXBUILTINS = 135`) back to the option's
/// lowercase name. Mirrors the C `optns[]` table in `Src/options.c`
/// indexed by `OPT_*` enum value (zsh.h:2050+).
///
/// Rust-only architectural helper: C iterates `optiontab` (a
/// HashTable keyed by name) and reads `Optname.optno` to get the
/// index; the reverse direction needs an explicit `optno -> name`
/// table, which doesn't exist in the C source — there it's just
/// implicit in the order of `OPT_*` enum entries paired with the
/// `optns[]` array. This match collapses both into one lookup.
fn index_to_name(idx: i32) -> Option<&'static str> {
    let i = idx.unsigned_abs() as i32;
    Some(match i {
        x if x == zh::ALIASESOPT          => "aliases",
        x if x == zh::ALIASFUNCDEF        => "aliasfuncdef",
        x if x == zh::ALLEXPORT           => "allexport",
        x if x == zh::ALWAYSLASTPROMPT    => "alwayslastprompt",
        x if x == zh::ALWAYSTOEND         => "alwaystoend",
        x if x == zh::APPENDCREATE        => "appendcreate",
        x if x == zh::APPENDHISTORY       => "appendhistory",
        x if x == zh::AUTOCD              => "autocd",
        x if x == zh::AUTOCONTINUE        => "autocontinue",
        x if x == zh::AUTOLIST            => "autolist",
        x if x == zh::AUTOMENU            => "automenu",
        x if x == zh::AUTONAMEDIRS        => "autonamedirs",
        x if x == zh::AUTOPARAMKEYS       => "autoparamkeys",
        x if x == zh::AUTOPARAMSLASH      => "autoparamslash",
        x if x == zh::AUTOPUSHD           => "autopushd",
        x if x == zh::AUTOREMOVESLASH     => "autoremoveslash",
        x if x == zh::AUTORESUME          => "autoresume",
        x if x == zh::BADPATTERN          => "badpattern",
        x if x == zh::BANGHIST            => "banghist",
        x if x == zh::BAREGLOBQUAL        => "bareglobqual",
        x if x == zh::BASHAUTOLIST        => "bashautolist",
        x if x == zh::BASHREMATCH         => "bashrematch",
        x if x == zh::BEEP                => "beep",
        x if x == zh::BGNICE              => "bgnice",
        x if x == zh::BRACECCL            => "braceccl",
        x if x == zh::BSDECHO             => "bsdecho",
        x if x == zh::CASEGLOB            => "caseglob",
        x if x == zh::CASEMATCH           => "casematch",
        x if x == zh::CDABLEVARS          => "cdablevars",
        x if x == zh::CHASEDOTS           => "chasedots",
        x if x == zh::CHASELINKS          => "chaselinks",
        x if x == zh::CHECKJOBS           => "checkjobs",
        x if x == zh::CLOBBER             => "clobber",
        x if x == zh::COMBININGCHARS      => "combiningchars",
        x if x == zh::COMPLETEALIASES     => "completealiases",
        x if x == zh::COMPLETEINWORD      => "completeinword",
        x if x == zh::CORRECT             => "correct",
        x if x == zh::CORRECTALL          => "correctall",
        x if x == zh::CPRECEDENCES        => "cprecedences",
        x if x == zh::CSHJUNKIEHISTORY    => "cshjunkiehistory",
        x if x == zh::CSHJUNKIELOOPS      => "cshjunkieloops",
        x if x == zh::CSHJUNKIEQUOTES     => "cshjunkiequotes",
        x if x == zh::CSHNULLCMD          => "cshnullcmd",
        x if x == zh::CSHNULLGLOB         => "cshnullglob",
        x if x == zh::DEBUGBEFORECMD      => "debugbeforecmd",
        x if x == zh::EMACSMODE           => "emacsmode",
        x if x == zh::EQUALSOPT           => "equals",
        x if x == zh::ERREXIT             => "errexit",
        x if x == zh::ERRRETURN           => "errreturn",
        x if x == zh::EXTENDEDGLOB        => "extendedglob",
        x if x == zh::EXTENDEDHISTORY     => "extendedhistory",
        x if x == zh::FLOWCONTROL         => "flowcontrol",
        x if x == zh::FORCEFLOAT          => "forcefloat",
        x if x == zh::FUNCTIONARGZERO     => "functionargzero",
        x if x == zh::GLOBOPT             => "glob",
        x if x == zh::GLOBALEXPORT        => "globalexport",
        x if x == zh::GLOBALRCS           => "globalrcs",
        x if x == zh::GLOBASSIGN          => "globassign",
        x if x == zh::GLOBCOMPLETE        => "globcomplete",
        x if x == zh::GLOBDOTS            => "globdots",
        x if x == zh::GLOBSTARSHORT       => "globstarshort",
        x if x == zh::GLOBSUBST           => "globsubst",
        x if x == zh::HASHCMDS            => "hashcmds",
        x if x == zh::HASHDIRS            => "hashdirs",
        x if x == zh::HASHEXECUTABLESONLY => "hashexecutablesonly",
        x if x == zh::HASHLISTALL         => "hashlistall",
        x if x == zh::HISTALLOWCLOBBER    => "histallowclobber",
        x if x == zh::HISTBEEP            => "histbeep",
        x if x == zh::HISTEXPIREDUPSFIRST => "histexpiredupsfirst",
        x if x == zh::HISTFCNTLLOCK       => "histfcntllock",
        x if x == zh::HISTFINDNODUPS      => "histfindnodups",
        x if x == zh::HISTIGNOREALLDUPS   => "histignorealldups",
        x if x == zh::HISTIGNOREDUPS      => "histignoredups",
        x if x == zh::HISTIGNORESPACE     => "histignorespace",
        x if x == zh::HISTLEXWORDS        => "histlexwords",
        x if x == zh::HISTNOFUNCTIONS     => "histnofunctions",
        x if x == zh::HISTNOSTORE         => "histnostore",
        x if x == zh::HISTREDUCEBLANKS    => "histreduceblanks",
        x if x == zh::HISTSAVEBYCOPY      => "histsavebycopy",
        x if x == zh::HISTSAVENODUPS      => "histsavenodups",
        x if x == zh::HISTSUBSTPATTERN    => "histsubstpattern",
        x if x == zh::HISTVERIFY          => "histverify",
        x if x == zh::HUP                 => "hup",
        x if x == zh::IGNOREBRACES        => "ignorebraces",
        x if x == zh::IGNORECLOSEBRACES   => "ignoreclosebraces",
        x if x == zh::IGNOREEOF           => "ignoreeof",
        x if x == zh::INCAPPENDHISTORY    => "incappendhistory",
        x if x == zh::INCAPPENDHISTORYTIME => "incappendhistorytime",
        x if x == zh::INTERACTIVE         => "interactive",
        x if x == zh::INTERACTIVECOMMENTS => "interactivecomments",
        x if x == zh::KSHARRAYS           => "ksharrays",
        x if x == zh::KSHAUTOLOAD         => "kshautoload",
        x if x == zh::KSHGLOB             => "kshglob",
        x if x == zh::KSHOPTIONPRINT      => "kshoptionprint",
        x if x == zh::KSHTYPESET          => "kshtypeset",
        x if x == zh::KSHZEROSUBSCRIPT    => "kshzerosubscript",
        x if x == zh::LISTAMBIGUOUS       => "listambiguous",
        x if x == zh::LISTBEEP            => "listbeep",
        x if x == zh::LISTPACKED          => "listpacked",
        x if x == zh::LISTROWSFIRST       => "listrowsfirst",
        x if x == zh::LISTTYPES           => "listtypes",
        x if x == zh::LOCALLOOPS          => "localloops",
        x if x == zh::LOCALOPTIONS        => "localoptions",
        x if x == zh::LOCALPATTERNS       => "localpatterns",
        x if x == zh::LOCALTRAPS          => "localtraps",
        x if x == zh::LOGINSHELL          => "loginshell",
        x if x == zh::LONGLISTJOBS        => "longlistjobs",
        x if x == zh::MAGICEQUALSUBST     => "magicequalsubst",
        x if x == zh::MAILWARNING         => "mailwarning",
        x if x == zh::MARKDIRS            => "markdirs",
        x if x == zh::MENUCOMPLETE        => "menucomplete",
        x if x == zh::MONITOR             => "monitor",
        x if x == zh::MULTIBYTE           => "multibyte",
        x if x == zh::MULTIFUNCDEF        => "multifuncdef",
        x if x == zh::MULTIOS             => "multios",
        x if x == zh::NOMATCH             => "nomatch",
        x if x == zh::NOTIFY              => "notify",
        x if x == zh::NULLGLOB            => "nullglob",
        x if x == zh::NUMERICGLOBSORT     => "numericglobsort",
        x if x == zh::OCTALZEROES         => "octalzeroes",
        x if x == zh::OVERSTRIKE          => "overstrike",
        x if x == zh::PATHDIRS            => "pathdirs",
        x if x == zh::PATHSCRIPT          => "pathscript",
        x if x == zh::PIPEFAIL            => "pipefail",
        x if x == zh::POSIXALIASES        => "posixaliases",
        x if x == zh::POSIXARGZERO        => "posixargzero",
        x if x == zh::POSIXBUILTINS       => "posixbuiltins",
        x if x == zh::POSIXCD             => "posixcd",
        x if x == zh::POSIXIDENTIFIERS    => "posixidentifiers",
        x if x == zh::POSIXJOBS           => "posixjobs",
        x if x == zh::POSIXSTRINGS        => "posixstrings",
        x if x == zh::POSIXTRAPS          => "posixtraps",
        x if x == zh::PRINTEIGHTBIT       => "printeightbit",
        x if x == zh::PRINTEXITVALUE      => "printexitvalue",
        x if x == zh::PRIVILEGED          => "privileged",
        x if x == zh::PROMPTBANG          => "promptbang",
        x if x == zh::PROMPTCR            => "promptcr",
        x if x == zh::PROMPTPERCENT       => "promptpercent",
        x if x == zh::PROMPTSP            => "promptsp",
        x if x == zh::PROMPTSUBST         => "promptsubst",
        x if x == zh::PUSHDIGNOREDUPS     => "pushdignoredups",
        x if x == zh::PUSHDMINUS          => "pushdminus",
        x if x == zh::PUSHDSILENT         => "pushdsilent",
        x if x == zh::PUSHDTOHOME         => "pushdtohome",
        x if x == zh::RCEXPANDPARAM       => "rcexpandparam",
        x if x == zh::RCQUOTES            => "rcquotes",
        x if x == zh::RCS                 => "rcs",
        x if x == zh::RECEXACT            => "recexact",
        x if x == zh::REMATCHPCRE         => "rematchpcre",
        x if x == zh::RESTRICTED          => "restricted",
        x if x == zh::RMSTARSILENT        => "rmstarsilent",
        x if x == zh::RMSTARWAIT          => "rmstarwait",
        x if x == zh::SHAREHISTORY        => "sharehistory",
        x if x == zh::SHFILEEXPANSION     => "shfileexpansion",
        x if x == zh::SHGLOB              => "shglob",
        x if x == zh::SHINSTDIN           => "shinstdin",
        x if x == zh::SHNULLCMD           => "shnullcmd",
        x if x == zh::SHOPTIONLETTERS     => "shoptionletters",
        x if x == zh::SHORTLOOPS          => "shortloops",
        x if x == zh::SHORTREPEAT         => "shortrepeat",
        x if x == zh::SHWORDSPLIT         => "shwordsplit",
        x if x == zh::SINGLECOMMAND       => "singlecommand",
        x if x == zh::SINGLELINEZLE       => "singlelinezle",
        x if x == zh::SOURCETRACE         => "sourcetrace",
        x if x == zh::SUNKEYBOARDHACK     => "sunkeyboardhack",
        x if x == zh::TRANSIENTRPROMPT    => "transientrprompt",
        x if x == zh::TRAPSASYNC          => "trapsasync",
        x if x == zh::TYPESETSILENT       => "typesetsilent",
        x if x == zh::UNSET               => "unset",
        x if x == zh::VERBOSE             => "verbose",
        x if x == zh::VIMODE              => "vimode",
        x if x == zh::WARNCREATEGLOBAL    => "warncreateglobal",
        x if x == zh::WARNNESTEDVAR       => "warnnestedvar",
        x if x == zh::XTRACE              => "xtrace",
        x if x == zh::USEZLE              => "zle",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use crate::zsh_h::isset;
    use super::*;

    // Tests share global OPTS_LIVE state; serialize via this mutex so
    // parallel cargo-test threads don't stomp each other's option-state
    // setup (e.g. test_emulation switching to `sh` while test_default
    // checks `exec` set under zsh defaults).
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_default_options() {
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
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();
        dosetopt(optlookup("xtrace"), if true { 1 } else { 0 }, 0);
        assert!(isset(optlookup("xtrace")));
        dosetopt(optlookup("xtrace"), if false { 1 } else { 0 }, 0);
        assert!(!isset(optlookup("xtrace")));
    }

    #[test]
    fn test_no_prefix() {
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
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();
        assert_eq!(optlookup("GLOB"), optlookup("glob"));
        assert_eq!(optlookup("GlOb"), optlookup("glob"));
    }

    #[test]
    fn test_underscore_ignored() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();
        assert_eq!(optlookup("auto_list"), optlookup("autolist"));
        assert_eq!(optlookup("AUTO_LIST"), optlookup("autolist"));
    }

    #[test]
    fn test_option_alias() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();
        // `braceexpand` aliases to `noignorebraces` (optns[]:269 -IGNOREBRACES).
        dosetopt(optlookup("braceexpand"), if true { 1 } else { 0 }, 0);
        assert!(!isset(optlookup("ignorebraces")));
    }

    #[test]
    fn test_single_letter() {
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
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        emulate("sh", true);
        assert_eq!(
            EMULATION.load(std::sync::atomic::Ordering::Relaxed),
            crate::ported::zsh_h::EMULATE_SH
        );
        assert!(isset(optlookup("shwordsplit")));

        emulate("zsh", true);
        assert_eq!(
            EMULATION.load(std::sync::atomic::Ordering::Relaxed),
            crate::ported::zsh_h::EMULATE_ZSH
        );
    }

    #[test]
    fn test_dash_string() {
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
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createoptiontable();
        // The canonicalised name "autolist" is the same option whether
        // written AUTO_LIST, AutoList, auto__list — `optlookup`
        // does the canonicalisation matching `optlookup()` (c:684).
        assert_eq!(optlookup("AUTO_LIST"), optlookup("autolist"));
        assert_eq!(optlookup("AutoList"), optlookup("autolist"));
        assert_eq!(optlookup("auto__list"), optlookup("autolist"));
    }
}
