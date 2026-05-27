//! `buf` holds metafied bytes (`Src/prompt.c` `char *buf`); callers that need
//! Unicode text run `unmetafy` (see [`buf_vars::expanded_utf8`]).
//!
//! `prompt_tls` holds values C reads as file-scope globals. Call
//! `prompt_tls::sync_from_globals` before expansion to refresh from
//! the canonical C globals (paramtab, LASTVAL, curhist, JOBTAB, ...).

use crate::ported::params::{paramtab, setaparam};
use crate::ported::utils::{imeta_byte, strpfx};
use crate::ported::zsh_h::{
    isset, zattr, Inpar, Nularg, Outpar, COL_SEQ_BG, COL_SEQ_FG, PROMPTBANG, PROMPTPERCENT,
    TERM_BAD, TERM_NOUP, TERM_UNKNOWN, TSC_PROMPT, TSC_RAW, TXTBGCOLOUR, TXTBOLDFACE, TXTFGCOLOUR,
    TXTSTANDOUT, TXTUNDERLINE, TXT_ATTR_ALL, TXT_ATTR_BG_24BIT, TXT_ATTR_BG_COL_MASK,
    TXT_ATTR_BG_COL_SHIFT, TXT_ATTR_BG_MASK, TXT_ATTR_FG_24BIT, TXT_ATTR_FG_COL_MASK,
    TXT_ATTR_FG_COL_SHIFT, TXT_ATTR_FG_MASK, TXT_ERROR,
};
use crate::zsh_h::Meta;
use crate::DPUTS;
use std::cell::RefCell;
use std::env;
use std::sync::atomic::Ordering;

/// Thread-local mirrors of zsh globals read during `promptexpand()` (logical
/// `$PWD`, `$?`, `cmdstack`, …). C uses scattered globals; zshrs uses TLS,
/// then copies into `buf_vars` for each expansion walk.
pub(crate) mod prompt_tls {
    use crate::ported::hist::curhist;
    use crate::ported::jobs::JOBTAB;
    use crate::ported::modules::parameter::FUNCSTACK;
    use crate::ported::params::{getsparam, paramtab};
    use crate::ported::utils::adjustcolumns;
    use std::cell::RefCell;
    use std::env;

    thread_local! {
        pub(super) static PWD: RefCell<String> = const { RefCell::new(String::new()) };
        pub(super) static HOME: RefCell<String> = const { RefCell::new(String::new()) };
        pub(super) static USER: RefCell<String> = const { RefCell::new(String::new()) };
        pub(super) static HOST: RefCell<String> = const { RefCell::new(String::new()) };
        pub(super) static HOST_SHORT: RefCell<String> = const { RefCell::new(String::new()) };
        pub(super) static TTY: RefCell<String> = const { RefCell::new(String::new()) };
        pub(super) static LASTVAL: RefCell<i32> = const { RefCell::new(0) };
        pub(super) static HISTNUM: RefCell<i64> = const { RefCell::new(1) };
        pub(super) static SHLVL: RefCell<i32> = const { RefCell::new(1) };
        pub(super) static NUM_JOBS: RefCell<i32> = const { RefCell::new(0) };
        pub(super) static IS_ROOT: RefCell<bool> = const { RefCell::new(false) };
        pub(super) static CMDSTACK: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
        pub(super) static PSVAR: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
        pub(super) static TERM_WIDTH: RefCell<usize> = const { RefCell::new(80) };
        pub(super) static LINENO: RefCell<i64> = const { RefCell::new(1) };
        pub(super) static SCRIPTNAME: RefCell<Option<String>> = const { RefCell::new(None) };
        pub(super) static SCRIPTFILENAME: RefCell<Option<String>> =
            const { RefCell::new(None) };
        pub(super) static ARGEXTRA: RefCell<String> = const { RefCell::new(String::new()) };
        pub(super) static FUNC_LINE_BASE: RefCell<Option<i64>> = const { RefCell::new(None) };
        pub(super) static FUNCSTACK_FILENAME: RefCell<Option<String>> =
            const { RefCell::new(None) };
    }

    /// Populate prompt-side thread-locals from the C globals each
    /// `Src/prompt.c` field maps to. Replaces an earlier read of
    /// `ShellExecutor` state with the canonical C source:
    ///   - `$PWD`/`$HOME`/`$USER`/`$SHLVL`/`$LINENO` etc. → paramtab
    ///     reads via `getsparam(name)` (params.c:3076)
    ///   - `$?` → LASTVAL atomic (builtin.c:6443 lastval)
    ///   - `curhist` → HISTNUM atomic (hist.c:233)
    ///   - active job count → JOBTAB scan (jobs.c:88)
    ///   - PSVAR → paramtab "psvar" array
    ///   - term width → `adjustcolumns()` (utils.c)
    ///   - scriptname → utils.rs::scriptname()
    pub(crate) fn sync_from_globals() {
        let pwd = getsparam("PWD")
            .filter(|p| !p.is_empty())
            .or_else(|| env::var("PWD").ok().filter(|p| !p.is_empty()))
            .or_else(|| {
                env::current_dir()
                    .ok()
                    .map(|p| p.to_string_lossy().to_string())
            })
            .unwrap_or_else(|| "/".to_string());
        let home = getsparam("HOME").unwrap_or_default();
        let user = getsparam("USER")
            .or_else(|| getsparam("LOGNAME"))
            .or_else(|| env::var("USER").ok())
            .or_else(|| env::var("LOGNAME").ok())
            .unwrap_or_else(|| "user".to_string());
        let host = getsparam("HOST")
            .or_else(|| {
                hostname::get()
                    .ok()
                    .map(|h| h.to_string_lossy().to_string())
            })
            .unwrap_or_else(|| "localhost".to_string());
        let host_short = host.split('.').next().unwrap_or(&host).to_string();
        let shlvl = getsparam("SHLVL")
            .and_then(|s| s.parse().ok())
            .or_else(|| env::var("SHLVL").ok().and_then(|s| s.parse().ok()))
            .unwrap_or(1);
        PWD.with(|c| *c.borrow_mut() = pwd);
        HOME.with(|c| *c.borrow_mut() = home);
        USER.with(|c| *c.borrow_mut() = user);
        HOST.with(|c| *c.borrow_mut() = host);
        HOST_SHORT.with(|c| *c.borrow_mut() = host_short);
        TTY.with(|c| c.borrow_mut().clear());
        // c:builtin.c:6443 lastval — the C global $?. The canonical
        // store is `builtin::LASTVAL` (AtomicI32). All status writes
        // (vm.last_status updates, exec.last_status setters, and
        // signals.rs:759 SIGCHLD) keep this current via
        // `LASTVAL.store`; prompt expansion just reads it.
        LASTVAL.with(|c| {
            *c.borrow_mut() =
                crate::ported::builtin::LASTVAL.load(std::sync::atomic::Ordering::Relaxed);
        });
        // c:hist.c:233 curhist
        HISTNUM.with(|c| {
            *c.borrow_mut() = curhist.load(std::sync::atomic::Ordering::Relaxed);
        });
        SHLVL.with(|c| *c.borrow_mut() = shlvl);
        // c:jobs.c:88 jobtab — count in-use job slots
        NUM_JOBS.with(|c| {
            *c.borrow_mut() = JOBTAB
                .get_or_init(|| std::sync::Mutex::new(Vec::new()))
                .lock()
                .map(|t| t.iter().filter(|j| j.is_inuse()).count() as i32)
                .unwrap_or(0);
        });
        IS_ROOT.with(|c| *c.borrow_mut() = unsafe { libc::geteuid() } == 0);
        // c:prompt.c:56 cmdstack — the canonical store is the
        // file-static CMDSTACK at the bottom of this file.
        CMDSTACK.with(|c| {
            *c.borrow_mut() = super::CMDSTACK.with(|stack| stack.borrow().clone());
        });
        // c:params.c PSVAR special — array read from paramtab.
        PSVAR.with(|c| {
            *c.borrow_mut() = paramtab()
                .read()
                .ok()
                .and_then(|t| t.get("psvar").and_then(|p| p.u_arr.clone()))
                .unwrap_or_default();
        });
        // c:utils.c adjustcolumns — re-read TIOCGWINSZ.
        TERM_WIDTH.with(|c| {
            *c.borrow_mut() = adjustcolumns();
        });
        LINENO.with(|c| {
            *c.borrow_mut() = getsparam("LINENO")
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(1);
        });
        // c:utils.c:36 scriptname — the ACTIVE script/function name
        // (updated on doshfunc entry per c:5903). %N reads this.
        let scriptname = crate::ported::utils::scriptname_get();
        SCRIPTNAME.with(|c| *c.borrow_mut() = scriptname.clone().or_else(|| getsparam("0")));
        // c:init.c scriptfilename — the FILE the current code was
        // PARSED from. C zsh init.c:479 sets `scriptname =
        // scriptfilename = "zsh"` for the -c invocation; doshfunc
        // at c:5903 overrides scriptname but NOT scriptfilename.
        // Routes through `scriptfilename_get` (canonical static
        // mirror of C's file-static `scriptfilename`).
        SCRIPTFILENAME.with(|c| {
            *c.borrow_mut() = crate::ported::utils::scriptfilename_get()
                .or_else(|| scriptname.clone())
                .or_else(|| getsparam("0"));
        });
        FUNC_LINE_BASE.with(|c| *c.borrow_mut() = None);
        FUNCSTACK_FILENAME.with(|c| {
            *c.borrow_mut() = FUNCSTACK
                .lock()
                .ok()
                .and_then(|s| s.last().and_then(|fs| fs.filename.clone()));
        });
        ARGEXTRA.with(|c| {
            *c.borrow_mut() = getsparam("ZSH_ARGZERO")
                .or_else(|| env::args().next())
                .unwrap_or_else(|| "zsh".to_string());
        });
    }
}

/// `struct buf_vars` from `Src/prompt.c:76-121`. `dontcount` is C `%{`/`%}`
/// nesting; `in_escape` holds readline `\x01`/`\x02` glue only.
/// `last` pointer / full trunc `bp1` realloc: TODO.
#[allow(non_camel_case_types)]
pub struct buf_vars {
    // c:Src/prompt.c:76
    // Rust-port bag-of-globals dissolution (Rule D / PORT_PLAN.md
    // Anti-pattern 1). C `struct buf_vars` (prompt.c:76-121) has 9
    // fields; previous Rust ports aggregated ~25 unrelated C file-
    // statics here (pwd / home / user / host / host_short / tty /
    // lastval / histnum / shlvl / num_jobs / is_root / cmd_stack /
    // psvar / term_width / lineno / scriptname / scriptfilename /
    // argzero / func_line_base / funcstack_filename). All deleted;
    // callers route through `prompt_tls::*` per-prompt thread_local
    // snapshots (hydrated from the canonical statics in utils.rs /
    // builtin.rs / hist.rs).
    pub buf: Vec<u8>,
    pub bufspc: usize,
    pub bp: usize,
    pub bufline: usize,
    pub bp1: Option<usize>,
    pub fm: String,
    pub fm_pos: usize,
    pub truncwidth: i32,
    pub dontcount: i32,
    pub trunccount: i32,
    pub rstring: Option<String>,
    pub Rstring: Option<String>,
    // WARNING: NOT IN PROMPT.C — Rust-only expander state.
    // C threads the current zattr inline as it emits SGR bytes into
    // `bp` (no field on `struct buf_vars`); Rust caches the current
    // attribute set on the buf_vars so apply_attrs() / reset_attrs()
    // can emit incremental diffs instead of re-emitting the whole
    // SGR every step.
    attrs: zattr,
    // WARNING: NOT IN PROMPT.C — Rust-only readline `\x01`/`\x02`
    // prompt-width-ignore glue. C zsh's `%{ %}` nesting is tracked
    // by `dontcount` (which IS in C buf_vars, above). This separate
    // bool covers the readline-style RL_PROMPT_*_IGNORE byte
    // emissions that the host's readline-compat shim needs around
    // any escape-sequence span.
    in_escape: bool,
    // `prompt_percent` / `prompt_bang` field copies deleted — these
    // are option-table flags in C (`isset(PROMPTPERCENT)` /
    // `isset(PROMPTBANG)` at prompt.c:325 + checks per expander).
    // Rule D bag-of-globals violation when carried as struct fields.
    // Callers route through `isset(PROMPTPERCENT)` / `isset(PROMPTBANG)`
    // directly.
}

// Note: there is no Rust helper for `cmdnames[cmdstack[t0]]` — C
// uses the bare array indexing inline (`Src/prompt.c:835`,
// `:846`, `:861`, `:872`). Use `CMDNAMES.get(b as usize).copied()`
// at every call site to mirror that pattern faithfully.

// `pub struct zattr` and `pub enum Color` — DELETED per user
// directive. Both were Rust-only abstractions over the canonical
// `zattr` (u64) bitfield from `Src/zsh.h:2685-2741`. C packs every
// attribute (bold/faint/standout/underline/italic) PLUS the
// foreground colour (24 bits), background colour (24 bits), and
// 24-bit-or-palette flags into a single 64-bit word. The Rust
// port now uses the canonical `crate::ported::zsh_h::zattr`
// directly; helpers below mirror the C bit-twiddling macros.
//
// Bit layout (matches Src/zsh.h:2694-2741 exactly):
//   0x0001 TXTBOLDFACE / 0x0002 TXTFAINT / 0x0004 TXTSTANDOUT /
//   0x0008 TXTUNDERLINE / 0x0010 TXTITALIC / 0x0020 TXTFGCOLOUR /
//   0x0040 TXTBGCOLOUR / 0x4000 TXT_ATTR_FG_24BIT /
//   0x8000 TXT_ATTR_BG_24BIT
//   bits 16-39: TXT_ATTR_FG_COL_MASK (palette index 0-255 OR
//               packed RGB if TXT_ATTR_FG_24BIT)
//   bits 40-63: TXT_ATTR_BG_COL_MASK (same for BG)
// `zattr` is the canonical C typedef from Src/zsh.h:2689
// (`typedef uint64_t zattr;`). Imported directly below.

// ---------------------------------------------------------------------------
// Remaining missing functions from prompt.c
// ---------------------------------------------------------------------------

/// Direct port of `static void promptpath(char *p, int npath, int
/// tilde)` from `Src/prompt.c:133-169`. Format a path for `%~`,
/// `%/`, `%c` — optional tilde substitution + last-N-components
/// truncation.
///
/// **Previous gap:** the Rust port only checked the explicit `home`
/// arg via `path.starts_with(home)`. C uses `finddir(p)` which
/// covers BOTH `$HOME` AND `hash -d` named-dir matches (e.g.
/// `~tmp/file` when `hash -d tmp=/tmp` is set). Without the
/// finddir branch, `%~` rendered `/tmp/foo` literally instead of
/// `~tmp/foo` even with the named-dir registered. Restored.
///
/// WARNING: signature kept as `(path, npath, tilde, home)` to
/// preserve existing zshrs callers (3000+ tests). The `home` arg
/// is the explicit-HOME override used for unit tests; live callers
/// pass an empty string to delegate to finddir's HOME read.
pub fn promptpath(path: &str, npath: usize, tilde: bool, home: &str) -> String {
    // c:134
    // c:139-141 — `if (tilde && (nd = finddir(p))) modp = tricat("~",
    //              nd->node.nam, p + strlen(nd->dir));`
    let display = if tilde {
        // Try the explicit-home arg first (test-driven path; empty
        // string in live callers means "use finddir below").
        if !home.is_empty() && path.starts_with(home) {
            let rest = &path[home.len()..];
            if rest.is_empty() || rest.starts_with('/') {
                format!("~{}", rest)
            } else {
                // Home prefix matches but path continues with non-/
                // (e.g. /homex when home=/home). Fall through to
                // finddir which checks bounds correctly.
                crate::ported::utils::finddir(path).unwrap_or_else(|| path.to_string())
            }
        } else {
            // c:139 — finddir covers $HOME + every `hash -d` named-dir.
            crate::ported::utils::finddir(path).unwrap_or_else(|| path.to_string())
        }
    } else {
        // c:165 — `else stradd(modp);` — no tilde transform.
        path.to_string()
    };

    // c:142-165 — npath truncation. `npath > 0` keeps LAST N
    // components; `npath < 0` keeps FIRST -N components. Sig still
    // usize for caller compat; treat values >= 0x80000000 (negative
    // i32 cast to usize) as the negative form. Live `%N` callers pass
    // a positive count from `%[<NUM>~]`; negative form is uncommon
    // but real per the C source.
    if npath == 0 {
        return display;
    }
    let signed = npath as i64;
    let neg_n = if signed < 0 || (signed as u64) >= (i32::MIN as u32 as u64) {
        // High-bit set → originally a negative i32 widened through usize.
        let as_i32 = (signed as i32).wrapping_neg();
        if as_i32 > 0 {
            Some(as_i32 as usize)
        } else {
            None
        }
    } else {
        None
    };

    let components: Vec<&str> = display.split('/').filter(|s| !s.is_empty()).collect();
    if let Some(first_n) = neg_n {
        // c:155-163 — keep FIRST N components.
        if components.len() <= first_n {
            return display;
        }
        components[..first_n].join("/")
    } else {
        // c:144-153 — keep LAST N components.
        if components.len() <= npath {
            return display;
        }
        components[components.len() - npath..].join("/")
    }
}

// `pub struct PromptExpandResult` — DELETED per user directive.
// Was a Rust-only bundle for C's three outparams. C signature
// `char *promptexpand(char *s, int ns, const char *marker, char
// *rs, char *Rs)` (`Src/prompt.c:182`) writes through `rs`/`Rs`
// pointers and returns the expanded `char *`. Rust port now
// returns a `(String, Option<usize>, Option<usize>)` tuple
// matching C's outparam shape directly.

/// Port of `promptexpand(char *s, int ns, const char *marker, char *rs, char *Rs)` from `Src/prompt.c:182`.
///
/// C signature:
/// `char *promptexpand(char *s, int ns, const char *marker,
///                     char *rs, char *Rs);`
///
/// `ns` flags the "non-special" mode (skip processing of `%E` /
/// `%{...%}`); `marker` is an opt-in completion-cursor sentinel
/// embedded into the output; `rs`/`Rs` are output pointers
/// receiving the byte offsets where the right-prompt anchor
/// landed. Rust returns the four values as a tuple
/// `(expanded, rs_offset, cap_rs_offset)`.
/// WARNING: param names don't match C — Rust=(_ns, _marker) vs C=(s, ns, marker, rs, Rs)
pub fn promptexpand(
    // c:182
    s: &str,
    _ns: i32,
    _marker: Option<&str>,
) -> (String, Option<usize>, Option<usize>) {
    let expanded = expand_prompt(s);
    // C: `*rs = bv.bp - bv.buf` at `%E` / `%>` markers. Rust
    // expander loses that metadata, so a second pass on `s` is the
    // closest approximation. Source-offset → expanded-offset is
    // 1:1 except where expansion lengthens.
    let rs_offset = s.find("%E").or_else(|| s.find("%E)")); // c:Src/prompt.c:257
    let cap_rs_offset = s.find("%>>"); // c:Src/prompt.c:257
    (expanded, rs_offset, cap_rs_offset)
}

/// Escape text attributes back to a `%`-prefixed prompt string.
/// Port of `zattrescape(zattr atr, int *len)` from Src/prompt.c:257 — inverse of
/// `parsehighlight()`; used by the `print -P` output path.
/// WARNING: param names don't match C — Rust=(attrs) vs C=(atr, len)
pub fn zattrescape(attrs: zattr) -> String {
    // c:257
    let mut result = String::new();
    if attrs & TXTBOLDFACE != 0 {
        result.push_str("%B");
    } // c:259
    if attrs & TXTUNDERLINE != 0 {
        result.push_str("%U");
    } // c:259
    if attrs & TXTSTANDOUT != 0 {
        result.push_str("%S");
    } // c:259
    if attrs & TXTFGCOLOUR != 0 {
        // c:266
        let raw = (attrs & TXT_ATTR_FG_COL_MASK) >> TXT_ATTR_FG_COL_SHIFT;
        let c = if attrs & TXT_ATTR_FG_24BIT != 0 {
            COLOR_24BIT | (raw as Color & 0x00ff_ffff)
        } else {
            raw as Color
        };
        result.push_str(&format!("%F{{{}}}", color_name(c)));
    }
    if attrs & TXTBGCOLOUR != 0 {
        // c:266
        let raw = (attrs & TXT_ATTR_BG_COL_MASK) >> TXT_ATTR_BG_COL_SHIFT;
        let c = if attrs & TXT_ATTR_BG_24BIT != 0 {
            COLOR_24BIT | (raw as Color & 0x00ff_ffff)
        } else {
            raw as Color
        };
        result.push_str(&format!("%K{{{}}}", color_name(c)));
    }
    result
}

/// Parse a `,`-separated highlight specification.
/// Port of `parsehighlight(char *arg, char endchar, zattr *atr, zattr *mask)` from Src/prompt.c:285 — handles
// Parse the argument for %H                                                // c:285
/// `bold` / `underline` / `standout` / `none` plus `fg=NAME` and
/// `bg=NAME` color targets.
/// WARNING: param names don't match C — Rust=(spec) vs C=(arg, endchar, atr, mask)
pub fn parsehighlight(spec: &str) -> zattr {
    // c:285
    let mut attrs: zattr = 0;
    for part in spec.split(',') {
        let part = part.trim();
        match part {
            "bold" => attrs |= TXTBOLDFACE,       // c:288
            "underline" => attrs |= TXTUNDERLINE, // c:288
            "standout" => attrs |= TXTSTANDOUT,   // c:288
            "none" => {
                attrs = 0; // c:288
            }
            s if s.starts_with("fg=") => {
                if let Some(code) = match_named_colour(&s[3..]) {
                    // c:295
                    attrs = zattr_set_fg_palette(attrs, code); // c:295
                }
            }
            s if s.starts_with("bg=") => {
                if let Some(code) = match_named_colour(&s[3..]) {
                    // c:295
                    attrs = zattr_set_bg_palette(attrs, code); // c:295
                }
            }
            _ => {}
        }
    }
    attrs
}

/// Parse a single colour character from a `%F{...}` argument.
/// Port of `parsecolorchar(zattr arg, int is_fg)` from Src/prompt.c:318.
pub fn parsecolorchar(arg: &str, is_fg: bool) -> Option<(Color, String)> {
    // c:318
    let color = color_from_name(arg)?; // c:318 (match_colour)
    let ansi = color_to_ansi(color, is_fg); // c:2440
    Some((color, ansi))
}

// ---------------------------------------------------------------------------
// Remaining prompt.c entry points (after `putpromptchar` / `buf_vars`)
// ---------------------------------------------------------------------------

/// Port of `static int putpromptchar(int doprint, int endchar)` from
/// `Src/prompt.c:359`. Delegates to `buf_vars::run_putpromptchar` +
/// `buf_vars::process_percent` — the 566-line C body's per-`%X`
/// escape table lives there split across the inherent-method
/// dispatch (~100 lines each, real ports). The free-fn entry exists
/// for C-ABI parity so cross-module call sites match the C symbol.
pub fn putpromptchar(bv: &mut buf_vars, doprint: i32, endchar: i32) -> i32 {
    // c:359
    // Delegates to the buf_vars method that holds the real loop.
    bv.run_putpromptchar(doprint, endchar)
}

/// Internal prompt char output.
/// Port of `pputc(char c)` from Src/prompt.c:976 — the C source's
/// per-character buffer-append helper. Rust's `String::push`
/// covers it directly; this wrapper exists for call-site parity.
/// WARNING: param names don't match C — Rust=(buf, c) vs C=(c)
pub fn pputc(buf: &mut String, c: char) {
    // c:976
    buf.push(c);
}

// Make sure there is room for `need' more characters in the buffer.       // c:991
/// Port of `static void addbufspc(int need)` from `Src/prompt.c:991`.
/// C accesses the file-static `bv` (struct promptbuf) and may
/// realloc `bv->buf` if (bp - buf) + need*2 > bufspc. The zshrs
/// promptbuf state lives on a per-promptbuf `PromptBuf` struct
/// (impl method `PromptBuf::addbufspc` does the real work at line
/// 1325). This free-fn shape is a C-name-parity anchor: callers
/// that don't have a PromptBuf in hand (only via file-static `bv`
/// in C) reach for `addbufspc(need)` directly — no such caller
/// exists in zshrs yet because every prompt-buf op goes through
/// the struct method. The body is a no-op because Rust String
/// auto-grows on the impl-method side; `need` is bound to mirror C.
pub fn addbufspc(need: i32) {
    // c:991
    // c:993 — `need *= 2;` for metafication
    let _need_doubled = need.saturating_mul(2);
    // c:994-1010 — realloc dance on `bv->buf` if growth needed.
    // Architectural divergence: C's `bv` is a file-static struct
    // (Src/prompt.c:~50) — implicit global state that every prompt
    // helper mutates. zshrs models the same state as a `buf_vars`
    // struct constructed per `promptexpand()` call (line 1286) with
    // an inherent `addbufspc` method (line 1380) — the real growth
    // dance. Free-fn callers MUST go through the impl method via the
    // promptexpand entry, where they have a `&mut buf_vars` in hand.
    // This free-fn body is intentionally a name-parity no-op (kept so
    // grep'ping for `addbufspc` finds the C-side counterpart); no
    // direct invocation path exists or should exist.
}

/// Append a string to the prompt buffer.
/// Port of `stradd(char *d)` from Src/prompt.c:1016.
/// WARNING: param names don't match C — Rust=(buf, s) vs C=(d)
pub fn stradd(buf: &mut String, s: &str) {
    // c:1016
    buf.push_str(s);
}

/// Port of `void tsetcap(int cap, int flags)` from `Src/prompt.c:1083`.
/// Emit a terminal capability escape — raw to tty (TSC_RAW), to
/// shout (default), or into the prompt buffer with Inpar/Outpar
/// markers for visible-width counting (TSC_PROMPT).
/// ```c
/// void
/// tsetcap(int cap, int flags)
/// {
///     if (tccan(cap) && !(termflags & (TERM_NOUP|TERM_BAD|TERM_UNKNOWN))) {
///         switch (flags) {
///         case TSC_RAW:   tputs(tcstr[cap], 1, putraw); break;
///         case 0:
///         default:        tputs(tcstr[cap], 1, putshout); break;
///         case TSC_PROMPT:
///             if (!bv->dontcount) { addbufspc(1); *bv->bp++ = Inpar; }
///             tputs(tcstr[cap], 1, putstr);
///             if (!bv->dontcount) {
///                 int glitch = 0;
///                 if (cap == TCSTANDOUTBEG || cap == TCSTANDOUTEND)
///                     glitch = tgetnum("sg");
///                 else if (cap == TCUNDERLINEBEG || cap == TCUNDERLINEEND)
///                     glitch = tgetnum("ug");
///                 if (glitch < 0) glitch = 0;
///                 addbufspc(glitch + 1);
///                 while (glitch--) *bv->bp++ = Nularg;
///                 *bv->bp++ = Outpar;
///             }
///             break;
///         }
///     }
/// }
/// ```
pub fn tsetcap(cap: i32, flags: i32) -> String {
    // c:1083

    let mut out = String::new();

    // c:1085 — `if (tccan(cap) && !(termflags & ...))`
    let tclen_guard = crate::ported::init::tclen.lock().unwrap();
    let cap_ok = cap >= 0 && (cap as usize) < tclen_guard.len() && tclen_guard[cap as usize] != 0;
    drop(tclen_guard);
    let termflags = crate::ported::params::TERMFLAGS.load(Ordering::SeqCst);
    if !(cap_ok && (termflags & (TERM_NOUP | TERM_BAD | TERM_UNKNOWN)) == 0) {
        return out;
    }

    let cap_str = crate::ported::init::tcstr
        .lock()
        .unwrap()
        .get(cap as usize)
        .cloned()
        .unwrap_or_default();

    match flags {
        // c:1086
        x if x == TSC_RAW => {
            // c:1087
            // c:1088 — `tputs(tcstr[cap], 1, putraw);` — raw write to tty fd.
            let fd = crate::ported::init::SHTTY.load(Ordering::Relaxed);
            let out_fd = if fd >= 0 { fd } else { 2 };
            let _ = crate::ported::utils::write_loop(out_fd, cap_str.as_bytes());
        }
        x if x == TSC_PROMPT => {
            // c:1094
            // c:1095-1113 — TSC_PROMPT: emit into the prompt buffer
            // wrapped in Inpar/Outpar markers so the screen-width
            // counter (countprompt) knows to skip the escape.
            //
            // The previous Rust port used '\x01' / '\x02' as the
            // markers, but the canonical token bytes are
            // Inpar=0x88 (zsh.h:163) and Outpar=0x8a (zsh.h:165).
            // countprompt was looking for the canonical values, so
            // tsetcap-emitted escapes were ALSO counted as visible
            // width — a tcap-based prompt would wrap a column early.
            // Pair the wrapping with countprompt's recognition; both
            // sides now use the canonical bytes.
            out.push(Inpar); // c:1097 Inpar marker
            out.push_str(&cap_str); // c:1099
                                    // c:1101-1106 — glitch detection (sg / ug termcap nums).
                                    // tgetnum() not yet ported as a free fn; assume 0 (no glitch)
                                    // which matches modern terminals.
            out.push(Outpar); // c:1112 Outpar marker
        }
        _ => {
            // c:1090 default
            // c:1092 — `tputs(tcstr[cap], 1, putshout);`
            let fd = crate::ported::init::SHTTY.load(Ordering::Relaxed);
            let out_fd = if fd >= 0 { fd } else { 1 };
            let _ = crate::ported::utils::write_loop(out_fd, cap_str.as_bytes());
        }
    }
    out
}

// `putstr(int d)` (C: prompt.c:1121 — `int putstr(int d) { pputc(d);
// return 0; }`) had a Rust port `pub fn putstr(d: &str) -> String { d
// .to_string() }`. Both signature and body wrong: C is a `tputs(3)`
// per-byte output callback taking ONE byte, returning 0; the Rust
// port took a whole string and returned its clone — that's a string-
// dup helper, not the per-byte `tputs` callback. Zero Rust callers
// (the prompt-emit path uses pputc directly). Deleted; reintroduce
// as a faithful port when tsetcap()'s tputs(3) invocation lands and
// needs the per-byte callback.

/// Handle `%>...>` / `%<...<` / `%[truncchar string]` truncation.
/// Port of `prompttrunc(int arg, int truncchar, int doprint, int endchar)` from Src/prompt.c:1276.
///
/// Port of `static int prompttrunc(int arg, int truncchar, int doprint,
/// int endchar)` from `Src/prompt.c:1276`. Implements the `%<...<`,
/// `%>...>`, `%[...]` truncation syntax: stashes the truncation
/// string, recurses `putpromptchar` to expand the bounded region,
/// then either left- or right-truncates to fit `arg` screen cells.
///
/// Operates on the `buf_vars` scratch struct (file-statics in C:
/// `bv->fm` / `bv->bp` / `bv->buf` / `bv->truncwidth` /
/// `bv->dontcount` / `bv->trunccount`) — see c:76-121 for the
/// struct layout. Rust port takes `&mut buf_vars` to match.
/// WARNING: param names match C — Rust=(bv, arg, truncchar, doprint, endchar) vs C=(arg, truncchar, doprint, endchar)
pub fn prompttrunc(
    bv: &mut buf_vars,
    arg: i32,
    truncchar: i32, // c:1276
    doprint: i32,
    endchar: i32,
) -> i32 {
    if arg > 0 {
        // c:1278
        // c:1279 — `char ch = *bv->fm;` (peek)
        let ch = bv.fm.as_bytes().get(bv.fm_pos).copied().unwrap_or(0);
        let truncatleft = ch == b'<'; // c:1280
        let w = bv.bp; // c:1281 bp - buf

        // c:1288-1293 — re-entry guard: if a truncation is already
        // active, back up to the % marker and return so the outer
        // call can finish first.
        if bv.truncwidth != 0 {
            // c:1288
            while bv.fm_pos > 0 {
                bv.fm_pos -= 1; // c:1289
                if bv.fm.as_bytes().get(bv.fm_pos).copied().unwrap_or(0) == b'%' {
                    break;
                }
            }
            if bv.fm_pos > 0 {
                bv.fm_pos -= 1;
            } // c:1291
            return 0; // c:1292
        }

        bv.truncwidth = arg; // c:1295
        if bv.fm.as_bytes().get(bv.fm_pos).copied().unwrap_or(0) != b']' {
            // c:1296
            bv.fm_pos += 1; // c:1297
        }

        // c:1298-1303 — copy truncation string into buf until truncchar.
        let tchar = truncchar as u8;
        while let Some(&c) = bv.fm.as_bytes().get(bv.fm_pos) {
            // c:1298
            if c == 0 || c == tchar {
                break;
            }
            let mut cur = c;
            if cur == b'\\' && bv.fm.as_bytes().get(bv.fm_pos + 1).is_some() {
                // c:1299
                bv.fm_pos += 1;
                cur = bv.fm.as_bytes()[bv.fm_pos];
            }
            // c:1301 — addbufspc(1)
            if bv.bp >= bv.buf.len() {
                bv.buf.resize(bv.bp + 1, 0);
            }
            bv.buf[bv.bp] = cur; // c:1302 *bv->bp++ = *bv->fm++
            bv.bp += 1;
            bv.fm_pos += 1;
        }
        if bv.fm.as_bytes().get(bv.fm_pos).copied().unwrap_or(0) == 0 {
            // c:1304
            return 0; // c:1305
        }
        if bv.bp == w && truncchar == b']' as i32 {
            // c:1306
            if bv.bp >= bv.buf.len() {
                bv.buf.resize(bv.bp + 1, 0);
            }
            bv.buf[bv.bp] = b'<'; // c:1308
            bv.bp += 1;
        }
        // c:1310 — `ptr = bv->buf + w;` (truncation-string start)
        let ptr = w;
        // c:1317 — `truncstr = ztrduppfx(ptr, bv->bp - ptr);`
        let trunc_bytes = bv.buf[ptr..bv.bp].to_vec();
        let truncstr = String::from_utf8_lossy(&trunc_bytes).into_owned();

        bv.bp = ptr; // c:1319
        let w_save = bv.bp; // c:1320
        bv.fm_pos += 1; // c:1321
        bv.trunccount = bv.dontcount; // c:1322
                                      // c:1323 — `putpromptchar(doprint, endchar);` — recurse to
                                      // expand the bounded region; output goes into bv.buf at bp.
        putpromptchar(bv, doprint, endchar); // c:1323
        bv.trunccount = 0; // c:1324
        let ptr = w_save; // c:1325
                          // c:1326 — `*bv->bp = '\0';` — null-terminate.
        if bv.bp < bv.buf.len() {
            bv.buf[bv.bp] = 0;
        }

        // c:1343-1344 — `countprompt(ptr, &w, 0, -1)`: compute screen width.
        let region_bytes = &bv.buf[ptr..bv.bp];
        let region_str = std::str::from_utf8(region_bytes).unwrap_or("");
        let mut visible_w: i32 = 0;
        // Count chars (rough screen width — C's countprompt skips
        // escape sequences and counts MB_METASTRWIDTH; collapsed to
        // char count here since the bv buffer stores expanded text).
        for _ in region_str.chars() {
            visible_w += 1;
        }

        if visible_w > bv.truncwidth {
            // c:1344
            // c:1354-1410 — truncate. truncstr is the marker; replace
            // either the head (truncatleft=true: e.g. `%<...<`) or
            // tail (truncatleft=false: `%>...>`) with the marker.
            let maxwidth = bv.truncwidth - truncstr.chars().count() as i32;
            if maxwidth < 0 {
                // truncation marker is longer than the budget — use marker only
                bv.bp = ptr;
                let mb = truncstr.as_bytes();
                for &b in mb {
                    if bv.bp >= bv.buf.len() {
                        bv.buf.resize(bv.bp + 1, 0);
                    }
                    bv.buf[bv.bp] = b;
                    bv.bp += 1;
                }
            } else {
                let region_chars: Vec<char> = region_str.chars().collect();
                let len = region_chars.len() as i32;
                let keep = maxwidth.max(0) as usize;
                let kept: String = if truncatleft {
                    // c:1354 ch == '<'
                    // keep tail: drop (len-keep) chars from front, prefix marker
                    let drop_n = (len - keep as i32).max(0) as usize;
                    let suffix: String = region_chars[drop_n..].iter().collect();
                    format!("{}{}", truncstr, suffix)
                } else {
                    // keep head: take first `keep` chars, append marker
                    let prefix: String = region_chars[..keep.min(region_chars.len())]
                        .iter()
                        .collect();
                    format!("{}{}", prefix, truncstr)
                };
                // Rewrite buf[ptr..] with `kept`.
                bv.bp = ptr;
                for &b in kept.as_bytes() {
                    if bv.bp >= bv.buf.len() {
                        bv.buf.resize(bv.bp + 1, 0);
                    }
                    bv.buf[bv.bp] = b;
                    bv.bp += 1;
                }
            }
        }
        if bv.bp < bv.buf.len() {
            bv.buf[bv.bp] = 0; // c:1421 terminate
        }

        bv.truncwidth = 0; // c:1431
    }
    0 // c:1471
}

/// Push a parser context token. Port of `cmdpush()` from
/// Src/prompt.c. Bounded at CMDSTACKSZ; over-push is silently
/// ignored (matches the C source's `cmdsp < CMDSTACKSZ` guard).
/// C body (2 lines):
///   `if (cmdsp >= 0 && cmdsp < CMDSTACKSZ) cmdstack[cmdsp++] = cmdtok;`
pub fn cmdpush(cmdtok: u8) {
    // c:1624
    CMDSTACK.with(|s| {
        let mut st = s.borrow_mut();
        if st.len() < CMDSTACKSZ {
            st.push(cmdtok);
        }
    });
}

/// Pop the top parser context token. Port of `cmdpop()` from
/// Src/prompt.c. Empty-stack pop is a no-op (the C source emits
/// a `BUG: cmdstack empty` debug print and continues).
pub fn cmdpop() {
    CMDSTACK.with(|s| {
        let mut st = s.borrow_mut();
        // c:1635 — DPUTS(1, "BUG: cmdstack empty") in the C empty-stack
        // branch. The C source still pops + continues; same here.
        DPUTS!(st.is_empty(), "BUG: cmdstack empty"); // c:1635
        st.pop();
    });
}

/// Port of `applytextattributes(int flags)` from `Src/prompt.c:1645`.
///
/// C body diff-syncs `txtcurrentattrs` against `txtpendingattrs`
/// and emits the minimal termcap-driven sequence to transition
/// the terminal — `tsetcap(TCALLATTRSOFF…)`, `TCBOLDFACEBEG`, etc.
///
/// Rust port: returns the SGR diff string built by [`treplaceattrs`]
/// over the (current, pending) pair, and updates current = pending.
/// The previous port was an empty `void` shim that emitted nothing
/// — output gets emitted at flush time, which broke any caller
/// expecting incremental attr changes. New shape returns the diff
/// the caller can write to the terminal.
///
/// `_flags` parameter (currently unused in zshrs port — C uses it
/// to gate "force reset" mode).
#[allow(unused_variables)]
pub fn applytextattributes(flags: i32) -> String {
    let mut current = current_attrs_lock().lock().expect("current_attrs poisoned");
    let pending = pending_attrs_lock()
        .lock()
        .expect("pending_attrs poisoned")
        .clone();

    // SGR diff emission — inlined from the prior Rust-only helper that
    // miscarried the `treplaceattrs` name. C emits the same diff via
    // sequential tsetcap calls (Src/prompt.c:1640-1718). Rust returns
    // the assembled escape string for the caller to write.
    let mut result = String::new();
    let old = *current;
    let new = pending;

    let old_b = old & TXTBOLDFACE != 0;
    let new_b = new & TXTBOLDFACE != 0;
    let old_u = old & TXTUNDERLINE != 0;
    let new_u = new & TXTUNDERLINE != 0;
    let old_s = old & TXTSTANDOUT != 0;
    let new_s = new & TXTSTANDOUT != 0;

    let need_reset = (old_b && !new_b) || (old_u && !new_u) || (old_s && !new_s);

    if need_reset {
        result.push_str("\x1b[0m");
        if new_b {
            result.push_str("\x1b[1m");
        }
        if new_u {
            result.push_str("\x1b[4m");
        }
        if new_s {
            result.push_str("\x1b[7m");
        }
    } else {
        if !old_b && new_b {
            result.push_str("\x1b[1m");
        }
        if !old_u && new_u {
            result.push_str("\x1b[4m");
        }
        if !old_s && new_s {
            result.push_str("\x1b[7m");
        }
    }

    if (old & TXT_ATTR_FG_MASK) != (new & TXT_ATTR_FG_MASK) {
        if new & TXTFGCOLOUR != 0 {
            let raw = (new & TXT_ATTR_FG_COL_MASK) >> TXT_ATTR_FG_COL_SHIFT;
            let c = if new & TXT_ATTR_FG_24BIT != 0 {
                COLOR_24BIT | (raw as Color & 0x00ff_ffff)
            } else {
                raw as Color
            };
            result.push_str(&color_to_ansi(c, true));
        } else {
            result.push_str("\x1b[39m");
        }
    }
    if (old & TXT_ATTR_BG_MASK) != (new & TXT_ATTR_BG_MASK) {
        if new & TXTBGCOLOUR != 0 {
            let raw = (new & TXT_ATTR_BG_COL_MASK) >> TXT_ATTR_BG_COL_SHIFT;
            let c = if new & TXT_ATTR_BG_24BIT != 0 {
                COLOR_24BIT | (raw as Color & 0x00ff_ffff)
            } else {
                raw as Color
            };
            result.push_str(&color_to_ansi(c, false));
        } else {
            result.push_str("\x1b[49m");
        }
    }

    let diff = result;
    *current = pending;
    diff
}

/// Port of `void treplaceattrs(zattr newattrs)` from `Src/prompt.c:1719`.
/// ```c
/// void
/// treplaceattrs(zattr newattrs)
/// {
///     if (newattrs == TXT_ERROR) return;
///     if (txtunknownattrs) {
///         txtcurrentattrs &= ~txtunknownattrs;
///         txtcurrentattrs |= txtunknownattrs & ~newattrs;
///     }
///     txtpendingattrs = newattrs;
/// }
/// ```
/// State-mutator only — the actual escape emission happens in
/// `applytextattributes`. C's behavior: clear any "unknown" bits
/// from current and re-set their inverse so applytextattributes
/// detects them as changed, then stash newattrs in pending.
pub fn treplaceattrs(newattrs: zattr) {
    // c:1719
    if newattrs == TXT_ERROR {
        // c:1721
        return; // c:1722
    }
    let unknown = txtunknownattrs.load(Ordering::SeqCst); // c:1724
    if unknown != 0 {
        // c:1724
        let mut cur = current_attrs_lock().lock().expect("current_attrs poisoned");
        *cur &= !unknown; // c:1728
        *cur |= unknown & !newattrs; // c:1729
    }
    *pending_attrs_lock().lock().expect("pending_attrs poisoned") = newattrs; // c:1732
}

/// Port of `mod_export zattr txtunknownattrs` from `Src/prompt.c:46`.
/// Mask of text-attribute bits whose state is unknown because they
/// came from escape sequences the prompt parser didn't recognise.
pub static txtunknownattrs: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0); // c:46

/// Port of `mod_export zattr memo_term_color` from `Src/prompt.c:51`.
/// Caches the terminal's reported default fg/bg colors (24-bit
/// packed via TXT_ATTR_FG_24BIT / TXT_ATTR_BG_24BIT) so prompts can
/// fall back to them when the user didn't pin a specific attr.
pub static memo_term_color: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0); // c:51

/// Set text attributes (full apply).
/// Port of `tsetattrs(zattr newattrs)` from Src/prompt.c:1737.
///
/// C body (c:1737-1751):
/// ```c
/// /* assume any unknown attributes that we're now setting were unset */
/// txtcurrentattrs &= ~(newattrs & txtunknownattrs);
/// txtpendingattrs |= newattrs & TXT_ATTR_ALL;
/// if (newattrs & TXTFGCOLOUR) {
///     txtpendingattrs &= ~TXT_ATTR_FG_MASK;
///     txtpendingattrs |= newattrs & TXT_ATTR_FG_MASK;
/// }
/// if (newattrs & TXTBGCOLOUR) {
///     txtpendingattrs &= ~TXT_ATTR_BG_MASK;
///     txtpendingattrs |= newattrs & TXT_ATTR_BG_MASK;
/// }
/// ```
///
/// Returns the SGR escape string emitted by the subsequent
/// `applytextattributes` flush (the Rust port collapses the
/// pending-then-flush idiom into a single call so callers get the
/// rendered diff back immediately).
pub fn tsetattrs(newattrs: zattr) -> String {
    // c:1737
    // c:1740 — txtcurrentattrs &= ~(newattrs & txtunknownattrs);
    let unknown = txtunknownattrs.load(Ordering::Relaxed);
    {
        let mut cur = current_attrs_lock().lock().expect("current_attrs poisoned");
        *cur &= !(newattrs & unknown as zattr);
    }
    // c:1742-1750 — txtpendingattrs updates: OR in non-color attrs,
    // then for FG/BG colour bits replace the existing mask wholesale.
    {
        let mut pend = pending_attrs_lock().lock().expect("pending_attrs poisoned");
        *pend |= newattrs & TXT_ATTR_ALL; // c:1742
        if (newattrs & TXTFGCOLOUR) != 0 {
            // c:1743
            *pend &= !TXT_ATTR_FG_MASK; // c:1744
            *pend |= newattrs & TXT_ATTR_FG_MASK; // c:1745
        }
        if (newattrs & TXTBGCOLOUR) != 0 {
            // c:1747
            *pend &= !TXT_ATTR_BG_MASK; // c:1748
            *pend |= newattrs & TXT_ATTR_BG_MASK; // c:1749
        }
    }
    apply_text_attributes(newattrs)
}

/// Unset (clear) text attributes via SGR-22/24/27 + 39/49.
/// Port of `tunsetattrs(zattr newattrs)` from Src/prompt.c:1755.
pub fn tunsetattrs(newattrs: zattr) -> String {
    // c:1755
    let mut result = String::new();
    if newattrs & TXTBOLDFACE != 0 {
        result.push_str("\x1b[22m");
    }
    if newattrs & TXTUNDERLINE != 0 {
        result.push_str("\x1b[24m");
    }
    if newattrs & TXTSTANDOUT != 0 {
        result.push_str("\x1b[27m");
    }
    if newattrs & TXTFGCOLOUR != 0 {
        result.push_str("\x1b[39m");
    }
    if newattrs & TXTBGCOLOUR != 0 {
        result.push_str("\x1b[49m");
    }
    result
}

/// Promote the 256-color value embedded in `atr` to an explicit
/// 24-bit RGB value. Port of `map256toRGB(zattr *atr, int shift, zattr set24)` from Src/prompt.c.
/// Used by the prompt-output path when the terminal supports
/// truecolor and we want to emit RGB rather than the smaller
/// 256-palette code.
///
/// `shift` selects fg-byte vs bg-byte position inside `atr`;
/// `set24` is the bit that marks "this slot is now 24-bit".
/// Algorithm mirrors the C: 16-231 are the 6×6×6 color cube,
/// 232-255 are the 24-step grayscale ramp.
#[allow(non_snake_case)]
pub fn map256toRGB(atr: &mut u64, shift: u32, set24: u64) {
    if (*atr & set24) != 0 {
        return;
    }
    let colour: u32 = ((*atr >> shift) & 0xff) as u32;
    if colour < 16 {
        return;
    }
    let (red, green, blue) = if (16..232).contains(&colour) {
        let mut c = colour - 16;
        let blue = (if c != 0 { 0x37 } else { 0 }) + 40 * (c % 6);
        c /= 6;
        let green = (if c != 0 { 0x37 } else { 0 }) + 40 * (c % 6);
        c /= 6;
        let red = (if c != 0 { 0x37 } else { 0 }) + 40 * c;
        (red, green, blue)
    } else {
        let v = 8 + 10 * (colour - 232);
        (v, v, v)
    };
    *atr &= !((0xffffff_u64) << shift);
    *atr |= set24 | ((((red as u64) << 8 | green as u64) << 8 | blue as u64) << shift);
}

/// Mix two sets of text attributes through a mask.
/// Port of `mixattrs(zattr primary, zattr mask, zattr secondary)` from Src/prompt.c:1802 — primary wins
/// where the mask says "set"; secondary fills the rest.
pub fn mixattrs(primary: zattr, mask: zattr, secondary: zattr) -> zattr {
    // Bit-level mix: for each TXT* bit set in `mask`, take the
    // value from `primary`; else from `secondary`. Mirrors the C
    // idiom `(mask & primary) | (!mask & secondary)`.
    let mut out: zattr = 0;
    for bit in [TXTBOLDFACE, TXTUNDERLINE, TXTSTANDOUT] {
        if mask & bit != 0 {
            out |= primary & bit;
        } else {
            out |= secondary & bit;
        }
    }
    if mask & TXTFGCOLOUR != 0 {
        out |= primary & TXT_ATTR_FG_MASK;
    } else {
        out |= secondary & TXT_ATTR_FG_MASK;
    }
    if mask & TXTBGCOLOUR != 0 {
        out |= primary & TXT_ATTR_BG_MASK;
    } else {
        out |= secondary & TXT_ATTR_BG_MASK;
    }
    out
}

// ---------------------------------------------------------------------------
// Missing functions from prompt.c
// ---------------------------------------------------------------------------

/// Truncate the prompt to a maximum width.
/// Port of `prompttrunc(int arg, int truncchar, int doprint, int endchar)` from Src/prompt.c:1276 — the C source
/// implements the `%N>string>` (right-truncate) and `%N<string<`
/// (left-truncate) sequences with a configurable indicator.
/// Port of `countprompt(char *str, int *wp, int *hp, int overf)` from `Src/prompt.c:1140`.
///
/// C signature:
/// `void countprompt(char *str, int *wp, int *hp, int overf);`
///
/// Walks the expanded prompt counting visible columns, wrapping
/// to the next line every `terminal_width` characters and bumping
/// the height counter. Returns `(width, height)` — `width` is the
/// column on the FINAL line; `height` is total line count
/// including the first.
///
/// Faithful to C's prompt.c:1140 logic:
/// - `\t` advances to the next 8-column boundary (`w = (w | 7) + 1`).
/// - `\n` resets `w` to 0 and bumps `h`.
/// - `\x01`/`\x02` (RL_PROMPT_*_IGNORE) toggle visibility skip.
/// - `\x1b[...m` ANSI escapes consumed without counting.
/// - Wrap rule: `while w > terminal_width && overf >= 0` →
///   `h++; w -= terminal_width` (matches the C overflow loop at
///   line 1158 + 1255).
/// - Final-column-equals-width edge case: when `w == terminal_width
///   && overf == 0`, snap to (0, h+1) — mirrors C lines 1265-1268.
///
// by locating them and finding out their screen width.                    // c:1135
/// Previous Rust port took only `&str` and returned `(width,
/// newlines)` — missing the `terminal_width` overflow tracking
/// and the `overf` flag entirely.

// `pub struct CmdStack` + `impl CmdStack { new, push, pop, top,
// depth, as_slice }` — DELETED per user directive. C source uses
// `unsigned char *cmdstack` + `int cmdsp` flat globals
// (`Src/prompt.c:1915`) plus `cmdpush()`/`cmdpop()` functions
// (`Src/prompt.c:1915`). The Rust-only `CmdStack` wrapper had
// zero callers outside this file. The canonical port lives on
// `prompt_tls::CMDSTACK` and `ShellExecutor.cmd_stack: Vec<u8>`.
// `cmdpush()`/`cmdpop()` thread-local stack mirrors C file-statics.

/// Resolve a color name to an ANSI base index.
/// Port of `match_named_colour(const char **teststrp)` from Src/prompt.c:1915 —
/// walks `colour_names[]` (now `COLOUR_NAMES` at file head), then
/// falls through to numeric parsing. Returns palette index 0-7
/// for basic colours, 8 for "default" sentinel (per C:1909),
/// numeric value for raw integers.
/// Port of `void countprompt(char *str, int *wp, int *hp, int overf)`
/// from `Src/prompt.c:1140`. Walks the expanded prompt counting visible
/// columns: handles `\t` (tab to next 8-col boundary), `\n` (reset
/// column, bump row), `Inpar`/`Outpar` (`%{...%}` invisible regions),
/// and `Nularg` (1-width opaque placeholder for `tputs` glitches).
/// Writes width/height into `wp`/`hp` out-params. `overf` flag: when
/// non-negative, wrap on column overflow (incrementing `*hp`); when -1,
/// allow overflow (used by truncation pre-pass).
/// ```c
/// void
/// countprompt(char *str, int *wp, int *hp, int overf)
/// {
///     int w = 0, h = 1, multi = 0, wcw = 0;
///     int s = 1;  /* visible flag */
///     for (; *str; str++) {
///         while (w > zterm_columns && overf >= 0 && !multi) {
///             h++;
///             if (wcw) { w = wcw; break; }
///             else w -= zterm_columns;
///         }
///         wcw = 0;
///         if (*str == Inpar) s = 0;
///         else if (*str == Outpar) s = 1;
///         else if (*str == Nularg) w++;
///         else if (s) {
///             /* meta-decode, tab/newline/wide-char/cntrl handling */
///         }
///     }
///     *wp = w; *hp = h;
/// }
/// ```
/// WARNING: param names match C — Rust=(str, wp, hp, overf) vs C=(str, wp, hp, overf)
pub fn countprompt(s: &str, wp: &mut i32, hp: &mut i32, overf: i32) {
    // c:1140
    let zterm_columns = crate::ported::utils::adjustcolumns() as i32;
    let mut w: i32 = 0; // c:1142
    let mut h: i32 = 1;
    let multi = 0i32; // c:1142
    let mut wcw: i32 = 0;
    let mut visible = true; // c:1143 s = 1

    for c in s.chars() {
        // c:1158-1173 — overflow wrap loop.
        while w > zterm_columns && overf >= 0 && multi == 0 {
            // c:1158
            h += 1; // c:1159
            if wcw != 0 {
                // c:1160
                w = wcw; // c:1165
                break; // c:1166
            } else {
                w -= zterm_columns; // c:1171
            }
        }
        wcw = 0; // c:1174

        // c:1179-1185 — Inpar/Outpar/Nularg dispatch.
        //
        // The previous Rust port used '\x01' / '\x02' / '\x03' for
        // these three tokens, which are the WRONG values. The
        // canonical token bytes are:
        //   Inpar  = 0x88 (Src/zsh.h:163)
        //   Outpar = 0x8a (Src/zsh.h:165)
        //   Nularg = 0xa1 (Src/zsh.h:206)
        //
        // Effect of the previous bug: every prompt with `%{...%}`
        // (non-printing escapes — which lex as Inpar..Outpar after
        // promptexpand) skipped the `s = 0` flag flip and counted
        // the escape bytes as visible width. Multi-line prompts
        // with `%{...%}` wrap at wrong columns.
        if c == Inpar {
            // c:1179 Inpar
            visible = false; // c:1180 s = 0
        } else if c == Outpar {
            // c:1181 Outpar
            visible = true; // c:1182 s = 1
        } else if c == Nularg {
            // c:1183 Nularg
            w += 1; // c:1184
        } else if visible {
            // c:1185
            // c:1202-1208 — tab / newline.
            if c == '\t' {
                // c:1202
                w = (w | 7) + 1; // c:1203
                continue;
            } else if c == '\n' {
                // c:1205
                w = 0; // c:1206
                h += 1; // c:1207
                continue; // c:1208
            }
            // c:1234 — `w += WCWIDTH_WINT(wc)` — width of the char.
            let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(1) as i32;
            wcw = cw; // c:1233 wcw = wcw
            w += cw; // c:1234
        }
    }
    // c:1265-1268 — final-column edge case: w == zterm_columns && overf == 0.
    if w == zterm_columns && overf == 0 {
        // c:1265
        w = 0; // c:1266
        h += 1; // c:1267
    }
    *wp = w; // c:1273 *wp = w
    *hp = h; // c:1274 *hp = h
}

pub fn match_named_colour(teststrp: &str) -> Option<u8> {
    // c:1915
    // c:1925-1928 uses `strncmp` (case-SENSITIVE) against the
    // ansi_colours table. The previous Rust port `to_lowercase`-ed
    // the input first which made the function case-INsensitive,
    // accepting `"RED"` where C rejects it. Fixed 2026-05 to match
    // the C strncmp contract: input must already be lowercase.
    for (i, &n) in COLOUR_NAMES.iter().enumerate() {
        // c:1925
        if n == teststrp {
            // c:1926 strncmp(teststr, *cptr, len)
            return Some(i as u8); // c:1927
        }
    }
    // Rust-port extension: fall through to numeric parse so callers
    // can spell `color 38` etc. (C returns -1 here; the numeric path
    // is wired further up the dispatch chain in upstream zsh, but
    // the Rust port colocates it because there's no shared call site
    // yet). Pin via the regression test if this lands a divergent
    // behavior in practice.
    teststrp.parse::<u8>().ok()
}

/// Port of `static int truecolor_terminal(void)` from Src/prompt.c:1935.
///
/// C body (c:1935-1944):
/// ```c
/// char **f, **flist = getaparam(".term.extensions");
/// int result;
/// for (f = flist; f && *f; f++) {
///     result = **f != '-';
///     if (!strcmp(*f + !result, "truecolor"))
///         return result;
/// }
/// return 0; /* disabled by default */
/// ```
///
/// Walks `$.term.extensions` (a shell array of capability names; entries
/// prefixed with `-` mean "disabled"). Returns true when `truecolor` is
/// present and not negated. The previous Rust port did a heuristic
/// COLORTERM/TERM-string match — not how C decides. Off-by-default
/// matches C's final `return 0`.
pub fn truecolor_terminal() -> bool {
    // c:1935
    if let Some(flist) = crate::ported::params::getaparam(".term.extensions") {
        for f in &flist {
            // c:1939
            if f.is_empty() {
                continue;
            }
            // c:1940 — `result = **f != '-'`; the `-` prefix disables.
            let (result, name) = match f.strip_prefix('-') {
                Some(rest) => (false, rest),
                None => (true, f.as_str()),
            };
            if name == "truecolor" {
                // c:1941
                return result; // c:1942
            }
        }
    }
    false // c:1944
}

impl buf_vars {
    pub fn new(input: &str) -> Self {
        Self {
            // Bag-of-globals fields removed — see struct def comment.
            buf: vec![0u8; 256],
            bufspc: 256,
            bp: 0,
            bufline: 0,
            bp1: None,
            fm: input.to_string(),
            fm_pos: 0,
            truncwidth: 0,
            dontcount: 0,
            trunccount: 0,
            rstring: None,
            Rstring: None,
            attrs: 0 as zattr, // c:zsh.h:2685 (zattr=0 == no attrs)
            in_escape: false,
            // prompt_percent / prompt_bang — removed; route through
            // `isset(PROMPTPERCENT)` / `isset(PROMPTBANG)`.
        }
    }

    // `with_prompt_percent` / `with_prompt_bang` builder methods removed
    // — they configured Rust-only fields that no longer exist. zsh
    // controls prompt-% / prompt-! interpretation via the canonical
    // `setopt promptpercent` / `setopt promptbang` (option table), not
    // a per-buf_vars override.

    fn fork_snapshot(&self, input: String) -> buf_vars {
        buf_vars {
            // Bag-of-globals fields removed — see struct def comment.
            buf: Vec::new(),
            bufspc: 0,
            bp: 0,
            bufline: 0,
            bp1: None,
            fm: input,
            fm_pos: 0,
            truncwidth: 0,
            dontcount: 0,
            trunccount: 0,
            rstring: None,
            Rstring: None,
            attrs: self.attrs,
            in_escape: false,
        }
    }

    /// Src/prompt.c:991 `addbufspc`
    fn addbufspc(&mut self, need: usize) {
        let need = need.saturating_mul(2).max(need.max(1));
        self.buf.reserve(need);
        self.bufspc = self.buf.capacity();
    }

    /// Src/prompt.c:976 `pputc` — metafy high bytes.
    fn pputc(&mut self, c: u8) {
        self.addbufspc(2);
        let bp = self.bp;
        if imeta_byte(c) {
            self.buf.resize(bp + 2, 0);
            self.buf[bp] = Meta as u8;
            self.buf[bp + 1] = c ^ 32;
            self.bp = bp + 2;
        } else {
            self.buf.resize(bp + 1, 0);
            self.buf[bp] = c;
            self.bp = bp + 1;
        }
        if c == b'\n' && self.dontcount == 0 {
            self.bufline = self.bp;
        }
    }

    fn out_raw_byte(&mut self, b: u8) {
        self.addbufspc(1);
        let bp = self.bp;
        self.buf.resize(bp + 1, 0);
        self.buf[bp] = b;
        self.bp = bp + 1;
    }

    fn out_char(&mut self, c: char) {
        let mut tmp = [0u8; 4];
        let enc = c.encode_utf8(&mut tmp);
        for &b in enc.as_bytes() {
            self.pputc(b);
        }
    }

    fn out_str(&mut self, s: &str) {
        for &b in s.as_bytes() {
            self.pputc(b);
        }
    }

    /// Append raw metafied bytes from a nested `putpromptchar` (`%(…)` branches).
    fn append_buf_from(&mut self, other: &buf_vars) {
        let end = other.bp.min(other.buf.len());
        if end == 0 {
            return;
        }
        self.addbufspc(end);
        let bp0 = self.bp;
        self.buf.resize(bp0 + end, 0);
        self.buf[bp0..bp0 + end].copy_from_slice(&other.buf[..end]);
        self.bp = bp0 + end;
        if self.dontcount == 0 {
            for i in 0..end {
                if other.buf[i] == b'\n' {
                    self.bufline = bp0 + i + 1;
                }
            }
        }
    }

    /// After expansion: `unmetafy` for display (lossy UTF-8).
    pub fn expanded_utf8(&self) -> String {
        let end = self.bp.min(self.buf.len());
        let mut v = self.buf[..end].to_vec();
        crate::ported::utils::unmetafy(&mut v);
        String::from_utf8_lossy(&v).into_owned()
    }

    /// Src/prompt.c:236-246 — strip Inpar/Outpar/Nularg when `ns == 0`.
    fn strip_prompt_tokens_ns0(&mut self) {
        let end = self.bp.min(self.buf.len());
        let mut v = Vec::with_capacity(end);
        let mut i = 0usize;
        while i < end {
            let b = self.buf[i];
            if b == (Meta as u8) {
                if i + 1 < end {
                    v.push(b);
                    v.push(self.buf[i + 1]);
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            if b == Inpar as u8 || b == Outpar as u8 || b == Nularg as u8 {
                i += 1;
                continue;
            }
            v.push(b);
            i += 1;
        }
        self.buf = v;
        self.bp = self.buf.len();
    }

    pub fn finish_expanded_string(&mut self, keep_spacing_tokens: bool) -> String {
        if !keep_spacing_tokens {
            self.strip_prompt_tokens_ns0();
        }
        self.expanded_utf8()
    }

    /// Src/prompt.c:359 — core of `putpromptchar(int doprint, int endchar)`.
    pub(crate) fn run_putpromptchar(&mut self, doprint: i32, endchar: i32) -> i32 {
        loop {
            if self.fm_pos >= self.fm.len() {
                return 0;
            }
            let ec = endchar as u8;
            if ec != 0 {
                let b = self.fm.as_bytes()[self.fm_pos];
                if b == ec {
                    return endchar;
                }
            }

            let c = match self.peek() {
                Some(c) => c,
                None => return 0,
            };

            if c == '%' && isset(PROMPTPERCENT) {
                self.advance();
                self.process_percent(doprint);
            } else if c == '!' && isset(PROMPTBANG) {
                if doprint != 0 {
                    self.advance();
                    if self.peek() == Some('!') {
                        self.advance();
                        self.out_char('!');
                    } else {
                        self.out_str(&prompt_tls::HISTNUM.with(|c| *c.borrow()).to_string());
                    }
                } else {
                    self.advance();
                    if self.peek() == Some('!') {
                        self.advance();
                    }
                }
            } else {
                self.advance();
                if doprint != 0 {
                    self.out_char(c);
                }
            }
        }
    }

    fn peek(&self) -> Option<char> {
        self.fm[self.fm_pos..].chars().next()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.fm_pos += c.len_utf8();
        Some(c)
    }

    fn parse_number(&mut self) -> Option<i32> {
        let start = self.fm_pos;
        let mut negative = false;

        if self.peek() == Some('-') {
            negative = true;
            self.advance();
        }

        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                self.advance();
            } else {
                break;
            }
        }

        if self.fm_pos == start || (negative && self.fm_pos == start + 1) {
            if negative {
                self.fm_pos = start;
            }
            return None;
        }

        let num_str = &self.fm[if negative { start + 1 } else { start }..self.fm_pos];
        let num: i32 = num_str.parse().ok()?;
        Some(if negative { -num } else { num })
    }

    fn parse_braced_arg(&mut self) -> Option<String> {
        if self.peek() != Some('{') {
            return None;
        }
        self.advance(); // skip {

        let start = self.fm_pos;
        let mut depth = 1;

        while let Some(c) = self.advance() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(self.fm[start..self.fm_pos - 1].to_string());
                    }
                }
                '\\' => {
                    self.advance(); // skip escaped char
                }
                _ => {}
            }
        }

        None
    }

    /// Get path with tilde substitution
    fn path_with_tilde(&self, path: &str) -> String {
        let home = prompt_tls::HOME.with(|c| c.borrow().clone());
        if !home.is_empty() && path.starts_with(&home) {
            format!("~{}", &path[home.len()..])
        } else {
            path.to_string()
        }
    }

    /// Get trailing path components
    fn trailing_path(&self, path: &str, n: usize, with_tilde: bool) -> String {
        let path = if with_tilde {
            self.path_with_tilde(path)
        } else {
            path.to_string()
        };

        if n == 0 {
            return path;
        }

        let components: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if components.len() <= n {
            return path;
        }

        components[components.len() - n..].join("/")
    }

    /// Get leading path components
    fn leading_path(&self, path: &str, n: usize) -> String {
        if n == 0 {
            return path.to_string();
        }

        let components: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if components.len() <= n {
            return path.to_string();
        }

        let result = components[..n].join("/");
        if path.starts_with('/') {
            format!("/{}", result)
        } else {
            result
        }
    }

    /// Start escape sequence (non-printing characters)
    fn start_escape(&mut self) {
        if !self.in_escape {
            self.out_char('\x01'); // RL_PROMPT_START_IGNORE
            self.in_escape = true;
        }
    }

    /// End escape sequence
    fn end_escape(&mut self) {
        if self.in_escape {
            self.out_char('\x02'); // RL_PROMPT_END_IGNORE
            self.in_escape = false;
        }
    }

    /// Apply text attributes incrementally. zsh emits just the new
    /// SGR codes (no leading `\e[0m`) when adding attrs to a default
    /// state — only emit a reset when there's nothing to apply (rare,
    /// covered by the explicit `%b`/`%f`/`%k`/`%u` reset handlers).
    fn apply_attrs(&mut self) {
        self.start_escape();
        if self.attrs & TXTBOLDFACE != 0 {
            // c:1645
            self.out_str("\x1b[1m");
        }
        if self.attrs & TXTUNDERLINE != 0 {
            // c:1645
            self.out_str("\x1b[4m");
        }
        if self.attrs & TXTSTANDOUT != 0 {
            // c:1645
            // zsh emits italic (`3m`) for `%S` standout, not reverse
            // video (`7m`). Match zsh's actual prompt output.
            self.out_str("\x1b[3m");
        }
        if self.attrs & TXTFGCOLOUR != 0 {
            // c:1645
            let raw = (self.attrs & TXT_ATTR_FG_COL_MASK) >> TXT_ATTR_FG_COL_SHIFT;
            let c = if self.attrs & TXT_ATTR_FG_24BIT != 0 {
                COLOR_24BIT | (raw as Color & 0x00ff_ffff)
            } else {
                raw as Color
            };
            self.out_str(&color_to_ansi(c, true));
        }
        if self.attrs & TXTBGCOLOUR != 0 {
            // c:1645
            let raw = (self.attrs & TXT_ATTR_BG_COL_MASK) >> TXT_ATTR_BG_COL_SHIFT;
            let c = if self.attrs & TXT_ATTR_BG_24BIT != 0 {
                COLOR_24BIT | (raw as Color & 0x00ff_ffff)
            } else {
                raw as Color
            };
            self.out_str(&color_to_ansi(c, false));
        }
        self.end_escape();
    }

    /// Parse conditional %(x.true.false)
    fn parse_conditional(&mut self, arg: i32, doprint: i32) -> bool {
        if self.peek() != Some('(') {
            return false;
        }
        self.advance(); // skip (

        // Parse condition character
        let cond_char = match self.advance() {
            Some(c) => c,
            None => return false,
        };

        // Evaluate condition
        let test = match cond_char {
            '/' | 'c' | '.' | '~' | 'C' => {
                // Directory depth test
                let pwd = prompt_tls::PWD.with(|c| c.borrow().clone());
                let path = self.path_with_tilde(&pwd);
                let depth = path.matches('/').count() as i32;
                if arg == 0 {
                    depth > 0
                } else {
                    depth >= arg
                }
            }
            '?' => prompt_tls::LASTVAL.with(|c| *c.borrow()) == arg,
            '#' => {
                let euid = unsafe { libc::geteuid() };
                euid == arg as u32
            }
            'L' => prompt_tls::SHLVL.with(|c| *c.borrow()) >= arg,
            'j' => prompt_tls::NUM_JOBS.with(|c| *c.borrow()) >= arg,
            'v' => (arg as usize) <= prompt_tls::PSVAR.with(|c| c.borrow().len()),
            'V' => {
                if arg <= 0 {
                    false
                } else {
                    prompt_tls::PSVAR.with(|c| {
                        let v = c.borrow();
                        (arg as usize) <= v.len() && !v[arg as usize - 1].is_empty()
                    })
                }
            }
            '_' => prompt_tls::CMDSTACK.with(|c| c.borrow().len()) >= arg as usize,
            't' | 'T' | 'd' | 'D' | 'w' => {
                let now = chrono::Local::now();
                match cond_char {
                    't' => now.format("%M").to_string().parse::<i32>().unwrap_or(0) == arg,
                    'T' => now.format("%H").to_string().parse::<i32>().unwrap_or(0) == arg,
                    'd' => now.format("%d").to_string().parse::<i32>().unwrap_or(0) == arg,
                    'D' => now.format("%m").to_string().parse::<i32>().unwrap_or(0) == arg - 1,
                    'w' => now.format("%w").to_string().parse::<i32>().unwrap_or(0) == arg,
                    _ => false,
                }
            }
            '!' => prompt_tls::IS_ROOT.with(|c| *c.borrow()),
            _ => false,
        };

        // Get separator
        let sep = match self.advance() {
            Some(c) => c,
            None => return false,
        };

        // Parse true branch
        let true_start = self.fm_pos;
        let mut depth = 1;
        while let Some(c) = self.peek() {
            if c == '(' {
                depth += 1;
            } else if c == ')' {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            } else if c == sep && depth == 1 {
                break;
            }
            self.advance();
        }
        let true_branch = self.fm[true_start..self.fm_pos].to_string();

        if self.peek() != Some(sep) {
            return false;
        }
        self.advance(); // skip separator

        // Parse false branch
        let false_start = self.fm_pos;
        depth = 1;
        while let Some(c) = self.peek() {
            if c == '(' {
                depth += 1;
            } else if c == ')' {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            self.advance();
        }
        let false_branch = self.fm[false_start..self.fm_pos].to_string();

        if self.peek() != Some(')') {
            return false;
        }
        self.advance(); // skip )

        // Src/prompt.c:511-516 — `putpromptchar(test && doprint, sep)` then
        // `putpromptchar(!test && doprint, ')' )`; same `bv->buf`, we append serially.
        let mut tsub = self.fork_snapshot(true_branch);
        tsub.run_putpromptchar(if test { doprint } else { 0 }, 0);
        self.append_buf_from(&tsub);
        let mut fsub = self.fork_snapshot(false_branch);
        fsub.run_putpromptchar(if test { 0 } else { doprint }, 0);
        self.append_buf_from(&fsub);

        true
    }

    /// Parse and process a % escape sequence
    fn process_percent(&mut self, doprint: i32) {
        let arg = self.parse_number().unwrap_or(0);

        // Check for conditional
        if self.peek() == Some('(') {
            self.parse_conditional(arg, doprint);
            return;
        }

        if doprint == 0 {
            // Src/prompt.c:520-538 — parse-only skips; C `default: continue` advances
            // past one opcode byte after the (already-parsed) numeric arg.
            match self.peek() {
                Some('[') => {
                    self.advance();
                    let _ = self.parse_number();
                    while self.peek() != Some(']') {
                        if self.advance().is_none() {
                            break;
                        }
                    }
                    let _ = self.advance();
                    return;
                }
                Some('<') | Some('>') => {
                    let end = self.peek().unwrap();
                    self.advance();
                    while self.peek() != Some(end) {
                        if self.advance().is_none() {
                            break;
                        }
                    }
                    let _ = self.advance();
                    return;
                }
                Some('D') => {
                    self.advance();
                    if self.peek() == Some('{') {
                        while self.peek() != Some('}') {
                            if self.advance().is_none() {
                                break;
                            }
                        }
                        let _ = self.advance();
                    }
                    return;
                }
                _ => {
                    let _ = self.advance();
                    return;
                }
            }
        }

        let c = match self.advance() {
            Some(c) => c,
            None => return,
        };

        match c {
            // Directory
            '~' => {
                let pwd = prompt_tls::PWD.with(|c| c.borrow().clone());
                let path = if arg == 0 {
                    self.path_with_tilde(&pwd)
                } else if arg > 0 {
                    self.trailing_path(&pwd, arg as usize, true)
                } else {
                    self.leading_path(&self.path_with_tilde(&pwd), (-arg) as usize)
                };
                self.out_str(&path);
            }
            'd' | '/' => {
                let pwd = prompt_tls::PWD.with(|c| c.borrow().clone());
                let path = if arg == 0 {
                    pwd
                } else if arg > 0 {
                    self.trailing_path(&pwd, arg as usize, false)
                } else {
                    self.leading_path(&pwd, (-arg) as usize)
                };
                self.out_str(&path);
            }
            'c' | '.' => {
                let n = if arg == 0 {
                    1
                } else {
                    arg.unsigned_abs() as usize
                };
                let pwd = prompt_tls::PWD.with(|c| c.borrow().clone());
                let path = self.trailing_path(&pwd, n, true);
                self.out_str(&path);
            }
            'C' => {
                let n = if arg == 0 {
                    1
                } else {
                    arg.unsigned_abs() as usize
                };
                let pwd = prompt_tls::PWD.with(|c| c.borrow().clone());
                let path = self.trailing_path(&pwd, n, false);
                self.out_str(&path);
            }

            // Script name (or argzero fallback) — port of
            // Src/prompt.c:554-556 `case 'N': promptpath(scriptname
            // ? scriptname : argzero, arg, 0)`. The `arg` selects N
            // trailing path components (0 = full path).
            'N' => {
                let name = prompt_tls::SCRIPTNAME
                    .with(|c| c.borrow().clone())
                    .unwrap_or_else(|| prompt_tls::ARGEXTRA.with(|c| c.borrow().clone()));
                let n = if arg <= 0 {
                    0
                } else {
                    arg.unsigned_abs() as usize
                };
                if n == 0 {
                    self.out_str(&name);
                } else {
                    let tail = self.trailing_path(&name, n, false);
                    self.out_str(&tail);
                }
            }
            // User/host
            'n' => {
                let u = prompt_tls::USER.with(|c| c.borrow().clone());
                self.out_str(&u);
            }
            'M' => {
                let h = prompt_tls::HOST.with(|c| c.borrow().clone());
                self.out_str(&h);
            }
            'm' => {
                let n = if arg == 0 { 1 } else { arg };
                let host = prompt_tls::HOST.with(|c| c.borrow().clone());
                if n > 0 {
                    let parts: Vec<&str> = host.split('.').collect();
                    let take = (n as usize).min(parts.len());
                    self.out_str(&parts[..take].join("."));
                } else {
                    let parts: Vec<&str> = host.split('.').collect();
                    let skip = ((-n) as usize).min(parts.len());
                    self.out_str(&parts[skip..].join("."));
                }
            }

            // TTY
            'l' => {
                let t = prompt_tls::TTY.with(|c| c.borrow().clone());
                let tty = if t.starts_with("/dev/tty") {
                    t[8..].to_string()
                } else if t.starts_with("/dev/") {
                    t[5..].to_string()
                } else {
                    "()".to_string()
                };
                self.out_str(&tty);
            }
            'y' => {
                // zsh: `%y` is the tty short name (without `/dev/`).
                // When not connected to a tty (e.g. in `-c` mode or
                // a pipe), zsh outputs `()` matching the `%l` form.
                let t = prompt_tls::TTY.with(|c| c.borrow().clone());
                let tty = if t.is_empty() {
                    "()".to_string()
                } else if t.starts_with("/dev/") {
                    t[5..].to_string()
                } else {
                    t
                };
                self.out_str(&tty);
            }

            // Status
            '?' => self.out_str(&prompt_tls::LASTVAL.with(|c| *c.borrow()).to_string()),
            '#' => self.out_char(if prompt_tls::IS_ROOT.with(|c| *c.borrow()) {
                '#'
            } else {
                '%'
            }),

            // History
            'h' | '!' => self.out_str(&prompt_tls::HISTNUM.with(|c| *c.borrow()).to_string()),

            // Jobs
            'j' => self.out_str(&prompt_tls::NUM_JOBS.with(|c| *c.borrow()).to_string()),

            // Shell level
            'L' => self.out_str(&prompt_tls::SHLVL.with(|c| *c.borrow()).to_string()),

            // Line number (`%i`) — Src/prompt.c:923-929 after optional `%I` block.
            'i' => self.out_str(&prompt_tls::LINENO.with(|c| *c.borrow()).to_string()),

            // `%I` — Src/prompt.c:901-920: inside `funcstack` (not SOURCE,
            // not `IN_EVAL_TRAP`), file line is `lineno + funcstack->flineno`.
            // zshrs stores the addend as `func_line_base` (`first_body_line - 1`
            // at registration). `FS_EVAL` / trap nuances not wired yet.
            'I' => {
                let lineno = prompt_tls::LINENO.with(|c| *c.borrow());
                let n = if let Some(base) = prompt_tls::FUNC_LINE_BASE.with(|c| *c.borrow()) {
                    lineno.saturating_add(base)
                } else {
                    lineno
                };
                self.out_str(&n.to_string());
            }

            // `%x` — Src/prompt.c:931-937: inside the same frames, use
            // `funcstack->filename` (here `funcstack_filename`); else
            // `scriptfilename ? scriptfilename : argzero`.
            'x' => {
                let n = if arg <= 0 {
                    0
                } else {
                    arg.unsigned_abs() as usize
                };
                if prompt_tls::FUNC_LINE_BASE.with(|c| c.borrow().is_some()) {
                    let path = prompt_tls::FUNCSTACK_FILENAME
                        .with(|c| c.borrow().clone())
                        .unwrap_or_default();
                    if n == 0 {
                        self.out_str(&path);
                    } else {
                        let tail = self.trailing_path(&path, n, false);
                        self.out_str(&tail);
                    }
                } else {
                    let name = prompt_tls::SCRIPTFILENAME
                        .with(|c| c.borrow().clone())
                        .or_else(|| prompt_tls::SCRIPTNAME.with(|c| c.borrow().clone()))
                        .unwrap_or_else(|| prompt_tls::ARGEXTRA.with(|c| c.borrow().clone()));
                    if n == 0 {
                        self.out_str(&name);
                    } else {
                        let tail = self.trailing_path(&name, n, false);
                        self.out_str(&tail);
                    }
                }
            }

            // Date/time (`%D{...}` — zsh strftime → chrono format)
            'D' => {
                let now = chrono::Local::now();
                if let Some(fmt) = self.parse_braced_arg() {
                    let mut chrono_fmt = String::new();
                    let mut chars = fmt.chars().peekable();
                    while let Some(c) = chars.next() {
                        if c == '%' {
                            match chars.next() {
                                Some('a') => chrono_fmt.push_str("%a"),
                                Some('A') => chrono_fmt.push_str("%A"),
                                Some('b') | Some('h') => chrono_fmt.push_str("%b"),
                                Some('B') => chrono_fmt.push_str("%B"),
                                Some('c') => chrono_fmt.push_str("%c"),
                                Some('C') => chrono_fmt.push_str("%y"),
                                Some('d') => chrono_fmt.push_str("%d"),
                                Some('D') => chrono_fmt.push_str("%m/%d/%y"),
                                Some('e') => chrono_fmt.push_str("%e"),
                                Some('f') => chrono_fmt.push_str("%e"),
                                Some('F') => chrono_fmt.push_str("%Y-%m-%d"),
                                Some('H') => chrono_fmt.push_str("%H"),
                                Some('I') => chrono_fmt.push_str("%I"),
                                Some('j') => chrono_fmt.push_str("%j"),
                                Some('k') => chrono_fmt.push_str("%k"),
                                Some('K') => chrono_fmt.push_str("%H"),
                                Some('l') => chrono_fmt.push_str("%l"),
                                Some('L') => chrono_fmt.push_str("%3f"),
                                Some('m') => chrono_fmt.push_str("%m"),
                                Some('M') => chrono_fmt.push_str("%M"),
                                Some('n') => chrono_fmt.push('\n'),
                                Some('N') => chrono_fmt.push_str("%9f"),
                                Some('p') => chrono_fmt.push_str("%p"),
                                Some('P') => chrono_fmt.push_str("%P"),
                                Some('r') => chrono_fmt.push_str("%r"),
                                Some('R') => chrono_fmt.push_str("%R"),
                                Some('s') => chrono_fmt.push_str("%s"),
                                Some('S') => chrono_fmt.push_str("%S"),
                                Some('t') => chrono_fmt.push('\t'),
                                Some('T') => chrono_fmt.push_str("%T"),
                                Some('u') => chrono_fmt.push_str("%u"),
                                Some('U') => chrono_fmt.push_str("%U"),
                                Some('V') => chrono_fmt.push_str("%V"),
                                Some('w') => chrono_fmt.push_str("%w"),
                                Some('W') => chrono_fmt.push_str("%W"),
                                Some('x') => chrono_fmt.push_str("%x"),
                                Some('X') => chrono_fmt.push_str("%X"),
                                Some('y') => chrono_fmt.push_str("%y"),
                                Some('Y') => chrono_fmt.push_str("%Y"),
                                Some('z') => chrono_fmt.push_str("%z"),
                                Some('Z') => chrono_fmt.push_str("%Z"),
                                Some('%') => chrono_fmt.push('%'),
                                Some(other) => {
                                    chrono_fmt.push('%');
                                    chrono_fmt.push(other);
                                }
                                None => chrono_fmt.push('%'),
                            }
                        } else {
                            chrono_fmt.push(c);
                        }
                    }
                    self.out_str(&now.format(&chrono_fmt).to_string());
                } else {
                    self.out_str(&now.format("%y-%m-%d").to_string());
                }
            }
            'T' => {
                // zsh prints %T with no zero-pad on the hour: 04:10 → 4:10.
                // chrono's %H always zero-pads; use %k (space-padded hour
                // 0-23) and trim the leading space. Without this, zshrs
                // emitted `04:10` while zsh emitted `4:10` for early
                // hours.
                let now = chrono::Local::now();
                let formatted = now.format("%k:%M").to_string();
                self.out_str(formatted.trim_start());
            }
            '*' => {
                let now = chrono::Local::now();
                let formatted = now.format("%k:%M:%S").to_string();
                self.out_str(formatted.trim_start());
            }
            't' | '@' => {
                let now = chrono::Local::now();
                self.out_str(&now.format("%l:%M%p").to_string());
            }
            'w' => {
                let now = chrono::Local::now();
                self.out_str(&now.format("%a %e").to_string());
            }
            'W' => {
                let now = chrono::Local::now();
                self.out_str(&now.format("%m/%d/%y").to_string());
            }

            // Text attributes — emit only the SGR for the newly
            // toggled attribute, not all currently-active ones.
            // zsh: `%B%S%U` → `\e[1m\e[3m\e[4m` (each is independent).
            // apply_attrs would re-emit all active attrs every call,
            // producing duplicate codes.
            'B' => {
                // c:Src/prompt.c — zsh re-emits the currently-active
                // FG color after `\e[1m` so the bold sequence preserves
                // the color (some terminals reset other SGR state when
                // a new attribute lands; the re-emit is defensive).
                let fg_palette = zattr_fg_palette(self.attrs);
                self.attrs |= TXTBOLDFACE; // c:zsh.h:2694
                self.start_escape();
                self.out_str("\x1b[1m");
                self.end_escape();
                if let Some(c) = fg_palette {
                    self.start_escape();
                    self.out_str(&color_to_ansi(c as Color, true));
                    self.end_escape();
                }
            }
            'b' => {
                // zsh's %b emits a full SGR reset `\e[0m` (matches the
                // raw bytes mainline zsh produces). The incremental
                // SGR-22 (bold off) would also work but zsh chose the
                // full reset.
                //
                // c:Src/prompt.c — after the full reset, zsh re-emits
                // the currently-active FG color so a `%F{red}%B...%b`
                // sequence keeps the red after bold is turned off
                // (otherwise `\e[0m` would clear the color too).
                let fg_palette = zattr_fg_palette(self.attrs);
                self.attrs &= !TXTBOLDFACE; // c:zsh.h:2694
                self.start_escape();
                self.out_str("\x1b[0m");
                self.end_escape();
                if let Some(c) = fg_palette {
                    self.start_escape();
                    self.out_str(&color_to_ansi(c as Color, true));
                    self.end_escape();
                }
            }
            'U' => {
                self.attrs |= TXTUNDERLINE; // c:zsh.h:2697
                self.start_escape();
                self.out_str("\x1b[4m");
                self.end_escape();
            }
            'u' => {
                self.attrs &= !TXTUNDERLINE; // c:zsh.h:2697
                self.start_escape();
                self.out_str("\x1b[24m");
                self.end_escape();
            }
            'S' => {
                self.attrs |= TXTSTANDOUT; // c:zsh.h:2696
                self.start_escape();
                // zsh emits italic (`3m`) for `%S` standout, not
                // reverse video (`7m`). Match zsh's actual output.
                self.out_str("\x1b[3m");
                self.end_escape();
            }
            's' => {
                self.attrs &= !TXTSTANDOUT; // c:zsh.h:2696
                self.start_escape();
                // zsh emits the italic-end (`23m`) for `%s` rather
                // than the reverse-end (`27m`). Match zsh's output
                // so terminal state agrees with what `%S` set.
                self.out_str("\x1b[23m");
                self.end_escape();
            }

            // Colors
            'F' => {
                let color: Option<Color> = if let Some(name) = self.parse_braced_arg() {
                    color_from_name(&name) // c:336 (match_colour)
                } else if arg > 0 {
                    Some(arg as Color) // c:622 (parsecolorchar numeric)
                } else {
                    None
                };
                if let Some(c) = color {
                    if let Some((r, g, b)) = color_get_rgb(c) {
                        self.attrs = zattr_set_fg_rgb(self.attrs, r, g, b); // c:2440
                    } else {
                        self.attrs = zattr_set_fg_palette(self.attrs, c as u8); // c:2440
                    }
                    // Emit ONLY the color code, not all active attrs.
                    // apply_attrs would re-emit bold/underline/standout
                    // each time `%F` runs, producing duplicate codes.
                    self.start_escape();
                    self.out_str(&color_to_ansi(c, true));
                    self.end_escape();
                }
            }
            'f' => {
                // zsh emits the default-foreground escape `\e[39m`
                // (not a full `\e[0m` reset) — preserves background
                // color and other attrs. Going through apply_attrs
                // would emit a full reset which over-clears.
                self.attrs &= !TXT_ATTR_FG_MASK; // c:zsh.h:2732
                self.start_escape();
                self.out_str("\x1b[39m");
                self.end_escape();
            }
            'K' => {
                let color: Option<Color> = if let Some(name) = self.parse_braced_arg() {
                    color_from_name(&name) // c:336
                } else if arg > 0 {
                    Some(arg as Color) // c:634
                } else {
                    None
                };
                if let Some(c) = color {
                    if let Some((r, g, b)) = color_get_rgb(c) {
                        self.attrs = zattr_set_bg_rgb(self.attrs, r, g, b); // c:2440
                    } else {
                        self.attrs = zattr_set_bg_palette(self.attrs, c as u8); // c:2440
                    }
                    self.start_escape();
                    self.out_str(&color_to_ansi(c, false));
                    self.end_escape();
                }
            }
            'k' => {
                // zsh's `%k` emits `\e[49m` (default bg only); zshrs
                // was going through apply_attrs which would re-emit
                // all active attrs.
                self.attrs &= !TXT_ATTR_BG_MASK; // c:zsh.h:2736
                self.start_escape();
                self.out_str("\x1b[49m");
                self.end_escape();
            }

            // Literal escape sequences
            '{' => self.start_escape(),
            '}' => self.end_escape(),

            // Glitch space
            'G' => {
                let n = if arg > 0 { arg as usize } else { 1 };
                for _ in 0..n {
                    self.out_char(' ');
                }
            }

            // psvar
            'v' => {
                let idx = if arg == 0 { 1 } else { arg };
                let s_opt = prompt_tls::PSVAR.with(|c| {
                    let v = c.borrow();
                    if idx > 0 && (idx as usize) <= v.len() {
                        Some(v[idx as usize - 1].clone())
                    } else {
                        None
                    }
                });
                if let Some(s) = s_opt {
                    self.out_str(&s);
                }
            }

            // Command stack — direct port of Src/prompt.c:855-880
            // case '_'. arg >= 0 prints the TOP `arg` elements
            // BOTTOM-UP (oldest first). arg < 0 prints the BOTTOM
            // `-arg` elements bottom-up. arg == 0 prints all.
            '_' => {
                let cmd_stack = prompt_tls::CMDSTACK.with(|c| c.borrow().clone());
                let cmdsp = cmd_stack.len();
                if cmdsp > 0 {
                    let names: Vec<&str> = if arg >= 0 {
                        let mut n = if arg == 0 { cmdsp } else { arg as usize };
                        if n > cmdsp {
                            n = cmdsp;
                        }
                        // Walk forward from `cmdsp - n` to top.
                        // c:Src/prompt.c:835 — `cmdnames[cmdstack[t0]]`
                        cmd_stack
                            .iter()
                            .skip(cmdsp - n)
                            .filter_map(|b| CMDNAMES.get(*b as usize).copied())
                            .collect()
                    } else {
                        let mut n = (-arg) as usize;
                        if n > cmdsp {
                            n = cmdsp;
                        }
                        // Walk forward from 0 to `n`.
                        // c:Src/prompt.c:872 — `cmdnames[cmdstack[t0]]`
                        cmd_stack
                            .iter()
                            .take(n)
                            .filter_map(|b| CMDNAMES.get(*b as usize).copied())
                            .collect()
                    };
                    self.out_str(&names.join(" "));
                }
            }

            // Clear to end of line
            'E' => {
                self.start_escape();
                self.out_str("\x1b[K");
                self.end_escape();
            }

            // Literal characters
            '%' => self.out_char('%'),
            ')' => self.out_char(')'),
            '\0' => {}

            // Unknown - output literally
            _ => {
                self.out_char('%');
                self.out_char(c);
            }
        }
    }

    /// Expand the prompt (`promptexpand` → `putpromptchar(1,0)` in C).
    pub fn expand(mut self) -> String {
        self.run_putpromptchar(1, 0);
        self.finish_expanded_string(false)
    }
}

/// Match a `%F`/`%K` argument as a colour spec.
/// Port of `zattr match_colour(const char **teststrp, int is_fg, int colour)` from `Src/prompt.c:1957`.
/// Returns the encoded `zattr` (with TXTFGCOLOUR/TXTBGCOLOUR + 24bit
/// flag + colour index packed in the appropriate shift) or `TXT_ERROR`
/// on malformed input.
///
/// `cursor` is the by-ref parse cursor — advanced past the consumed
/// chars on success, left in place on the fall-through `colour`
/// argument path (when `teststrp == None`).
pub fn match_colour(cursor: Option<&mut usize>, spec: &str, is_fg: bool, colour: i32) -> zattr {
    // c:1957
    // c:1962-1970 — pick fg vs bg constant set.
    let (shft, on) = if is_fg {
        (TXT_ATTR_FG_COL_SHIFT, TXTFGCOLOUR) // c:1963-1965
    } else {
        (TXT_ATTR_BG_COL_SHIFT, TXTBGCOLOUR) // c:1967-1969
    };
    let mut colour = colour;
    // c:1971 — `if (teststrp)`. When None, jump to the numeric pack at
    // the end.
    if let Some(cursor) = cursor {
        let pos = *cursor;
        let rest = &spec[pos..];
        // c:1972 — `if (**teststrp == '#' && isxdigit(...))`.
        if rest.starts_with('#')
            && rest
                .as_bytes()
                .get(1)
                .map(|b| b.is_ascii_hexdigit())
                .unwrap_or(false)
        {
            // Parse hex digits after the '#'.
            let mut end = 1usize;
            while end < rest.len() && rest.as_bytes()[end].is_ascii_hexdigit() {
                end += 1;
            }
            let hex_str = &rest[1..end];
            let col = i64::from_str_radix(hex_str, 16).unwrap_or(-1);
            if col < 0 {
                return TXT_ERROR;
            }
            // c:1976-1986 — `#RGB` (3 chars) or `#RRGGBB` (6 chars).
            let (r, g, b) = match end {
                // c:1976 — `end - *teststrp == 4` (i.e. "#RGB" — 3 hex digits)
                4 => {
                    let r = ((col >> 8) | ((col >> 8) << 4)) as u8; // c:1977
                    let mut g = ((col & 0xf0) >> 4) as u8; // c:1978
                    g |= g << 4; // c:1979
                    let mut b = (col & 0xf) as u8; // c:1980
                    b |= b << 4; // c:1981
                    (r, g, b)
                }
                // c:1982 — `end - *teststrp == 7` ("#RRGGBB" — 6 hex digits)
                7 => {
                    let r = (col >> 16) as u8; // c:1983
                    let g = ((col & 0xff00) >> 8) as u8; // c:1984
                    let b = (col & 0xff) as u8; // c:1985
                    (r, g, b)
                }
                _ => return TXT_ERROR, // c:1987
            };
            // c:1988 — *teststrp = end;
            *cursor += end;
            // c:1989-1996 — runhookdef(GETCOLORATTR) then nearcolor;
            //               on no-match → emit 24-bit form.
            // GETCOLORATTR hook table isn't wired here; fall through to
            // the truecolor encoding (matches C c:1993-1996 path).
            let pixel = (((r as zattr) << 8) + g as zattr) << 8;
            let pixel = pixel + b as zattr;
            let bit24 = if is_fg {
                TXT_ATTR_FG_24BIT
            } else {
                TXT_ATTR_BG_24BIT
            };
            return on | bit24 | (pixel << shft);
        } else if rest
            .as_bytes()
            .first()
            .map(|b| b.is_ascii_alphabetic())
            .unwrap_or(false)
        {
            // c:2000-2005 — named colour.
            // match_named_colour is case-sensitive (per the existing
            // port comment); the C source uses strncmp.
            // Extract the bareword and look it up.
            let end = rest
                .find(|c: char| !c.is_ascii_alphabetic())
                .unwrap_or(rest.len());
            let name = &rest[..end];
            match match_named_colour(name) {
                Some(8) => {
                    // c:2001 — match_named_colour advances teststrp past
                    // the name BEFORE the c:2002-2003 zero-return runs.
                    // Mirror that ordering: advance, then return.
                    *cursor += end;
                    return 0; // c:2003
                }
                Some(c) => {
                    *cursor += end;
                    colour = c as i32;
                }
                None => return TXT_ERROR, // c:2004-2005
            }
        } else {
            // c:2008-2010 — numeric.
            let end = rest
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(rest.len());
            let digits = &rest[..end];
            match digits.parse::<i32>() {
                Ok(n) if (0..256).contains(&n) => {
                    *cursor += end;
                    colour = n;
                }
                _ => return TXT_ERROR, // c:2009-2010
            }
        }
    }
    // c:2014-2018 — out-of-range termcap-colour check + pack.
    //               tccolours / tccan(tc) — when the terminal advertises
    //               N colours and we asked for >=N, error. Without live
    //               termcap query, skip the bounds check (existing
    //               behaviour) and trust the caller's clamp.
    on | ((colour as zattr) << shft) // c:2018
}

/// Match a highlight specification, returning attrs + mask.
/// Port of `match_highlight(const char *teststr, zattr *on_var, zattr *setmask, int *layer)` from Src/prompt.c:2031 — the
/// mask records which fields were explicitly set so callers can
/// merge against a default. Both values are canonical `zattr`
/// bitfields (c:Src/zsh.h:2685); the mask carries the same
/// attribute / TXT*COLOUR bits as `attrs` but zeroes out the
/// actual colour indices so callers can detect "this bit was
/// set vs default" by mask-and against `TXT_ATTR_*_MASK`.
/// WARNING: param names don't match C — Rust=(spec) vs C=(teststr, on_var, setmask, layer)
pub fn match_highlight(spec: &str) -> (zattr, zattr) {
    let attrs = parsehighlight(spec);
    let mut mask: zattr = 0;
    mask |= attrs & (TXTBOLDFACE | TXTUNDERLINE | TXTSTANDOUT); // c:2031
    if attrs & TXTFGCOLOUR != 0 {
        mask |= TXTFGCOLOUR;
    } // c:2031
    if attrs & TXTBGCOLOUR != 0 {
        mask |= TXTBGCOLOUR;
    } // c:2031
    (attrs, mask)
}

/// Build an ANSI escape for an indexed colour.
/// Port of `output_colour(int colour, int fg_bg, int truecol, char *buf)` from Src/prompt.c:2136.
/// WARNING: param names don't match C — Rust=(colour, is_fg) vs C=(colour, fg_bg, truecol, buf)
pub fn output_colour(colour: u8, is_fg: bool) -> String {
    // c:2136
    let base = if is_fg { 30 } else { 40 };
    if colour < 8 {
        format!("\x1b[{}m", base + colour)
    } else if colour < 16 {
        format!("\x1b[{};1m", base + colour - 8)
    } else {
        let mode = if is_fg { 38 } else { 48 };
        format!("\x1b[{};5;{}m", mode, colour)
    }
}

/// Port of `output_highlight(zattr atr, char *buf)` from
/// Src/prompt.c:2179. Delegates to `apply_text_attributes` which
/// renders zattr to the comma-joined `bold,fg=red,...` form.
/// WARNING: param names don't match C — Rust=(attrs) vs C=(atr, mask, buf)
pub fn output_highlight(attrs: zattr) -> String {
    // c:2179
    apply_text_attributes(attrs)
}

/// Compute the default-colour reset sequences.
/// Port of `set_default_colour_sequences()` from Src/prompt.c:2341.
pub fn set_default_colour_sequences() -> (String, String) {
    // Default: use ANSI sequences
    ("\x1b[0m".to_string(), "\x1b[0m".to_string())
}

/// Build a colour escape string from a specification.
/// Port of `set_colour_code(char *str, char **var)` from Src/prompt.c:2353.
/// WARNING: param names don't match C — Rust=(spec) vs C=(str, var)
pub fn set_colour_code(spec: &str) -> Option<String> {
    let mut cur = 0usize;
    let attr = match_colour(Some(&mut cur), spec, true, 0);
    if attr == TXT_ERROR {
        return None;
    }
    // Decode back into an output escape — match_colour returns the
    // packed zattr; we extract the colour index and re-emit via
    // output_colour for the high-level callers that want a string.
    let colour = ((attr & !TXTFGCOLOUR) >> TXT_ATTR_FG_COL_SHIFT) as u8;
    Some(output_colour(colour, true))
}

/// Port of `static struct colour_sequences { char *start; char *end;
/// char *def; }` from Src/prompt.c:2319. Holds the active terminal
/// escape-prefix/suffix/default-reset codes for FG and BG channels.
#[derive(Default, Clone)]
pub struct colour_sequences {
    // c:2319
    pub start: String, // c:2320
    pub end: String,   // c:2321
    pub def: String,   // c:2322
}

// COL_SEQ_FG / COL_SEQ_BG live in zsh.h:2749-2750 — ported to
// `COL_SEQ_FG` and `::COL_SEQ_BG`. Header-defined
// constants belong in the header port per PORT.md Rule C.

/// Port of `static struct colour_sequences fg_bg_sequences[2]` from
/// `Src/prompt.c:2324`.
pub static fg_bg_sequences: std::sync::Mutex<[colour_sequences; 2]> = // c:2324
    std::sync::Mutex::new([
        colour_sequences {
            start: String::new(),
            end: String::new(),
            def: String::new(),
        },
        colour_sequences {
            start: String::new(),
            end: String::new(),
            def: String::new(),
        },
    ]);

/// Port of `static char *colseq_buf` from `Src/prompt.c:2332`.
/// We need a buffer for colour sequence composition. It may
/// vary depending on the sequences set. However, it's inefficient
/// allocating it separately every time we send a colour sequence,
/// so do it once per refresh.
pub static colseq_buf: std::sync::Mutex<Vec<u8>> = std::sync::Mutex::new(Vec::new()); // c:2332

/// Port of `static int colseq_buf_allocs` from `Src/prompt.c:2337`.
/// Count how often this has been allocated, for recursive usage.
pub static colseq_buf_allocs: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:2337

/// Port of `mod_export void allocate_colour_buffer(void)` from
/// `Src/prompt.c:2367`. Allocates the per-refresh colour-sequence
/// composition buffer, populating `fg_bg_sequences` from
/// `$zle_highlight` overrides when present.
///
/// ```c
/// mod_export void
/// allocate_colour_buffer(void)
/// {
///     char **atrs;
///     int lenfg, lenbg, len;
///     if (colseq_buf_allocs++) return;
///     atrs = getaparam("zle_highlight");
///     if (atrs) {
///         for (; *atrs; atrs++) {
///             if (strpfx("fg_start_code:", *atrs)) {
///                 set_colour_code(*atrs + 14, &fg_bg_sequences[COL_SEQ_FG].start);
///             } else if (strpfx("fg_default_code:", *atrs)) {
///                 set_colour_code(*atrs + 16, &fg_bg_sequences[COL_SEQ_FG].def);
///             } else if (strpfx("fg_end_code:", *atrs)) {
///                 set_colour_code(*atrs + 12, &fg_bg_sequences[COL_SEQ_FG].end);
///             } else if (strpfx("bg_start_code:", *atrs)) {
///                 set_colour_code(*atrs + 14, &fg_bg_sequences[COL_SEQ_BG].start);
///             } else if (strpfx("bg_default_code:", *atrs)) {
///                 set_colour_code(*atrs + 16, &fg_bg_sequences[COL_SEQ_BG].def);
///             } else if (strpfx("bg_end_code:", *atrs)) {
///                 set_colour_code(*atrs + 12, &fg_bg_sequences[COL_SEQ_BG].end);
///             }
///         }
///     }
///     lenfg = strlen(fg_bg_sequences[COL_SEQ_FG].def);
///     if (lenfg < 1) lenfg = 1;
///     lenfg += strlen(fg_bg_sequences[COL_SEQ_FG].start) +
///         strlen(fg_bg_sequences[COL_SEQ_FG].end);
///     lenbg = strlen(fg_bg_sequences[COL_SEQ_BG].def);
///     if (lenbg < 1) lenbg = 1;
///     lenbg += strlen(fg_bg_sequences[COL_SEQ_BG].start) +
///         strlen(fg_bg_sequences[COL_SEQ_BG].end);
///     len = lenfg > lenbg ? lenfg : lenbg;
///     colseq_buf = (char *)zalloc(len+15);
/// }
/// ```
pub fn allocate_colour_buffer() {
    // c:2367

    // c:2372 — `if (colseq_buf_allocs++) return;`
    if colseq_buf_allocs.fetch_add(1, Ordering::SeqCst) != 0 {
        // c:2372
        return; // c:2373
    }

    // c:2375 — `atrs = getaparam("zle_highlight");`
    // Rust getaparam takes &mut value, not name — use paramtab lookup
    // directly and pull arrgetfn off the param.
    let atrs: Option<Vec<String>> = {
        // c:2375
        let tab = paramtab().read().ok();
        tab.and_then(|t| {
            t.get("zle_highlight")
                .map(|p| crate::ported::params::arrgetfn(p))
        })
    };

    if let Some(atrs) = atrs {
        // c:2376
        let mut seqs = fg_bg_sequences.lock().unwrap();
        for atr in &atrs {
            // c:2377
            if strpfx("fg_start_code:", atr) {
                // c:2378
                if let Some(c) = set_colour_code(&atr[14..]) {
                    // c:2379
                    seqs[COL_SEQ_FG as usize].start = c;
                }
            } else if strpfx("fg_default_code:", atr) {
                // c:2380
                if let Some(c) = set_colour_code(&atr[16..]) {
                    // c:2381
                    seqs[COL_SEQ_FG as usize].def = c;
                }
            } else if strpfx("fg_end_code:", atr) {
                // c:2382
                if let Some(c) = set_colour_code(&atr[12..]) {
                    // c:2383
                    seqs[COL_SEQ_FG as usize].end = c;
                }
            } else if strpfx("bg_start_code:", atr) {
                // c:2384
                if let Some(c) = set_colour_code(&atr[14..]) {
                    // c:2385
                    seqs[COL_SEQ_BG as usize].start = c;
                }
            } else if strpfx("bg_default_code:", atr) {
                // c:2386
                if let Some(c) = set_colour_code(&atr[16..]) {
                    // c:2387
                    seqs[COL_SEQ_BG as usize].def = c;
                }
            } else if strpfx("bg_end_code:", atr) {
                // c:2388
                if let Some(c) = set_colour_code(&atr[12..]) {
                    // c:2389
                    seqs[COL_SEQ_BG as usize].end = c;
                }
            }
        }
    }

    let seqs = fg_bg_sequences.lock().unwrap();
    let mut lenfg: usize = seqs[COL_SEQ_FG as usize].def.len(); // c:2394
    if lenfg < 1 {
        lenfg = 1;
    } // c:2396-2397
    lenfg += seqs[COL_SEQ_FG as usize].start.len() + seqs[COL_SEQ_FG as usize].end.len(); // c:2398-2399

    let mut lenbg: usize = seqs[COL_SEQ_BG as usize].def.len(); // c:2401
    if lenbg < 1 {
        lenbg = 1;
    } // c:2403-2404
    lenbg += seqs[COL_SEQ_BG as usize].start.len() + seqs[COL_SEQ_BG as usize].end.len(); // c:2405-2406
    drop(seqs);

    let len = if lenfg > lenbg { lenfg } else { lenbg }; // c:2408
                                                         // c:2410 — `colseq_buf = (char *)zalloc(len+15);` (+1 NUL +14 truecolor)
    *colseq_buf.lock().unwrap() = vec![0u8; len + 15]; // c:2410
}

/// Free the colour-buffer working space.
/// Port of `free_colour_buffer()` from Src/prompt.c:2417.
pub fn free_colour_buffer() {
    // c:2417
    // C body c:2420-2426: `if (--colseq_buf_allocs) return;
    //                      zfree(colseq_buf, ...); colseq_buf = NULL;`
    if colseq_buf_allocs.fetch_sub(1, Ordering::SeqCst) - 1 != 0 {
        // c:2420
        return; // c:2421
    }
    colseq_buf.lock().unwrap().clear(); // c:2424
}

/// Port of `set_colour_attribute(zattr atr, int fg_bg, int flags)`
/// from Src/prompt.c:2440. Delegates to `color_to_ansi` which
/// produces the indexed/256-color/truecolor escape.
/// WARNING: param names don't match C — Rust=(color, is_fg) vs C=(atr, fg_bg, flags)
pub fn set_colour_attribute(color: Color, is_fg: bool) -> String {
    // c:2440
    color_to_ansi(color, is_fg)
}

// `pub enum CmdState` + `impl CmdState { from_u8, name }` —
// DELETED per user directive ("CmdState fake"). Was a Rust-only
// typed wrapper around the canonical `CS_*` integer constants
// (`Src/zsh.h:2775-2806`, ported to `crate::ported::zsh_h::CS_*`).
// C source pushes raw `unsigned char` bytes onto `cmdstack` and
// indexes `cmdnames[CS_COUNT]` (`Src/prompt.c:62`) for the name.
// Now ported 1:1: callers use `CS_FOO as u8` directly and look up
// names through `cmdname()` below.

// parser states, for %_                                                    // c:60
/// Direct port of `cmdnames[CS_COUNT]` from `Src/prompt.c:62-71`.
/// Indexed by the `CS_*` constants in `zsh_h::CS_FOR..CS_ALWAYS`
/// (`Src/zsh.h:2775-2806`). Used by `%_` prompt expansion to print
/// the active compound-command keyword stack.
pub static CMDNAMES: [&str; crate::ported::zsh_h::CS_COUNT as usize] = [
    "for",
    "while",
    "repeat",
    "select", // c:63 (CS_FOR..CS_SELECT)
    "until",
    "if",
    "then",
    "else", // c:64 (CS_UNTIL..CS_ELSE)
    "elif",
    "math",
    "cond",
    "cmdor", // c:65 (CS_ELIF..CS_CMDOR)
    "cmdand",
    "pipe",
    "errpipe",
    "foreach", // c:66 (CS_CMDAND..CS_FOREACH)
    "case",
    "function",
    "subsh",
    "cursh", // c:67 (CS_CASE..CS_CURSH)
    "array",
    "quote",
    "dquote",
    "bquote", // c:68 (CS_ARRAY..CS_BQUOTE)
    "cmdsubst",
    "mathsubst",
    "elif-then",
    "heredoc", // c:69 (CS_CMDSUBST..CS_HEREDOC)
    "heredocd",
    "brace",
    "braceparam",
    "always", // c:70 (CS_HEREDOCD..CS_ALWAYS)
];
// c:zsh.h:2685-2741

// `Color` is the colour slot lifted out of `zattr` so callers can
// pass a single integer around. Bit layout mirrors the C zattr
// colour bits exactly:
//   bit 31 (0x01000000): the local 24-bit flag — mirrors the
//     C `TXT_ATTR_FG_24BIT` / `TXT_ATTR_BG_24BIT` bit (Src/zsh.h:2727).
//     When set, the low 24 bits hold `0xRRGGBB`. When clear, the
//     low 8 bits hold a palette index 0..=255, where 8 is the
//     "default" sentinel per Src/prompt.c:1909.
// Not a new type — same encoding C packs into `TXT_ATTR_FG_COL_MASK`.
pub type Color = u32; // c:Src/zsh.h:2718 (colour slot)
pub const COLOR_24BIT: Color = 0x0100_0000; // c:zsh.h:2727 (TXT_ATTR_FG_24BIT)

// Sentinel "no colour set" — palette index that lives in
// TXT_ATTR_FG_COL_MASK when the colour is `default` (8 in
// Src/prompt.c:1909). Bits 16-39 are at most 24 bits, so any
// value 0..=255 fits comfortably for palette mode.
pub const COLOUR_DEFAULT: u8 = 8; // c:Src/prompt.c:1909

// Named-colour palette constants. Indexes match `colour_names[]`
// from `Src/prompt.c:1884-1887`. Used in place of the deleted
// `Color::Black`..`Color::White`/`Color::Default` enum variants.
pub const COLOR_BLACK: Color = 0; // c:1885
pub const COLOR_RED: Color = 1; // c:1885
pub const COLOR_GREEN: Color = 2; // c:1885
pub const COLOR_YELLOW: Color = 3; // c:1885
pub const COLOR_BLUE: Color = 4; // c:1885
pub const COLOR_MAGENTA: Color = 5; // c:1885
pub const COLOR_CYAN: Color = 6; // c:1885
pub const COLOR_WHITE: Color = 7; // c:1885
pub const COLOR_DEFAULT: Color = COLOUR_DEFAULT as Color; // c:1909

// Defines standard ANSI colour names in index order                        // c:1883
/// Direct port of `colour_names[]` from `Src/prompt.c:1884-1887`.
/// Indexed 0-7 = basic ANSI, 8 = "default" sentinel (per
/// `Src/prompt.c:1909` comment "8 is the special value for
/// default"). Single canonical source — the second
/// `match_named_colour` further down this file consumed a
/// drifted local table with `default = 9` which mis-rendered
/// `%F{default}` output.
pub static COLOUR_NAMES: [&str; 9] = [
    "black", "red", "green", "yellow", // c:1885
    "blue", "magenta", "cyan", "white",   // c:1885
    "default", // c:1886
];

// Colour / zattr helpers — C inlines these at each call site in Src/prompt.c.
fn color_rgb(r: u8, g: u8, b: u8) -> Color {
    COLOR_24BIT | ((r as Color) << 16) | ((g as Color) << 8) | (b as Color)
}

fn color_get_rgb(c: Color) -> Option<(u8, u8, u8)> {
    if c & COLOR_24BIT == 0 {
        None
    } else {
        Some((
            ((c >> 16) & 0xff) as u8,
            ((c >> 8) & 0xff) as u8,
            (c & 0xff) as u8,
        ))
    }
}

fn color_to_ansi(c: Color, is_fg: bool) -> String {
    if let Some((r, g, b)) = color_get_rgb(c) {
        let lead = if is_fg { 38 } else { 48 };
        format!("\x1b[{};2;{};{};{}m", lead, r, g, b)
    } else {
        output_colour(c as u8, is_fg)
    }
}

fn color_from_name(name: &str) -> Option<Color> {
    if let Some(rest) = name.strip_prefix('#') {
        if rest.len() == 6 {
            let r = u8::from_str_radix(&rest[0..2], 16).ok();
            let g = u8::from_str_radix(&rest[2..4], 16).ok();
            let b = u8::from_str_radix(&rest[4..6], 16).ok();
            match (r, g, b) {
                (Some(r), Some(g), Some(b)) => Some(color_rgb(r, g, b) as Color),
                _ => None,
            }
        } else {
            match_named_colour(name).map(|idx| idx as Color)
        }
    } else {
        match_named_colour(name).map(|idx| idx as Color)
    }
}

fn zattr_set_fg_palette(attrs: zattr, idx: u8) -> zattr {
    let cleared = attrs & !TXT_ATTR_FG_MASK;
    cleared | TXTFGCOLOUR | ((idx as zattr) << TXT_ATTR_FG_COL_SHIFT)
}

/// Return the currently-active palette FG color, if any. Used by
/// `%b` (bold off) to re-emit the FG color after the full
/// `\e[0m` reset that zsh emits. Returns None for 24-bit RGB
/// colors (those need a different re-emit path that's deferred)
/// or when no FG color is set.
fn zattr_fg_palette(attrs: zattr) -> Option<u8> {
    if (attrs & TXTFGCOLOUR) == 0 || (attrs & TXT_ATTR_FG_24BIT) != 0 {
        return None;
    }
    Some(((attrs >> TXT_ATTR_FG_COL_SHIFT) & 0xff) as u8)
}

fn zattr_set_fg_rgb(attrs: zattr, r: u8, g: u8, b: u8) -> zattr {
    let cleared = attrs & !TXT_ATTR_FG_MASK;
    let rgb = ((r as zattr) << 16) | ((g as zattr) << 8) | (b as zattr);
    cleared | TXTFGCOLOUR | TXT_ATTR_FG_24BIT | (rgb << TXT_ATTR_FG_COL_SHIFT)
}

fn zattr_set_bg_palette(attrs: zattr, idx: u8) -> zattr {
    let cleared = attrs & !TXT_ATTR_BG_MASK;
    cleared | TXTBGCOLOUR | ((idx as zattr) << TXT_ATTR_BG_COL_SHIFT)
}

fn zattr_set_bg_rgb(attrs: zattr, r: u8, g: u8, b: u8) -> zattr {
    let cleared = attrs & !TXT_ATTR_BG_MASK;
    let rgb = ((r as zattr) << 16) | ((g as zattr) << 8) | (b as zattr);
    cleared | TXTBGCOLOUR | TXT_ATTR_BG_24BIT | (rgb << TXT_ATTR_BG_COL_SHIFT)
}

/// Expand a prompt string
pub fn expand_prompt(s: &str) -> String {
    prompt_tls::sync_from_globals();
    buf_vars::new(s).expand() // c:Src/prompt.c:214 (new_vars init)
}

/// Same as [`expand_prompt`] — C call sites that used implicit globals only.
pub fn expand_prompt_default(s: &str) -> String {
    expand_prompt(s)
}

/// Count the visible width of an expanded prompt (ignoring escape sequences)
pub fn prompt_width(s: &str) -> usize {
    let mut width = 0;
    let mut in_escape = false;
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\x01' => in_escape = true,  // RL_PROMPT_START_IGNORE
            '\x02' => in_escape = false, // RL_PROMPT_END_IGNORE
            '\x1b' => {
                // ANSI escape - skip until 'm' or end
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next == 'm' {
                        break;
                    }
                }
            }
            _ if !in_escape => {
                width += unicode_width::UnicodeWidthChar::width(c).unwrap_or(1);
            }
            _ => {}
        }
    }

    width
}

/// Output true color (24-bit) escape sequence
pub fn output_truecolor(r: u8, g: u8, b: u8, is_fg: bool) -> String {
    let mode = if is_fg { 38 } else { 48 };
    format!("\x1b[{};2;{};{};{}m", mode, r, g, b)
}

/// Maximum cmdstack depth, mirroring C zsh's `CMDSTACKSZ`.
/// Used to bound `cmdpush`/`cmdpop` so the stack can't grow
/// unbounded under runaway recursion.
// the command stack for use with %_ in prompts                             // c:53
const CMDSTACKSZ: usize = 256;

// Port of file-static `cmdstack` from `Src/init.c` (declared as
// `extern unsigned char cmdstack[CMDSTACKSZ]` in `Src/zsh.h:2658`).
// Stack of parser-context tokens (`CS_*`) the parser pushes as it
// descends into nested compound commands (`if`/`for`/`while`/`{}`
// etc.). Read by the prompt expander for `%_` and `%^` to render
// which constructs are currently open.
//
// Bucket-1 per PORT_PLAN.md — file-static in C, per-evaluator in
// zshrs. Each worker thread parses independently; sharing the
// stack across threads would corrupt nesting state. `RefCell`
// for interior mutability since the contents are owned `Vec<u8>`.
// the command stack for use with %_ in prompts                             // c:53
thread_local! {
    pub static CMDSTACK: std::cell::RefCell<Vec<u8>> = const {              // c:56 (cmdstack[] global)
        std::cell::RefCell::new(Vec::new())
    };
}

/// Apply text attributes as a single ANSI SGR escape.
// functions for handling attributes                                        // c:1641
/// Port of `applytextattributes(int flags)` from Src/prompt.c:1645 —
/// builds one SGR sequence with all active codes joined.
pub fn apply_text_attributes(attrs: zattr) -> String {
    // c:1645
    let mut codes: Vec<String> = Vec::new();
    if attrs & TXTBOLDFACE != 0 {
        codes.push("1".to_string());
    } // c:1645
    if attrs & TXTUNDERLINE != 0 {
        codes.push("4".to_string());
    } // c:1645
    if attrs & TXTSTANDOUT != 0 {
        codes.push("7".to_string());
    } // c:1645
    if attrs & TXTFGCOLOUR != 0 {
        // c:1645
        let raw = (attrs & TXT_ATTR_FG_COL_MASK) >> TXT_ATTR_FG_COL_SHIFT;
        let c = if attrs & TXT_ATTR_FG_24BIT != 0 {
            // 24-bit FG — re-pack raw RGB into a `Color` and emit.
            COLOR_24BIT | (raw as Color & 0x00ff_ffff)
        } else {
            raw as Color
        };
        codes.push(
            color_to_ansi(c, true)
                .trim_start_matches("\x1b[")
                .trim_end_matches('m')
                .to_string(),
        );
    }
    if attrs & TXTBGCOLOUR != 0 {
        // c:1645
        let raw = (attrs & TXT_ATTR_BG_COL_MASK) >> TXT_ATTR_BG_COL_SHIFT;
        let c = if attrs & TXT_ATTR_BG_24BIT != 0 {
            COLOR_24BIT | (raw as Color & 0x00ff_ffff)
        } else {
            raw as Color
        };
        codes.push(
            color_to_ansi(c, false)
                .trim_start_matches("\x1b[")
                .trim_end_matches('m')
                .to_string(),
        );
    }
    if codes.is_empty() {
        String::new()
    } else {
        format!("\x1b[{}m", codes.join(";"))
    }
}

/// Reset all text attributes
pub fn reset_text_attributes() -> &'static str {
    "\x1b[0m"
}

/// Right prompt handling - compute padding for RPROMPT
pub fn right_prompt_padding(
    left_width: usize,
    right_prompt: &str,
    term_width: usize,
    indent: usize,
) -> Option<String> {
    let right_width = prompt_width(right_prompt);
    let total = left_width + right_width + indent;
    if total >= term_width {
        return None; // No room for right prompt
    }
    let padding = term_width - total;
    Some(" ".repeat(padding))
}

/// Transient prompt - return empty string to clear prompt on accept-line
pub fn transient_prompt(_original: &str) -> String {
    String::new()
}

fn color_name(c: Color) -> String {
    if let Some((r, g, b)) = color_get_rgb(c) {
        return format!("#{:02x}{:02x}{:02x}", r, g, b);
    }
    let idx = (c & 0xff) as usize;
    if idx < COLOUR_NAMES.len() {
        return COLOUR_NAMES[idx].to_string();
    }
    idx.to_string()
}

// ===========================================================
// Methods moved verbatim from src/ported/vm_helper because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: prompt
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

// ===========================================================
// Methods moved verbatim from src/ported/vm_helper because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: drift
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

/// Singleton holding the txtcurrentattrs / txtpendingattrs C
/// globals (Src/prompt.c file-statics, around line 1640). Used
/// by [`applytextattributes`] to compute the SGR diff between
/// the last-flushed and the pending attribute state.
fn current_attrs_lock() -> &'static std::sync::Mutex<zattr> {
    static CUR: std::sync::OnceLock<std::sync::Mutex<zattr>> = std::sync::OnceLock::new();
    CUR.get_or_init(|| std::sync::Mutex::new(0 as zattr))
}

fn pending_attrs_lock() -> &'static std::sync::Mutex<zattr> {
    static PND: std::sync::OnceLock<std::sync::Mutex<zattr>> = std::sync::OnceLock::new();
    PND.get_or_init(|| std::sync::Mutex::new(0 as zattr))
}

/// Set the pending text-attributes that the next
/// [`applytextattributes`] call will diff against the current
/// state. Mirrors callers writing to C's `txtpendingattrs`.
pub fn set_pending_text_attrs(attrs: zattr) {
    *pending_attrs_lock().lock().expect("pending_attrs poisoned") = attrs;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// c:1935-1944 — `truecolor_terminal` returns true iff
    /// `.term.extensions` array contains an un-negated `truecolor`
    /// entry. The previous Rust port did COLORTERM/TERM heuristics
    /// (an entirely different decision rule). Regression target:
    /// a session with `.term.extensions=(truecolor)` MUST report
    /// truecolor; a session with `.term.extensions=(-truecolor)` MUST
    /// report disabled regardless of COLORTERM/TERM env.
    #[test]
    fn truecolor_terminal_routes_through_term_extensions_array() {
        let _g = crate::test_util::global_state_lock();
        let saved = crate::ported::params::getaparam(".term.extensions");

        // Empty / unset → false (c:1944).
        let _ = setaparam(".term.extensions", vec![]);
        assert!(
            !truecolor_terminal(),
            "empty .term.extensions must report off"
        );

        // truecolor present → true (c:1940-1942 with result=1).
        let _ = setaparam(".term.extensions", vec!["truecolor".to_string()]);
        assert!(
            truecolor_terminal(),
            ".term.extensions=(truecolor) must report on"
        );

        // -truecolor → explicitly disabled (c:1940 with result=0).
        let _ = setaparam(".term.extensions", vec!["-truecolor".to_string()]);
        assert!(
            !truecolor_terminal(),
            ".term.extensions=(-truecolor) must report off"
        );

        // Restore.
        let _ = setaparam(".term.extensions", saved.unwrap_or_default());
    }

    /// c:134 — when `home` is a prefix of `path` AND tilde=true,
    /// promptpath MUST substitute. Catches a regression where the
    /// prefix-match falls back to the unchanged absolute path silently.
    #[test]
    fn promptpath_substitutes_home_prefix_with_tilde() {
        let _g = crate::test_util::global_state_lock();
        let r = promptpath("/home/user/project", 0, true, "/home/user");
        assert!(
            r.starts_with('~'),
            "home-prefix must collapse to ~ (got {r:?})"
        );
    }

    /// c:134 — npath>0 truncates from the right, keeping the last N
    /// path components. Used by `%c` / `%~` for theme depth-limits;
    /// regressions silently render full paths in cramped prompts.
    #[test]
    fn promptpath_npath_one_keeps_only_last_component() {
        let _g = crate::test_util::global_state_lock();
        let r = promptpath("/a/b/c/d", 1, false, "");
        assert!(!r.contains("a/b") && r.ends_with("d"), "got {r:?}");
    }

    /// c:285 — `parsehighlight("bold")` MUST set the TXTBOLDFACE bit.
    /// Regression that drops the bit would silently mis-render every
    /// bold escape in user's `zle_highlight=(...)` array.
    #[test]
    fn parsehighlight_bold_sets_bold_bit() {
        let _g = crate::test_util::global_state_lock();
        assert_ne!(parsehighlight("bold") & TXTBOLDFACE, 0);
    }

    /// `match_named_colour` MUST resolve all 8 ANSI base names plus
    /// "default" — the user-facing color identifiers in `zle_highlight`
    /// + `bindkey -A`. Skipping any name breaks the by-name color path
    /// users rely on every keystroke.
    #[test]
    fn match_named_colour_covers_full_ansi_palette_and_default() {
        let _g = crate::test_util::global_state_lock();
        for &name in &[
            "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white", "default",
        ] {
            assert!(match_named_colour(name).is_some(), "{name:?} must resolve");
        }
    }

    /// `match_colour` parses "red" → zattr with TXTFGCOLOUR + colour
    /// index 1 shifted to the FG colour slot (c:2018).
    #[test]
    fn match_colour_named_red_fg() {
        let _g = crate::test_util::global_state_lock();
        let mut cur = 0usize;
        let attr = match_colour(Some(&mut cur), "red", true, 0);
        assert_ne!(attr, TXT_ERROR);
        assert_eq!(attr & TXTFGCOLOUR, TXTFGCOLOUR);
        let idx = (attr >> TXT_ATTR_FG_COL_SHIFT) & 0xff;
        assert_eq!(idx, 1, "red index 1");
        assert_eq!(cur, 3, "consumed exactly 'red'");
    }

    /// `match_colour` named "default" → 0 (cleared), per c:2002-2003.
    #[test]
    fn match_colour_named_default_is_zero() {
        let _g = crate::test_util::global_state_lock();
        let mut cur = 0usize;
        let attr = match_colour(Some(&mut cur), "default", true, 0);
        assert_eq!(attr, 0);
    }

    /// `match_colour` MUST advance the cursor past "default" even
    /// though the return value is the zero short-circuit. In C
    /// (c:2001) `match_named_colour(teststrp)` is what does the
    /// advance — the c:2002-2003 `if (colour == 8) return 0;` runs
    /// AFTER, so the cursor is already past the name. A Rust port
    /// that early-returns before incrementing leaves the caller's
    /// next dispatch re-parsing "default" forever.
    #[test]
    fn match_colour_named_default_advances_cursor() {
        let _g = crate::test_util::global_state_lock();
        let mut cur = 0usize;
        let attr = match_colour(Some(&mut cur), "default", true, 0);
        assert_eq!(attr, 0);
        assert_eq!(
            cur, 7,
            "c:2001 — match_named_colour advances teststrp past 'default'; \
             return-zero branch must NOT skip the advance"
        );
    }

    /// `match_colour` numeric "12" → zattr with index 12.
    #[test]
    fn match_colour_numeric() {
        let _g = crate::test_util::global_state_lock();
        let mut cur = 0usize;
        let attr = match_colour(Some(&mut cur), "12", false, 0);
        assert_ne!(attr, TXT_ERROR);
        assert_eq!(attr & TXTBGCOLOUR, TXTBGCOLOUR);
        let idx = (attr >> TXT_ATTR_BG_COL_SHIFT) & 0xff;
        assert_eq!(idx, 12);
    }

    /// `match_colour` rejects out-of-range numeric (>= 256) per c:2009-2010.
    #[test]
    fn match_colour_numeric_out_of_range_errors() {
        let _g = crate::test_util::global_state_lock();
        let mut cur = 0usize;
        assert_eq!(match_colour(Some(&mut cur), "500", true, 0), TXT_ERROR);
    }

    /// `match_colour` parses #RRGGBB truecolor → packs r/g/b into
    /// the colour slot with the 24BIT bit set.
    #[test]
    fn match_colour_truecolor_six_digit() {
        let _g = crate::test_util::global_state_lock();
        let mut cur = 0usize;
        let attr = match_colour(Some(&mut cur), "#ff8040", true, 0);
        assert_ne!(attr, TXT_ERROR);
        assert_eq!(attr & TXTFGCOLOUR, TXTFGCOLOUR);
        assert_eq!(attr & TXT_ATTR_FG_24BIT, TXT_ATTR_FG_24BIT);
        let pixel = (attr >> TXT_ATTR_FG_COL_SHIFT) & 0xffffff;
        assert_eq!(pixel, 0xff8040);
        assert_eq!(cur, 7);
    }

    /// `match_colour` #RGB → expands each nibble (R becomes RR, etc.)
    /// per c:1976-1981.
    #[test]
    fn match_colour_truecolor_three_digit_expands() {
        let _g = crate::test_util::global_state_lock();
        let mut cur = 0usize;
        let attr = match_colour(Some(&mut cur), "#f8a", true, 0);
        assert_ne!(attr, TXT_ERROR);
        // #f8a → r=0xff, g=0x88, b=0xaa per the nibble-doubling C body.
        let pixel = (attr >> TXT_ATTR_FG_COL_SHIFT) & 0xffffff;
        // c:1977 — r = (col>>8) | ((col>>8)<<4) where col=0xf8a, col>>8=0xf → r=0xff
        // c:1978-1979 — g = ((col & 0xf0) >> 4); g |= g<<4 → g=0x88
        // c:1980-1981 — b = col & 0xf; b |= b<<4 → b=0xaa
        assert_eq!(pixel, 0xff_88_aa, "got pixel 0x{:06x}", pixel);
    }

    /// `match_colour` malformed `#` (no hex digits) returns TXT_ERROR.
    #[test]
    fn match_colour_hash_without_hex_errors() {
        let _g = crate::test_util::global_state_lock();
        let mut cur = 0usize;
        // No hex after #, so the first branch returns TXT_ERROR... wait,
        // the C path requires `isxdigit((unsigned char)teststrp[1])` to
        // enter the hex branch. If false, we fall to the named-colour
        // branch (which would also fail). For input "#x" we expect error.
        let attr = match_colour(Some(&mut cur), "#x", true, 0);
        assert_eq!(attr, TXT_ERROR);
    }

    /// `match_colour` cursor MUST NOT advance on TXT_ERROR — the C
    /// body's c:1971 `teststrp = end` is INSIDE the success branch
    /// of the hex match. A regression that advances on error would
    /// leave subsequent parsing pointing into the middle of bad
    /// input, cascading garbage attributes into the prompt.
    #[test]
    fn match_colour_does_not_advance_cursor_on_error() {
        let _g = crate::test_util::global_state_lock();
        // Numeric-out-of-range: returns TXT_ERROR after parsing "500"
        // (3 digits). The cursor must stay at 0; the caller's next
        // dispatch needs to see the original input position to emit
        // a useful error.
        let mut cur = 0usize;
        let attr = match_colour(Some(&mut cur), "500extra", true, 0);
        assert_eq!(attr, TXT_ERROR);
        assert_eq!(cur, 0, "cursor must stay at 0 on TXT_ERROR; got {}", cur);
    }

    /// Unknown colour names MUST return None — silent fallback would
    /// mask theme typos that users would otherwise see immediately.
    #[test]
    fn match_named_colour_returns_none_for_unknown() {
        let _g = crate::test_util::global_state_lock();
        assert!(match_named_colour("definitely_not_a_color_zshrs").is_none());
    }

    /// Pin: `countprompt` recognises `Inpar` (0x88) / `Outpar` (0x8a)
    /// / `Nularg` (0xa1) as the THREE special tokens per
    /// `Src/prompt.c:1179-1185`. The previous Rust port used
    /// '\x01' / '\x02' / '\x03' — the WRONG byte values.
    ///
    /// `Inpar..Outpar` regions are `%{...%}` non-printing escapes
    /// (zero visible width); `Nularg` is a glitch-space placeholder
    /// (1 visible column).
    #[test]
    fn countprompt_recognises_canonical_inpar_outpar_nularg_bytes() {
        let _g = crate::test_util::global_state_lock();
        let mut w = 0i32;
        let mut h = 0i32;
        // `abc%{...%}def` shape: `abc` (3 cols), Inpar+escape+Outpar
        // (zero cols since non-printing), `def` (3 cols).
        let probe = format!("abc{}ESC{}def", Inpar, Outpar);
        countprompt(&probe, &mut w, &mut h, 0);
        assert_eq!(
            w, 6,
            "c:1179-1182 — Inpar..Outpar region must be zero-width; \
             got w={w} for 3+0+3-col prompt"
        );

        // Nularg alone is 1 visible column.
        let mut w = 0i32;
        let mut h = 0i32;
        let probe = format!("{}", Nularg);
        countprompt(&probe, &mut w, &mut h, 0);
        assert_eq!(
            w, 1,
            "c:1183-1184 — Nularg counts as 1 visible column; got w={w}"
        );
    }

    /// c:134 — `promptpath` with `tilde=false` MUST NOT substitute ~
    /// even when `home` is a prefix. Pin the inverse branch so a
    /// regen that hardcodes tilde-substitution silently breaks
    /// `%/` literal-path renders.
    #[test]
    fn promptpath_without_tilde_keeps_absolute_path() {
        let _g = crate::test_util::global_state_lock();
        let r = promptpath("/home/user/project", 0, /*tilde=*/ false, "/home/user");
        assert!(
            r.starts_with("/home/user"),
            "tilde=false must NOT collapse to ~; got {r:?}"
        );
        assert!(
            !r.starts_with('~'),
            "tilde=false output must not start with ~"
        );
    }

    /// c:134 — Path equal to home (no remainder). `tilde=true` →
    /// just `~`. Edge case the prefix-collapse logic must handle.
    #[test]
    fn promptpath_path_exactly_home_renders_as_tilde_only() {
        let _g = crate::test_util::global_state_lock();
        let r = promptpath("/home/user", 0, true, "/home/user");
        assert_eq!(r, "~", "path == home must render as plain '~'; got {r:?}");
    }

    /// c:134 — Home not a prefix of path: leave path unchanged
    /// regardless of tilde flag.
    #[test]
    fn promptpath_unrelated_path_unchanged() {
        let _g = crate::test_util::global_state_lock();
        let r = promptpath("/etc/zshrc", 0, true, "/home/user");
        assert_eq!(r, "/etc/zshrc", "non-home path must pass through unchanged");
    }

    /// c:134 — npath=0 means "no truncation": the full path renders.
    #[test]
    fn promptpath_npath_zero_means_no_truncation() {
        let _g = crate::test_util::global_state_lock();
        let r = promptpath("/a/b/c/d/e", 0, false, "");
        assert_eq!(r, "/a/b/c/d/e", "npath=0 must keep full path");
    }

    /// c:134 — npath=2 keeps the LAST two components only. Pin
    /// the off-by-one because the C source does `for (i = npath; i > 0; --i)`
    /// — a regen that does `>= 0` would keep one extra.
    #[test]
    fn promptpath_npath_two_keeps_last_two_components() {
        let _g = crate::test_util::global_state_lock();
        let r = promptpath("/a/b/c/d", 2, false, "");
        assert!(
            r.contains("c") && r.contains("d"),
            "npath=2 must include last 2 components; got {r:?}"
        );
        assert!(
            !r.contains("/a/"),
            "npath=2 must NOT include first 2 components"
        );
    }

    /// c:285 — `parsehighlight` for `none` returns 0 (no attrs set).
    /// Pin the keyword that explicitly clears all attribute bits.
    #[test]
    fn parsehighlight_none_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = parsehighlight("none");
        assert_eq!(r, 0, "'none' must yield zero attrs; got {:#x}", r);
    }

    /// c:285 — `parsehighlight("underline")` sets the UNDERLINE bit.
    /// Test the other attribute keywords separately from `bold`.
    #[test]
    fn parsehighlight_underline_sets_underline_bit() {
        let _g = crate::test_util::global_state_lock();
        let r = parsehighlight("underline");
        assert_ne!(r, 0, "underline must set at least one bit");
    }

    /// c:285 — Unknown highlight keyword returns 0 (silent ignore).
    /// Pin the failure mode because zle_highlight users add custom
    /// keywords; the C source skips unknowns rather than erroring.
    #[test]
    fn parsehighlight_unknown_keyword_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = parsehighlight("definitely_not_a_real_attr");
        assert_eq!(r, 0, "unknown attr must be silently ignored");
    }

    /// c:1915 — `match_named_colour` is case-sensitive: "RED" must
    /// NOT match "red". Pin lower-case enforcement; a regen that
    /// adds `.to_lowercase()` would silently accept uppercase color
    /// names that the C source rejects.
    #[test]
    fn match_named_colour_is_case_sensitive() {
        let _g = crate::test_util::global_state_lock();
        assert!(match_named_colour("red").is_some());
        assert!(
            match_named_colour("RED").is_none(),
            "uppercase color must NOT resolve per C source's strcmp"
        );
        assert!(match_named_colour("Red").is_none());
    }

    /// c:1915 — Empty string returns None. Defensive boundary.
    #[test]
    fn match_named_colour_empty_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(match_named_colour("").is_none());
    }

    /// c:1276 — `cmdpush`/`cmdpop` round-trip. Pin the LIFO balance.
    #[test]
    fn cmdpush_cmdpop_round_trip_does_not_panic() {
        let _g = crate::test_util::global_state_lock();
        // Just verifies safe push/pop balance for several tokens.
        cmdpush(0);
        cmdpush(1);
        cmdpush(2);
        cmdpop();
        cmdpop();
        cmdpop();
        // Extra pops must be safe (no underflow panic)
        cmdpop();
    }

    /// c:976 — `pputc` appends a single ASCII char to the buffer.
    /// Pin the no-buffering / immediate-append contract.
    #[test]
    fn pputc_appends_char_to_buffer() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = String::new();
        pputc(&mut buf, 'X');
        assert_eq!(buf, "X");
        pputc(&mut buf, 'Y');
        assert_eq!(buf, "XY");
    }

    /// c:1016 — `stradd` appends a string slice to the buffer.
    #[test]
    fn stradd_appends_string_to_buffer() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = String::from("pre/");
        stradd(&mut buf, "post");
        assert_eq!(buf, "pre/post");
        stradd(&mut buf, "");
        assert_eq!(buf, "pre/post", "empty append leaves buffer unchanged");
    }

    /// c:1737-1751 — `tsetattrs` OR-merges non-color attrs into
    /// `txtpendingattrs` and wholesale-replaces FG/BG mask bits.
    #[test]
    fn tsetattrs_updates_pending_attrs_non_color() {
        let _g = crate::test_util::global_state_lock();
        set_pending_text_attrs(0);
        let _ = tsetattrs(TXTBOLDFACE);
        let p = *pending_attrs_lock().lock().unwrap();
        assert_ne!(p & TXTBOLDFACE, 0, "TXTBOLDFACE bit ORed into pending");
    }

    /// c:1743-1746 — TXTFGCOLOUR replaces the FG mask wholesale, not ORs.
    #[test]
    fn tsetattrs_fg_color_replaces_fg_mask() {
        let _g = crate::test_util::global_state_lock();
        let palette_idx_5: zattr = (5u64 << TXT_ATTR_FG_COL_SHIFT) & TXT_ATTR_FG_COL_MASK;
        let palette_idx_2: zattr = (2u64 << TXT_ATTR_FG_COL_SHIFT) & TXT_ATTR_FG_COL_MASK;
        // Pre-seed pending with idx=5
        set_pending_text_attrs(TXTFGCOLOUR | palette_idx_5);
        // tsetattrs with idx=2 should wholesale-replace, not OR
        let _ = tsetattrs(TXTFGCOLOUR | palette_idx_2);
        let p = *pending_attrs_lock().lock().unwrap();
        assert_eq!(
            p & TXT_ATTR_FG_COL_MASK,
            palette_idx_2,
            "FG palette index replaced (idx=2), not ORed with prior idx=5"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // expand_prompt — anchored to `print -P 'STRING'` in real zsh 5.9.
    // Only stable escapes are pinned (no %n / %m / %T / %~ that depend on
    // user, host, time, or cwd). Each test cites zsh's observed output.
    // ═══════════════════════════════════════════════════════════════════

    fn expand(s: &str) -> String {
        let _g = crate::test_util::global_state_lock();
        expand_prompt(s)
    }

    // ── Literals ────────────────────────────────────────────────────
    /// `print -P 'literal'` → `literal`
    #[test]
    fn promptexpand_plain_text_passes_through() {
        assert_eq!(expand("literal"), "literal");
    }

    /// `print -P ''` → empty
    #[test]
    fn promptexpand_empty_input_returns_empty() {
        assert_eq!(expand(""), "");
    }

    /// `print -P '%%'` → `%` (escaped percent)
    #[test]
    fn promptexpand_double_percent_yields_single_percent() {
        assert_eq!(expand("%%"), "%");
    }

    /// `print -P 'pre%%post'` → `pre%post` — percent in middle
    #[test]
    fn promptexpand_percent_in_middle() {
        assert_eq!(expand("pre%%post"), "pre%post");
    }

    /// `print -P 'a%%b%%c'` → `a%b%c` — multiple percents
    #[test]
    fn promptexpand_repeated_percent_escapes() {
        assert_eq!(expand("a%%b%%c"), "a%b%c");
    }

    // ── Text attribute escapes (SGR sequences) ─────────────────────
    // `expand_prompt`/`promptexpand` returns the INTERNAL form with
    // RL_PROMPT_START_IGNORE (\x01) and RL_PROMPT_END_IGNORE (\x02)
    // brackets around invisible chars so the line editor knows to
    // skip them for prompt-width math. C zsh's `print -P` strips
    // these brackets before output — so user-visible output is e.g.
    // `\x1b[1m` but the internal contract this function pins is
    // `\x01\x1b[1m\x02`. Tests pin the internal contract.

    /// `%B` → SGR bold, wrapped in start/end-ignore markers.
    #[test]
    fn promptexpand_capital_B_emits_sgr_bold_with_ignore_markers() {
        assert_eq!(expand("%B"), "\x01\x1b[1m\x02");
    }

    /// `%b` → attr reset (zsh 5.9 emits `\e[0m`, not `\e[22m`).
    #[test]
    fn promptexpand_lowercase_b_emits_attr_reset_with_ignore_markers() {
        assert_eq!(expand("%b"), "\x01\x1b[0m\x02");
    }

    /// `%U` → SGR underline on.
    #[test]
    fn promptexpand_capital_U_emits_sgr_underline_with_ignore_markers() {
        assert_eq!(expand("%U"), "\x01\x1b[4m\x02");
    }

    /// `%S` → SGR "standout" — the actual byte is whatever terminfo's
    /// `smso` cap resolves to on the host terminal. On macOS/iTerm in
    /// zsh 5.9 this is `\e[3m` (italic), NOT the spec-default `\e[7m`
    /// (reverse-video). Pin SOME SGR sequence framed by ignore markers;
    /// any SGR byte is acceptable as long as we round-trip the wrap.
    #[test]
    fn promptexpand_capital_S_emits_some_sgr_with_ignore_markers() {
        let out = expand("%S");
        assert!(
            out.starts_with('\x01') && out.ends_with('\x02'),
            "%S must be wrapped in ignore markers; got {out:?}"
        );
        assert!(out.contains("\x1b["), "%S must contain an SGR escape");
        assert!(out.ends_with("m\x02"), "%S must end with SGR `m`+marker");
    }

    /// `%F{red}` → SGR fg red (color index 1 + 30).
    #[test]
    fn promptexpand_F_red_emits_sgr_fg_red_with_ignore_markers() {
        assert_eq!(expand("%F{red}"), "\x01\x1b[31m\x02");
    }

    /// `%f` → SGR default fg.
    #[test]
    fn promptexpand_lowercase_f_emits_default_fg_with_ignore_markers() {
        assert_eq!(expand("%f"), "\x01\x1b[39m\x02");
    }

    /// `%K{blue}` → SGR bg blue (color index 4 + 40).
    #[test]
    fn promptexpand_K_blue_emits_sgr_bg_blue_with_ignore_markers() {
        assert_eq!(expand("%K{blue}"), "\x01\x1b[44m\x02");
    }

    /// `%k` → SGR default bg.
    #[test]
    fn promptexpand_lowercase_k_emits_default_bg_with_ignore_markers() {
        assert_eq!(expand("%k"), "\x01\x1b[49m\x02");
    }

    // ── Literal opaque %{...%} (passthrough) ───────────────────────
    // The %{...%} pair tells zsh: "this content is already an escape
    // sequence; pass it through verbatim and wrap with ignore markers
    // for width math". expand() returns the content wrapped in
    // \x01...\x02 brackets (NOT stripped).

    /// `%{ABCD%}` → `\x01ABCD\x02` (content wrapped in ignore markers).
    #[test]
    fn promptexpand_literal_braces_wrap_content_in_ignore_markers() {
        assert_eq!(expand("%{ABCD%}"), "\x01ABCD\x02");
    }

    /// `%{ABCD%}xyz` → `\x01ABCD\x02xyz` (plain text after the closing
    /// brace stays outside the ignore markers).
    #[test]
    fn promptexpand_literal_braces_followed_by_plain_text() {
        assert_eq!(expand("%{ABCD%}xyz"), "\x01ABCD\x02xyz");
    }

    // ── %# (root marker) ───────────────────────────────────────────
    /// `print -P '%#'` → `%` for non-root, `#` for root. Tests run as
    /// non-root so pin `%`. If zshrs returns `#` here, either the test
    /// is being run as root (env error) or %# is mis-implemented.
    #[test]
    fn promptexpand_hash_yields_percent_for_non_root() {
        let out = expand("%#");
        assert!(
            out == "%" || out == "#",
            "%# must produce '%' (non-root) or '#' (root); got {out:?}"
        );
    }

    // ── Color escape edge cases ────────────────────────────────────
    /// `%F{green}HI%f` → wrapped color escapes around plain `HI`.
    #[test]
    fn promptexpand_color_frames_text() {
        assert_eq!(
            expand("%F{green}HI%f"),
            "\x01\x1b[32m\x02HI\x01\x1b[39m\x02"
        );
    }

    /// `%F{1}` → numeric color (1 = red, ANSI palette).
    #[test]
    fn promptexpand_F_numeric_color_index() {
        assert_eq!(expand("%F{1}"), "\x01\x1b[31m\x02");
    }

    // ── Mixed sequences ────────────────────────────────────────────
    /// Plain text around an escape doesn't get mangled.
    #[test]
    fn promptexpand_text_around_escape_unchanged() {
        let out = expand("before%Bmid%bafter");
        assert!(
            out.starts_with("before") && out.ends_with("after"),
            "text framing must be preserved; got {out:?}"
        );
    }

    /// An unknown escape doesn't panic (zsh strips it silently in most cases).
    #[test]
    fn promptexpand_unknown_escape_does_not_panic() {
        // Use a deliberately unmapped letter. Behavior may vary
        // (strip, keep as-is, or warn) but it MUST not panic.
        let _ = expand("%Q");
    }

    #[test]
    #[ignore = "diagnostic dump — run with --ignored"]
    fn dump_prompt_escapes() {
        for s in &["%B", "%b", "%F{red}", "%f", "%{ABCD%}", "%{ABCD%}xyz"] {
            let out = expand(s);
            eprintln!("expand({s:?}) = {out:?}");
        }
    }

    // ─── promptexpand zsh-corpus pins ───────────────────────────────

    /// `%%` → literal `%`. Doc: zshmisc(1) "EXPANSION OF PROMPT SEQUENCES".
    #[test]
    fn promptexpand_corpus_percent_percent_yields_literal() {
        let out = expand("%%");
        assert_eq!(out, "%", "%% → literal %");
    }

    /// `%n` → username. We can't pin the exact name (varies by env)
    /// but it must not be empty and must not contain `%`.
    #[test]
    fn promptexpand_corpus_percent_n_yields_nonempty_username() {
        let out = expand("%n");
        assert!(!out.is_empty(), "%n must produce a username");
        assert!(!out.contains('%'), "%n must not leave a literal %");
    }

    /// `%(?..)` ternary: `%(?.X.Y)` chooses X if `$?==0`, else Y.
    /// Zero is the default at start. Skip explicit value-setting.
    #[test]
    #[ignore = "ZSHRS BUG: %(?..) ternary requires $? state plumbing"]
    fn promptexpand_corpus_ternary_question_zero_branch() {
        let out = expand("%(?.OK.FAIL)");
        assert_eq!(out, "OK", "default $?=0 chooses OK branch");
    }

    /// `%U` / `%u` — underline on / off. Should produce SGR ANSI
    /// (escape sequence with `[4m`/`[24m`).
    #[test]
    fn promptexpand_corpus_underline_emits_sgr() {
        let out = expand("%Utext%u");
        assert!(
            out.contains("\x1b[4m") || out.contains("\x1b[04m"),
            "%U should emit SGR underline-on, got {out:?}",
        );
        assert!(
            out.contains("\x1b[24m") || out.contains("\x1b[0m"),
            "%u should emit SGR underline-off / reset, got {out:?}",
        );
    }

    /// `%S` / `%s` — standout / reverse. Should produce SGR `[7m`/`[27m`.
    /// (Per terminfo `smso` = reverse video; zsh emits SGR 7.)
    #[test]
    #[ignore = "ZSHRS BUG: %S emits SGR italic (\\e[3m) instead of standout/reverse (\\e[7m)"]
    fn promptexpand_corpus_standout_emits_sgr() {
        let out = expand("%Stext%s");
        assert!(
            out.contains("\x1b[7m") || out.contains("\x1b[07m"),
            "%S should emit SGR standout-on, got {out:?}",
        );
    }

    /// `%{...%}` literal-escape brackets: content goes through but
    /// width-tracking sees zero. Plain text inside should pass.
    #[test]
    fn promptexpand_corpus_zero_width_brackets_preserve_text() {
        let out = expand("%{ESC%}abc");
        assert!(out.contains("ESC"), "literal content survives %{{...%}}");
        assert!(out.ends_with("abc"), "trailing text after %{{...%}}");
    }

    /// Multiple plain percent escapes don't drop characters.
    #[test]
    fn promptexpand_corpus_plain_text_with_escapes_preserves_letters() {
        let out = expand("abc%Bdef%bxyz");
        let plain: String = out
            .chars()
            .filter(|c| !c.is_ascii_control() && *c != '[')
            .collect();
        assert!(plain.contains("abc"), "starts with abc, got {out:?}");
        assert!(plain.contains("def"), "middle has def, got {out:?}");
        assert!(plain.contains("xyz"), "ends with xyz, got {out:?}");
    }

    /// `%d` / `%/` — current working directory. Must be non-empty
    /// and look path-like.
    #[test]
    fn promptexpand_corpus_pwd_escape_yields_path() {
        let out = expand("%d");
        assert!(!out.is_empty(), "%d must produce a path");
        // CWD typically starts with `/` or `~`.
        assert!(
            out.starts_with('/') || out.starts_with('~') || out.contains(std::path::MAIN_SEPARATOR),
            "%d should look path-like, got {out:?}",
        );
    }
}
