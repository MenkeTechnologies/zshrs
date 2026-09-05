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
        // dash: six decimals (microseconds).
        Personality::Dash => {
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
/// | ksh93u+m | one bare name per line |
/// | dash | signal `0` first, then one bare name per line |
/// | mksh | two columns of `%2d %6s %s` with the strsignal text |
pub fn kill_list(sigs: &[(i32, String)]) -> Option<String> {
    let mut out = String::new();
    match personality() {
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
        // dash lists signal 0 as well, before the real signals.
        Personality::Dash => {
            out.push_str("0\n");
            for (_, name) in sigs {
                out.push_str(name);
                out.push('\n');
            }
        }
        // ksh93u+m: one bare name per line, no number and no `SIG`. It
        // also keeps the historical `IOT` spelling for signal 6, where
        // every other shell here says `ABRT` — the only name that differs
        // across the whole list.
        Personality::Ksh93 | Personality::Sh | Personality::Csh => {
            let ksh = personality() == Personality::Ksh93;
            for (num, name) in sigs {
                if ksh && *num == 6 {
                    out.push_str("IOT");
                } else {
                    out.push_str(name);
                }
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

/// One `hash` listing row, or `None` to use zsh's `name=path` form.
///
/// bash's first column is a per-command hit counter. zshrs keeps no such
/// counter, so it reports 0 — correct for a freshly hashed entry (which
/// is what `hash NAME; hash` shows) and low by however many times the
/// command has since run. The alternative, omitting the column, would
/// break the format for every script that reads it.
pub fn hash_entry(name: &str, path: &str) -> Option<String> {
    match personality() {
        Personality::Bash => Some(format!("{:>4}\t{path}", 0)),
        // dash prints the resolved path alone, no name and no `=`.
        Personality::Dash => Some(path.to_string()),
        _ => {
            let _ = name;
            None
        }
    }
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
        _ => None,
    }
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
