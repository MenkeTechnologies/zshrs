//! Per-shell output formats for builtins whose listing differs between
//! the shells zshrs stands in for.
//!
//! **zshrs-only — no zsh C counterpart.** `Src/builtin.c` prints one
//! format, zsh's. A drop-in has to print what ITS shell prints, because
//! these strings are parsed by scripts (`export -p` is specified to be
//! re-inputtable) and read by people (`times`, `kill -l`).
//!
//! Every format here was captured from the reference binary in an empty
//! environment, and is re-checked against it by the parity harness.

use crate::extensions::emulation_startup::{personality, Personality};
use crate::ported::builtin::{bin_enable, bin_trap, traps_table, BUILTINS_DISABLED};
use crate::ported::exec::hashcmd;
use crate::ported::hashtable::{cmdnamtab_lock, emptycmdnamtable};
use crate::ported::hashtable_h::BIN_DISABLE;
use crate::ported::params::getsparam;
use crate::ported::utils::zwarnnam;
use crate::ported::zsh_h::{cmdnam, hashnode, options, HASHED, MAX_OPS, OPT_ISSET};
use crate::ported::builtin::LASTVAL;
use std::sync::atomic::Ordering;


/// How a shell renders one `times` field.
///
/// Captured with `<shell> -c times`:
///
/// | shell | output |
/// |-------|--------|
/// | zsh 5.9 | `0m0.00s 0m0.00s` |
/// | bash 5.3 | `0m0.008s 0m0.011s` |
/// | ksh93u+m | `0m00.000s 0m00.000s` |
/// | mksh | `0m00.00s 0m00.00s` |
/// | dash | `0m0.000000s 0m0.000000s` |
pub fn times_field(ticks: i64, clktck: i64) -> String {
    let clktck = if clktck <= 0 { 100 } else { clktck };
    let mins = ticks / (60 * clktck);
    let p = personality();
    // zsh computes the seconds field as `X/clktck % clktck`
    // (c:Src/builtin.c:7315-7318) — modulo the CLOCK TICK, not 60 — so a
    // shell that has burned 125s of CPU prints `2m25.00s`. Every other
    // shell here takes `% 60` and prints `2m5s`. The zsh arm keeps the
    // upstream arithmetic verbatim; the drop-ins take their own.
    let secs = if p == Personality::Zsh {
        (ticks / clktck) % clktck
    } else {
        (ticks / clktck) % 60
    };
    match p {
        // bash: three decimals (milliseconds).
        Personality::Bash => {
            let ms = (ticks * 1000 / clktck) % 1000;
            format!("{mins}m{secs}.{ms:03}s")
        }
        // dash (POSIX sh on Debian): six decimals (microseconds).
        Personality::Dash | Personality::Sh => {
            let us = (ticks * 1_000_000 / clktck) % 1_000_000;
            format!("{mins}m{secs}.{us:06}s")
        }
        // ksh93u+m: milliseconds AND a zero-padded seconds field. (The
        // legacy 93u+ 2012 line printed a labelled `user\t0m0.00s`
        // instead; zshrs follows the maintained fork, as it does for the
        // default alias set.)
        Personality::Ksh93 => {
            let ms = (ticks * 1000 / clktck) % 1000;
            format!("{mins}m{secs:02}.{ms:03}s")
        }
        // mksh: centiseconds, but the SECONDS field is zero-padded to two.
        Personality::Mksh | Personality::Pdksh => {
            let cs = (ticks * 100 / clktck) % 100;
            format!("{mins}m{secs:02}.{cs:02}s")
        }
        // zsh, ksh93 and the POSIX shells: centiseconds, unpadded seconds.
        // c:Src/builtin.c:7315-7318.
        _ => {
            let cs = (ticks * 100 / clktck) % 100;
            format!("{mins}m{secs}.{cs:02}s")
        }
    }
}

/// The signals bare `kill -l` lists, as `(number, name)`: zsh's
/// `sigs[1..SIGCOUNT]` followed, where SIGRTMIN/SIGRTMAX exist, by
/// `rtsigname(sig, 0)` for every real-time signal
/// (c:Src/jobs.c:2896-2902). bin_kill and bash's `trap -l` both list it.
pub fn kill_l_signals() -> Vec<(i32, String)> {
    use crate::ported::signals_h::{sigs_name, SIGCOUNT};
    #[allow(unused_mut)]
    let mut sigs: Vec<(i32, String)> = (1..=SIGCOUNT)
        .filter_map(|s| sigs_name(s).map(|n| (s, n.to_string())))
        .collect();
    #[cfg(target_os = "linux")]
    for s in libc::SIGRTMIN()..=libc::SIGRTMAX() {
        sigs.push((s, crate::ported::signals::rtsigname(s))); // c:2900-2901
    }
    sigs
}

/// `kill -l`, rendered the way this personality's shell renders it.
///
/// Returns `None` for native zsh, whose own single-line listing stays in
/// `Src/jobs.c`'s port. Captured from each reference in an empty
/// environment; the SIGNAL SET itself comes from the running system, not
/// a table, because names and numbers differ between macOS and Linux
/// (macOS has `EMT`/`INFO`, Linux has `PWR`/`STKFLT`).
///
/// | shell | shape |
/// |-------|-------|
/// | bash 5.3 | `%2d) SIG%s`, five per row, tab-separated |
/// | ksh93u+m | every slot 1..NSIG-1, one bare name per line, `SIG<n>` where unnamed |
/// | dash | every slot 0..NSIG-1, one per line, the number where unnamed |
/// | mksh | two columns of `%2d %6s %s` with the strsignal text |
pub fn kill_list(sigs: &[(i32, String)]) -> Option<String> {
    let p = personality();
    // zsh and ksh93u+m name Linux's signal 29 `POLL` (signames2.awk lets
    // POLL replace IO); bash's and dash's mksignames write IO after POLL,
    // so IO wins there.
    let sigs: Vec<(i32, &str)> = sigs
        .iter()
        .map(|(num, name)| {
            let io = name == "POLL" && p != Personality::Ksh93;
            (*num, if io { "IO" } else { name.as_str() })
        })
        .collect();
    // NSIG: one past the highest signal listed (RTMAX on Linux).
    let nsig = sigs.last().map_or(1, |(num, _)| num + 1);
    let mut out = String::new();
    match p {
        Personality::Zsh => return None,
        // bash: `%2d) SIG%s` five to a row, tab after every entry
        // including the last on a row, so the final row ends in a tab.
        Personality::Bash => {
            for (i, (num, name)) in sigs.iter().enumerate() {
                out.push_str(&format!("{num:2}) SIG{name}"));
                if i % 5 == 4 {
                    out.push('\n');
                } else {
                    out.push('\t');
                }
            }
            if !sigs.len().is_multiple_of(5) {
                out.push('\n');
            }
        }
        // dash (POSIX sh on Debian): every slot from 0 to NSIG-1, one per
        // line, a slot its mksignames.c has no name for printed as the
        // number. That table knows no STKFLT, and glibc keeps signals 32
        // and 33 for itself, so Linux shows `16`, `32` and `33`.
        Personality::Dash | Personality::Sh => {
            out.push_str("0\n");
            for slot in 1..nsig {
                match sigs.iter().find(|(num, name)| *num == slot && *name != "STKFLT") {
                    Some((_, name)) => out.push_str(name),
                    None => out.push_str(&slot.to_string()),
                }
                out.push('\n');
            }
        }
        // ksh93u+m: every slot from 1 to NSIG-1, one bare name per line,
        // no number and no `SIG` — except an unnamed slot, which prints as
        // `SIG<n>` (glibc's reserved 32 and 33). It also keeps the
        // historical `IOT` spelling for signal 6, where every other shell
        // here says `ABRT`.
        Personality::Ksh93 => {
            for slot in 1..nsig {
                match sigs.iter().find(|(num, _)| *num == slot) {
                    Some(_) if slot == 6 => out.push_str("IOT"),
                    Some((_, name)) => out.push_str(name),
                    None => out.push_str(&format!("SIG{slot}")),
                }
                out.push('\n');
            }
        }
        Personality::Csh => {
            for (_, name) in &sigs {
                out.push_str(name);
                out.push('\n');
            }
        }
        // mksh: two side-by-side columns, the left half then the right,
        // each `%2d %6s %s` with the system's signal description. The
        // split point is the midpoint, rounded up.
        Personality::Mksh | Personality::Pdksh => {
            let half = sigs.len().div_ceil(2);
            let left_width = 40usize;
            for i in 0..half {
                let (num, name) = &sigs[i];
                let cell = format!("{num:2} {name:>6} {}", signal_description(*num));
                match sigs.get(i + half) {
                    Some((rnum, rname)) => out.push_str(&format!(
                        "{cell:<left_width$}{rnum:2} {rname:>6} {}\n",
                        signal_description(*rnum)
                    )),
                    None => {
                        out.push_str(cell.trim_end());
                        out.push('\n');
                    }
                }
            }
        }
    }
    Some(out)
}

/// The system's description for a signal — mksh prints `strsignal(3)`'s
/// text ("Hangup", "Broken pipe"), which is locale- and platform-defined
/// rather than something zshrs can tabulate.
fn signal_description(num: i32) -> String {
    // SAFETY: `strsignal` returns a pointer to a static, NUL-terminated
    // string for any int; it is not thread-safe against a concurrent
    // `setlocale`, which the shell does not do while listing signals.
    let p = unsafe { libc::strsignal(num) };
    if p.is_null() {
        return String::new();
    }
    let text = unsafe { std::ffi::CStr::from_ptr(p) }
        .to_string_lossy()
        .into_owned();
    // macOS's strsignal appends the number ("Hangup: 1"); glibc's does
    // not, and neither does mksh's own listing. Strip a trailing
    // `": <digits>"` so both platforms render mksh's text.
    match text.rsplit_once(": ") {
        Some((head, tail)) if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) => {
            head.to_string()
        }
        _ => text,
    }
}

/// The header line a shell prints before its `hash` listing, if any.
///
/// Only bash has one: `hits\tcommand`, then `%7d\t%s` per entry. ksh,
/// mksh and zsh print `name=path` with no header; dash prints the bare
/// path.
pub fn hash_header() -> Option<&'static str> {
    match personality() {
        Personality::Bash => Some("hits\tcommand"),
        _ => None,
    }
}

/// bash's per-command `hash` hit counters, keyed by command name.
///
/// bash bumps the count each time it runs the hashed path; zsh keeps none.
/// Only meaningful under `--bash`; entries die with the table (`hash -r`).
static BASH_HASH_HITS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, u32>>> =
    std::sync::LazyLock::new(Default::default);

/// Count one execution of the hashed command `name` (bash's `hits` column).
pub fn bash_hash_hit(name: &str) {
    if personality() == Personality::Bash {
        if let Ok(mut m) = BASH_HASH_HITS.lock() {
            *m.entry(name.to_string()).or_insert(0) += 1;
        }
    }
}

/// Forget the hit counter of `name` (`hash -d`), or of every command (`hash -r`).
pub fn bash_hash_forget(name: Option<&str>) {
    if let Ok(mut m) = BASH_HASH_HITS.lock() {
        match name {
            Some(n) => {
                m.remove(n);
            }
            None => m.clear(),
        }
    }
}

fn bash_hash_hits(name: &str) -> u32 {
    BASH_HASH_HITS
        .lock()
        .ok()
        .and_then(|m| m.get(name).copied())
        .unwrap_or(0)
}

/// One `hash` listing row, or `None` to use zsh's `name=path` form.
///
/// bash's first column is the per-command hit counter (`bash_hash_hit`).
pub fn hash_entry(name: &str, path: &str) -> Option<String> {
    match personality() {
        Personality::Bash => Some(format!("{:>4}\t{path}", bash_hash_hits(name))),
        // dash (POSIX sh on Debian) prints the resolved path alone, no
        // name and no `=`.
        Personality::Dash | Personality::Sh => Some(path.to_string()),
        _ => None,
    }
}

/// bash 5.2's builtin table in `enable` listing order, with whether each is
/// a POSIX special builtin (`enable -s`).
pub const BASH_BUILTINS: &[(&str, bool)] = &[
    (".", true), (":", true), ("[", false), ("alias", false), ("bg", false),
    ("bind", false), ("break", true), ("builtin", false), ("caller", false),
    ("cd", false), ("command", false), ("compgen", false), ("complete", false),
    ("compopt", false), ("continue", true), ("declare", false), ("dirs", false),
    ("disown", false), ("echo", false), ("enable", false), ("eval", true),
    ("exec", true), ("exit", true), ("export", true), ("false", false),
    ("fc", false), ("fg", false), ("getopts", false), ("hash", false),
    ("help", false), ("history", false), ("jobs", false), ("kill", false),
    ("let", false), ("local", false), ("logout", false), ("mapfile", false),
    ("popd", false), ("printf", false), ("pushd", false), ("pwd", false),
    ("read", false), ("readarray", false), ("readonly", true), ("return", true),
    ("set", true), ("shift", true), ("shopt", false), ("source", true),
    ("suspend", false), ("test", false), ("times", true), ("trap", true),
    ("true", false), ("type", false), ("typeset", false), ("ulimit", false),
    ("umask", false), ("unalias", false), ("unset", true), ("wait", false),
];

/// !!! EMULATION-ONLY (no C counterpart) !!! bash's `hash`:
/// `hash [-lr] [-p path] [-dt] [name ...]` over the command table, with the
/// `hits<TAB>command` listing and bash's "hash table empty" notice.
pub fn bash_hash(argv: &[String], ops: &options) -> i32 {
    use crate::extensions::emulation_output as eo;
    let path_dirs = || -> Vec<String> {
        getsparam("PATH").unwrap_or_default().split(':').map(String::from).collect()
    };
    let lookup = |n: &str| -> Option<String> {
        cmdnamtab_lock()
            .read()
            .ok()
            .and_then(|t| t.get_full_path(n))
            .map(|p| p.display().to_string())
    };
    if OPT_ISSET(ops, b'r') {
        emptycmdnamtable();
        eo::bash_hash_forget(None);
        if argv.is_empty() {
            return 0;
        }
    }
    if OPT_ISSET(ops, b'p') {
        // `hash -p PATH NAME`
        let (Some(path), Some(n)) = (argv.first(), argv.get(1)) else {
            zwarnnam("hash", "-p: option requires an argument");
            return 1;
        };
        if let Ok(mut t) = cmdnamtab_lock().write() {
            t.add(cmdnam {
                node: hashnode { next: None, nam: n.clone(), flags: HASHED as i32 },
                name: None,
                cmd: Some(path.clone()),
            });
        }
        eo::bash_hash_forget(Some(n));
        return 0;
    }
    if OPT_ISSET(ops, b'd') {
        let mut rc = 0;
        for n in argv {
            let gone = cmdnamtab_lock().write().ok().map_or(false, |mut t| t.remove(n).is_some());
            if gone {
                eo::bash_hash_forget(Some(n));
            } else {
                zwarnnam("hash", &format!("{n}: not found"));
                rc = 1;
            }
        }
        return rc;
    }
    if OPT_ISSET(ops, b't') {
        let mut rc = 0;
        for n in argv {
            match lookup(n) {
                Some(p) => {
                    eo::bash_hash_hit(n);
                    if argv.len() > 1 {
                        println!("{n}\t{p}");
                    } else {
                        println!("{p}");
                    }
                }
                None => {
                    zwarnnam("hash", &format!("{n}: not found"));
                    rc = 1;
                }
            }
        }
        return rc;
    }
    if argv.is_empty() {
        let mut entries: Vec<(String, String)> = cmdnamtab_lock()
            .read()
            .map(|t| t.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|n| lookup(&n).map(|p| (n, p)))
            .collect();
        entries.sort();
        if entries.is_empty() {
            if !OPT_ISSET(ops, b'l') {
                println!("hash: hash table empty");
            }
            return 0;
        }
        if OPT_ISSET(ops, b'l') {
            for (n, p) in entries {
                println!("builtin hash -p {p} {n}");
            }
        } else {
            println!("hits\tcommand");
            for (n, p) in entries {
                if let Some(row) = eo::hash_entry(&n, &p) {
                    println!("{row}");
                }
            }
        }
        return 0;
    }
    // `hash NAME ...`: resolve through $PATH and remember.
    let mut rc = 0;
    for n in argv {
        if n.contains('/') {
            continue;
        }
        if hashcmd(n, &path_dirs()).is_none() {
            zwarnnam("hash", &format!("{n}: not found"));
            rc = 1;
        }
    }
    rc
}

/// !!! EMULATION-ONLY (no C counterpart) !!! bash's `enable`:
/// `enable [-a] [-nps] [name ...]` over bash's own builtin table. With
/// names, `-n` disables and a bare name enables; without, it lists.
pub fn bash_enable(argv: &[String], ops: &options, func: i32) -> i32 {
    let disabled = |n: &str| BUILTINS_DISABLED.lock().map_or(false, |s| s.contains(n));
    if argv.is_empty() {
        let (all, only_off, special) = (OPT_ISSET(ops, b'a'), OPT_ISSET(ops, b'n'), OPT_ISSET(ops, b's'));
        for (n, is_special) in crate::extensions::emulation_output::BASH_BUILTINS {
            if (special && !is_special) || (!all && only_off != disabled(n)) {
                continue;
            }
            println!("enable {}{n}", if disabled(n) { "-n " } else { "" });
        }
        return 0;
    }
    let mut plain = ops.clone();
    for f in [b'n', b'a', b'p', b's'] {
        plain.ind[f as usize] = 0;
    }
    let func = if OPT_ISSET(ops, b'n') { BIN_DISABLE } else { func };
    let rc = bin_enable("enable", argv, &plain, func);
    if rc != 0 {
        // bash words the failure for a name that is not a builtin.
        for n in argv {
            eprintln!("enable: {n}: not a shell builtin");
        }
    }
    rc
}

/// RETURN traps of the callers of the running function (bash).
static BASH_RETURN_SAVED: std::sync::Mutex<Vec<Option<String>>> = std::sync::Mutex::new(Vec::new());

/// !!! EMULATION-ONLY (no C counterpart) !!! bash function entry: a `RETURN`
/// trap set by a caller is not inherited (no `functrace`), so the callee
/// starts without one; the caller's is put back at exit unless the callee
/// installed its own.
pub fn bash_return_trap_enter() {
    if !crate::dash_mode::bash_mode() {
        return;
    }
    let outer = traps_table().lock().ok().and_then(|mut t| t.remove("RETURN"));
    if let Ok(mut st) = BASH_RETURN_SAVED.lock() {
        st.push(outer);
    }
}

/// !!! EMULATION-ONLY (no C counterpart) !!! bash function exit: run the
/// `RETURN` trap the function installed (`$?` is preserved), or restore the
/// caller's.
pub fn bash_return_trap_exit() {
    if !crate::dash_mode::bash_mode() {
        return;
    }
    let outer = BASH_RETURN_SAVED.lock().ok().and_then(|mut st| st.pop()).flatten();
    let own = traps_table().lock().ok().and_then(|t| t.get("RETURN").cloned());
    match own {
        Some(body) if !body.is_empty() => {
            // Hide the trap while it runs so a function it calls does not re-enter it.
            if let Ok(mut t) = traps_table().lock() {
                t.remove("RETURN");
            }
            let olastval = LASTVAL.load(Ordering::SeqCst);
            let _ = crate::ported::exec::execute_script(&body);
            LASTVAL.store(olastval, Ordering::SeqCst);
            if let Ok(mut t) = traps_table().lock() {
                t.insert("RETURN".to_string(), body);
            }
        }
        Some(_) => {}
        None => {
            if let (Some(body), Ok(mut t)) = (outer, traps_table().lock()) {
                t.insert("RETURN".to_string(), body);
            }
        }
    }
}

/// !!! EMULATION-ONLY (no C counterpart) !!! `trap BODY ... RETURN ...` and
/// `trap - RETURN`: RETURN is a bash pseudo-signal with no zsh slot, so it
/// is kept in `traps_table` only. Returns `None` when RETURN is not named.
pub fn bash_trap_return(argv: &[String]) -> Option<i32> {
    let args: Vec<&String> = argv.iter().skip_while(|a| *a == "--").collect();
    if args.len() < 2 || args[0] == "-p" || !args[1..].iter().any(|a| *a == "RETURN") {
        return None;
    }
    let body = args[0].clone();
    let others: Vec<String> = args[1..].iter().filter(|a| **a != "RETURN").map(|a| a.to_string()).collect();
    if let Ok(mut t) = traps_table().lock() {
        if body == "-" {
            t.remove("RETURN");
        } else {
            t.insert("RETURN".to_string(), body.clone());
        }
    }
    if others.is_empty() {
        return Some(0);
    }
    let mut rest = vec![body];
    rest.extend(others);
    Some(bin_trap("trap", &rest, &options { ind: [0u8; MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 }, 0))
}

/// One row of a shell's `set -o` listing.
///
/// `zsh` names the zshrs option that carries the state, `invert` flips it
/// (the Korn shells list the POSITIVE sense — `clobber on` where zsh has
/// `noclobber` off), and `default_on` is used for names no zsh option
/// backs, taken from the reference shell's own fresh-shell output.
struct SetOpt {
    name: &'static str,
    zsh: Option<&'static str>,
    invert: bool,
    default_on: bool,
}

const fn o(name: &'static str, zsh: &'static str) -> SetOpt {
    SetOpt {
        name,
        zsh: Some(zsh),
        invert: false,
        default_on: false,
    }
}
const fn inv(name: &'static str, zsh: &'static str) -> SetOpt {
    SetOpt {
        name,
        zsh: Some(zsh),
        invert: true,
        default_on: false,
    }
}
const fn fixed(name: &'static str, default_on: bool) -> SetOpt {
    SetOpt {
        name,
        zsh: None,
        invert: false,
        default_on,
    }
}

/// dash's `set -o`, in dash's own (non-alphabetical) order.
const DASH_SET_O: &[SetOpt] = &[
    o("errexit", "errexit"),
    o("noglob", "noglob"),
    o("ignoreeof", "ignoreeof"),
    o("interactive", "interactive"),
    o("monitor", "monitor"),
    o("noexec", "noexec"),
    o("stdin", "stdin"),
    o("xtrace", "xtrace"),
    o("verbose", "verbose"),
    o("vi", "vi"),
    o("emacs", "emacs"),
    o("noclobber", "noclobber"),
    o("allexport", "allexport"),
    o("notify", "notify"),
    o("nounset", "nounset"),
    // Debian's dash (the `/bin/sh` of Debian and Ubuntu) carries a
    // privileged-mode patch that lists `privileged` here; upstream dash
    // and the dash macOS ships as `/bin/dash` have no such option.
    #[cfg(target_os = "linux")]
    o("privileged", "privileged"),
    o("nolog", "nolog"),
    fixed("debug", false),
];

/// ksh93u+m's `set -o`, alphabetical. Note the POSITIVE spellings —
/// `clobber`, `glob`, `exec`, `log`, `unset` are the inverses of zsh's
/// `noclobber`, `noglob`, `noexec`, `nolog`, `nounset`.
const KSH93_SET_O: &[SetOpt] = &[
    o("allexport", "allexport"),
    fixed("backslashctrl", true),
    o("bgnice", "bgnice"),
    o("braceexpand", "braceexpand"),
    inv("clobber", "noclobber"),
    o("emacs", "emacs"),
    o("errexit", "errexit"),
    inv("exec", "noexec"),
    fixed("functrace", false),
    inv("glob", "noglob"),
    fixed("globcasedetect", false),
    fixed("globstar", false),
    fixed("gmacs", false),
    fixed("histexpand", false),
    fixed("histreedit", false),
    fixed("histverify", false),
    o("ignoreeof", "ignoreeof"),
    o("interactive", "interactive"),
    fixed("keyword", false),
    fixed("letoctal", false),
    inv("log", "nolog"),
    o("login_shell", "loginshell"),
    o("markdirs", "markdirs"),
    o("monitor", "monitor"),
    fixed("multiline", true),
    o("notify", "notify"),
    o("pipefail", "pipefail"),
    fixed("posix", false),
    o("privileged", "privileged"),
    fixed("rc", false),
    o("restricted", "restricted"),
    fixed("showme", false),
    o("trackall", "trackall"),
    inv("unset", "nounset"),
    o("verbose", "verbose"),
    o("vi", "vi"),
    fixed("viraw", true),
    o("xtrace", "xtrace"),
];

/// mksh's `set -o`, alphabetical, printed in four COLUMN-MAJOR columns.
const MKSH_SET_O: &[SetOpt] = &[
    o("allexport", "allexport"),
    o("bgnice", "bgnice"),
    o("braceexpand", "braceexpand"),
    fixed("emacs", true),
    o("errexit", "errexit"),
    fixed("gmacs", false),
    o("ignoreeof", "ignoreeof"),
    fixed("inherit-xtrace", true),
    o("interactive", "interactive"),
    fixed("keyword", false),
    o("login", "login"),
    o("markdirs", "markdirs"),
    o("monitor", "monitor"),
    o("noclobber", "noclobber"),
    o("noexec", "noexec"),
    o("noglob", "noglob"),
    fixed("nohup", true),
    o("nolog", "nolog"),
    o("notify", "notify"),
    o("nounset", "nounset"),
    o("physical", "physical"),
    o("pipefail", "pipefail"),
    fixed("posix", false),
    o("privileged", "privileged"),
    o("restricted", "restricted"),
    fixed("sh", false),
    o("stdin", "stdin"),
    fixed("trackall", true),
    fixed("utf8-mode", false),
    o("verbose", "verbose"),
    o("vi", "vi"),
    fixed("vi-esccomplete", false),
    fixed("vi-tabcomplete", true),
    fixed("viraw", false),
    o("xtrace", "xtrace"),
];

impl SetOpt {
    fn state(&self) -> bool {
        match self.zsh {
            Some(name) => {
                let on = crate::ported::options::opt_state_get(name).unwrap_or(self.default_on);
                if self.invert {
                    !on
                } else {
                    on
                }
            }
            None => self.default_on,
        }
    }
}

/// True when an ERROR in a POSIX special builtin must abort a
/// non-interactive shell, as POSIX requires ("If there is an error in a
/// special built-in utility ... the shell shall abort").
///
/// Measured with `{ CMD; } >/dev/null 2>&1; echo AFTER`: ksh93u+m, mksh
/// and dash all abort for `readonly -X`, `set -X` and `shift 99`; bash
/// and zsh never do (bash only under `set -o posix`). The boundary is an
/// ERROR, not a non-zero status — `eval false`, `trap "" BOGUSSIG` and
/// `unset novar` all continue in every one of them.
pub fn special_builtin_error_aborts() -> bool {
    matches!(
        personality(),
        Personality::Ksh93
            | Personality::Mksh
            | Personality::Pdksh
            | Personality::Dash
            | Personality::Sh
    )
}

/// The status the shell exits with when [`special_builtin_error_aborts`]
/// fires. `usage_error` distinguishes a bad option or operand from an
/// otherwise-failing special builtin, because ksh93 uses 2 for the first
/// and the builtin's own status for the second (`readonly -X` exits 2,
/// `shift 99` exits 1). dash uses 2 for both; mksh uses 1.
pub fn special_builtin_abort_status(usage_error: bool, builtin_status: i32) -> i32 {
    match personality() {
        Personality::Dash | Personality::Sh => 2,
        Personality::Mksh | Personality::Pdksh => 1,
        Personality::Ksh93 if usage_error => 2,
        _ => builtin_status,
    }
}

/// True for the shells that ALWAYS single-quote a value in their
/// `alias`, `export -p` and `readonly -p` listings.
///
/// dash does (`alias yy=1` lists as `yy='1'`, `export FOO=bar` as
/// `export FOO='bar'`); ksh93, mksh and zsh quote only when the value
/// needs it, and bash has its own `alias NAME='v'` / `declare -x`
/// spellings handled separately.
pub fn alias_always_quotes() -> bool {
    matches!(personality(), Personality::Dash | Personality::Sh)
}

/// `set -o` rendered the way this personality's shell renders it, or
/// `None` for zsh and bash (whose listings are handled by their own
/// existing paths).
///
/// All three Bourne/Korn shells head the listing with
/// "Current option settings"; they differ in column width and, for mksh,
/// in laying the names out in four COLUMN-MAJOR columns.
pub fn set_o_listing() -> Option<String> {
    set_o_render(false)
}

/// `set +o` — the REUSABLE form, one `set -o NAME` / `set +o NAME` per
/// line. Implemented for the Bourne legs, whose shape is a
/// straightforward inversion of the listing above.
///
/// ksh93u+m and mksh instead emit a single compact line of long options
/// naming only the NON-DEFAULT settings (`set --default --braceexpand
/// --multiline --trackall --viraw`, `set -o .reset -o braceexpand`); that
/// form is not modelled, so those legs keep zsh's output.
pub fn set_plus_o_listing() -> Option<String> {
    match personality() {
        Personality::Dash | Personality::Sh => set_o_render(true),
        // ksh93u+m and mksh emit ONE line: a reset word followed by the
        // options that are on and are NOT part of the shell's built-in
        // default set — `set --default --braceexpand --multiline
        // --trackall --viraw`, `set -o .reset -o braceexpand`. Adding
        // `set -e` adds `--errexit` / `-o errexit` to each, which is how
        // the default sets below were derived.
        Personality::Ksh93 => Some(compact_set_plus_o(
            KSH93_SET_O,
            KSH93_DEFAULT_ON,
            "set --default",
            "--",
        )),
        Personality::Mksh | Personality::Pdksh => Some(compact_set_plus_o(
            MKSH_SET_O,
            MKSH_DEFAULT_ON,
            "set -o .reset",
            "-o ",
        )),
        _ => None,
    }
}

/// Options ksh93u+m already has on after `set --default`, so `set +o`
/// never re-lists them. Derived by diffing a fresh shell's `set -o`
/// (on: backslashctrl braceexpand clobber exec glob log multiline
/// trackall unset viraw) against its `set +o` (which names only
/// braceexpand, multiline, trackall, viraw).
const KSH93_DEFAULT_ON: &[&str] = &["backslashctrl", "clobber", "exec", "glob", "log", "unset"];

/// The same for mksh, derived the same way: a fresh shell lists only
/// `braceexpand` after `-o .reset`, so everything else that is on is part
/// of the reset state.
const MKSH_DEFAULT_ON: &[&str] = &[
    "emacs",
    "inherit-xtrace",
    "nohup",
    "trackall",
    "vi-tabcomplete",
];

/// Render the Korn shells' one-line `set +o`: a reset word, then each
/// non-default option that is currently on, in table (alphabetical) order.
fn compact_set_plus_o(table: &[SetOpt], defaults: &[&str], reset: &str, prefix: &str) -> String {
    let mut out = String::from(reset);
    for e in table {
        if e.state() && !defaults.contains(&e.name) {
            out.push(' ');
            out.push_str(prefix);
            out.push_str(e.name);
        }
    }
    out.push('\n');
    out
}

fn set_o_render(reusable: bool) -> Option<String> {
    let (table, width) = match personality() {
        Personality::Dash | Personality::Sh => (DASH_SET_O, 15usize),
        Personality::Ksh93 => (KSH93_SET_O, 24),
        Personality::Mksh | Personality::Pdksh => (MKSH_SET_O, 14),
        _ => return None,
    };
    let on = |b: bool| if b { "on" } else { "off" };
    if reusable {
        // `set -o NAME` when the option is on, `set +o NAME` when off —
        // the form the shell can read back in.
        let mut out = String::new();
        for e in table {
            out.push_str(&format!(
                "set {}o {}\n",
                if e.state() { "-" } else { "+" },
                e.name
            ));
        }
        return Some(out);
    }
    let mut out = String::from("Current option settings\n");
    if matches!(personality(), Personality::Mksh | Personality::Pdksh) {
        // Four columns, filled top-to-bottom then left-to-right, with the
        // row count taken from the first column.
        let rows = table.len().div_ceil(4);
        for r in 0..rows {
            let mut line = String::new();
            for c in 0..4 {
                let Some(e) = table.get(c * rows + r) else {
                    continue;
                };
                // mksh pads the state to five columns, not four: `off` is
                // followed by two spaces before the next name.
                line.push_str(&format!("{:<width$} {:<5}", e.name, on(e.state())));
            }
            out.push_str(line.trim_end());
            out.push('\n');
        }
        return Some(out);
    }
    for e in table {
        out.push_str(&format!("{:<width$} {}\n", e.name, on(e.state())));
    }
    Some(out)
}

// Unit tests for the per-shell formats live in
// tests/builtin_output_parity.rs, not here: asserting them means moving
// the process-global personality, and the library test binary runs its
// tests in parallel threads that share it — doing it inline made unrelated
// compsys / zle / hist tests fail depending on scheduling.
