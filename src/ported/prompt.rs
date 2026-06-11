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
    /// `buf` field.
    pub buf: Vec<u8>,
    /// `bufspc` field.
    pub bufspc: usize,
    /// `bp` field.
    pub bp: usize,
    /// `bufline` field.
    pub bufline: usize,
    /// `bp1` field.
    pub bp1: Option<usize>,
    /// `fm` field.
    pub fm: String,
    /// `fm_pos` field.
    pub fm_pos: usize,
    /// `truncwidth` field.
    pub truncwidth: i32,
    /// `dontcount` field.
    pub dontcount: i32,
    /// `trunccount` field.
    pub trunccount: i32,
    /// `rstring` field.
    pub rstring: Option<String>,
    pub Rstring: Option<String>,
    // WARNING: NOT IN PROMPT.C — Rust-only expander state.
    // C threads the current zattr inline as it emits SGR bytes into
    // `bp` (no field on `struct buf_vars`); Rust caches the current
    // attribute set on the buf_vars so apply_attrs() / reset_attrs()
    // can emit incremental diffs instead of re-emitting the whole
    // SGR every step.
    /// `attrs` field.
    attrs: zattr,
    // WARNING: NOT IN PROMPT.C — Rust-only readline `\x01`/`\x02`
    // prompt-width-ignore glue. C zsh's `%{ %}` nesting is tracked
    // by `dontcount` (which IS in C buf_vars, above). This separate
    // bool covers the readline-style RL_PROMPT_*_IGNORE byte
    // emissions that the host's readline-compat shim needs around
    // any escape-sequence span.
    /// `in_escape` field.
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
    // c:189-190 init_term lazy-load — lives in expand_prompt (the
    // shared Rust entry; bin_print -P / PS1 / PS4 call it directly).
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

/// Port of `static zattr parsecolorchar(zattr arg, int is_fg)` from
/// `Src/prompt.c:318`. C body:
/// ```c
/// if (bv->fm[1] == '{') {
///     char *ep;
///     bv->fm += 2; /* skip over F{ */
///     if ((ep = strchr(bv->fm, '}'))) {
///         … promptexpand the brace body, then
///         arg = match_colour((const char **)&coll, is_fg, 0);
///         bv->fm = ep;
///     } else {
///         arg = match_colour((const char **)&bv->fm, is_fg, 0);
///         if (*bv->fm != '}') bv->fm--;
///     }
/// } else
///     arg = match_colour(NULL, is_fg, arg);
/// return arg;
/// ```
///
/// Reads `bv->fm[1]` to detect the optional `{NAME}` brace form;
/// otherwise treats `arg` as a pre-supplied color index and returns
/// the corresponding `TXTFGCOLOUR|(arg<<SHFT)` zattr packing.
///
/// Sig matches C (bv first by reference, then arg+is_fg). The
/// previous Rust port took `(&str, bool) -> Option<(Color, String)>`
/// which couldn't read bv->fm at all and wasn't callable from the
/// matching C site.
pub fn parsecolorchar(bv: &mut buf_vars, arg: zattr, is_fg: bool) -> zattr {
    // c:318
    use crate::ported::zsh_h::{
        TXTBGCOLOUR, TXTFGCOLOUR, TXT_ATTR_BG_COL_SHIFT, TXT_ATTR_FG_COL_SHIFT,
    };
    let on_bit = if is_fg { TXTFGCOLOUR } else { TXTBGCOLOUR };
    let shift = if is_fg {
        TXT_ATTR_FG_COL_SHIFT
    } else {
        TXT_ATTR_BG_COL_SHIFT
    };
    // c:320 — `if (bv->fm[1] == '{')`.
    if bv.fm.as_bytes().get(bv.fm_pos + 1).copied() == Some(b'{') {
        // c:322 — `bv->fm += 2; /* skip over F{ */`.
        bv.fm_pos += 2;
        let bytes = bv.fm.as_bytes();
        // c:323 — `strchr(bv->fm, '}')`.
        let mut ep = bv.fm_pos;
        while ep < bytes.len() && bytes[ep] != b'}' {
            ep += 1;
        }
        if ep < bytes.len() {
            // c:325-340 — extract name, promptexpand-wrap, match_colour.
            // The promptexpand round-trip lets `%F{%vCOLOR}` resolve
            // dynamic color names; collapsed here to a direct name
            // lookup since the brace-content is consumed verbatim
            // (promptexpand-as-input-of-color-name is rare and the
            // Rust port's color_from_name already handles `bg=` /
            // `fg=` / numeric / named forms).
            let name: String = bv.fm[bv.fm_pos..ep].to_string();
            // c:337 — `bv->fm = ep;` — consume up through the `}`.
            bv.fm_pos = ep;
            if let Some(color) = color_from_name(&name) {
                return on_bit | ((color as zattr) << shift);
            }
            // C falls back to default arg on lookup miss.
            on_bit | (arg << shift)
        } else {
            // c:343-346 — no close-brace; match_colour walks bv->fm
            // and returns. Without the close-brace path, the rest of
            // the prompt would be consumed as the color name — which
            // is wrong. Back up to before `{` and treat as no color.
            if bv.fm_pos > 0 {
                bv.fm_pos -= 1;
            }
            arg
        }
    } else {
        // c:349 — `arg = match_colour(NULL, is_fg, arg)` — with NULL
        // name and pre-supplied arg, returns the color-bits version
        // of arg (no parsing).
        on_bit | (arg << shift)
    }
}

// ---------------------------------------------------------------------------
// Remaining prompt.c entry points (after `putpromptchar` / `buf_vars`)
// ---------------------------------------------------------------------------

/// Port of `static int putpromptchar(int doprint, int endchar)` from
/// `Src/prompt.c:359`. Walks `bv->fm` byte-by-byte; non-`%` chars go
/// straight through `pputc`; `%X` escapes dispatch on the case
/// letter. All helper logic (prefix-arg parse, `%(test.true.false)`
/// conditional, paths, time, attrs, colors) is INLINED here matching
/// the C body — no Rule-0-invented flat helpers.
///
/// State threaded through `bv: &mut buf_vars`:
/// - `bv.fm` / `bv.fm_pos` — format string cursor (C `bv->fm` ptr)
/// - `bv.buf` / `bv.bp` — output buffer + write cursor (C `bv->bp`)
/// - `bv.dontcount` — `%{`/`%}` non-printing span depth
/// - `bv.truncwidth` / `bv.trunccount` — `%[`/`%<`/`%>` truncation state
/// - `bv.attrs` — current zattr set (read+written by `tsetattrs`,
///   `tunsetattrs`, `treplaceattrs`, `applytextattributes`)
///
/// Returns the byte that stopped the walk (0 = end-of-string, else
/// `endchar` match) — matches C's `return *bv->fm` semantics.
pub fn putpromptchar(bv: &mut buf_vars, doprint: i32, endchar: i32) -> i32 {
    // c:359
    use crate::ported::zsh_h::{isset, PROMPTPERCENT};
    use crate::ported::ztype_h::idigit;

    // c:369 — `for (; *bv->fm && *bv->fm != endchar; bv->fm++)`.
    loop {
        let c = match bv.fm.as_bytes().get(bv.fm_pos).copied() {
            Some(0) | None => return 0,                       // c:369 `*bv->fm == 0`
            Some(c) if c == endchar as u8 => return c as i32, // c:369 endchar match
            Some(c) => c,
        };

        let mut arg: i32 = 0; // c:370

        if c == b'%' && isset(PROMPTPERCENT) {
            // c:371
            // c:373 — `int minus = 0; bv->fm++;`
            let mut minus = 0;
            bv.fm_pos += 1;
            // c:374-377 — `if (*bv->fm == '-') { minus = 1; bv->fm++; }`
            if bv.fm.as_bytes().get(bv.fm_pos).copied() == Some(b'-') {
                minus = 1;
                bv.fm_pos += 1;
            }
            // c:378-382 — `if (idigit(*bv->fm)) arg = zstrtol(bv->fm, &bv->fm, 10);
            //              else if (minus) arg = -1;`
            let nb = bv.fm.as_bytes().get(bv.fm_pos).copied().unwrap_or(0);
            if idigit(nb) {
                let start = bv.fm_pos;
                while bv.fm_pos < bv.fm.len() && idigit(bv.fm.as_bytes()[bv.fm_pos]) {
                    bv.fm_pos += 1;
                }
                arg = bv.fm[start..bv.fm_pos].parse::<i32>().unwrap_or(0);
                if minus != 0 {
                    arg = -arg;
                }
            } else if minus != 0 {
                arg = -1;
            }

            // c:383-487 — `if (*bv->fm == '(')` — conditional ternary.
            // C body computes `test` (0/1) for the named condition,
            // then recursively calls `putpromptchar(test==1 && doprint, sep)`
            // for the true branch and `putpromptchar(test==0 && doprint, ')')`
            // for the false branch — the recursive walks share `bv->fm`
            // so the second call resumes where the first stopped.
            if bv.fm.as_bytes().get(bv.fm_pos).copied() == Some(b'(') {
                bv.fm_pos += 1; // c:407 — `*++bv->fm`
                                // c:408-413 — optional digit arg after `(`.
                if bv.fm_pos < bv.fm.len() && idigit(bv.fm.as_bytes()[bv.fm_pos]) {
                    let start = bv.fm_pos;
                    while bv.fm_pos < bv.fm.len() && idigit(bv.fm.as_bytes()[bv.fm_pos]) {
                        bv.fm_pos += 1;
                    }
                    arg = bv.fm[start..bv.fm_pos].parse::<i32>().unwrap_or(0);
                } else if arg < 0 {
                    arg = -arg; // c:412
                }
                // c:414-455 — switch on test char. Subset ported:
                //   `?` — `lastval == arg` (most common ternary)
                //   `#` — `geteuid() == arg`
                //   Others fall through to test=0 (false).
                let tc = bv.fm.as_bytes().get(bv.fm_pos).copied().unwrap_or(0);
                let mut test: i32 = 0;
                match tc {
                    // c:396-413 — `c` / `.` / `~` / `/` / `C` PWD-depth
                    // tests. C body:
                    //   if (finddir(ss)) { arg--; ss += strlen(nd->dir); }
                    //   /*FALLTHROUGH*/
                    //   if (*ss && *ss++ == '/' && *ss)  arg--;
                    //   for (; *ss; ss++)
                    //       if (*ss == '/') arg--;
                    //   if (arg <= 0) test = 1;
                    // Net effect: test=1 when PWD has ≥ (arg+1) path
                    // components after the home-prefix dir (if any).
                    // For the canonical `%(c..yes)` form, arg=0 →
                    // test=1 for any non-empty PWD (matches zsh).
                    b'c' | b'.' | b'~' | b'/' | b'C' => {
                        let pwd = prompt_tls::PWD.with(|c| c.borrow().clone());
                        let home = prompt_tls::HOME.with(|c| c.borrow().clone());
                        // c:399 — `finddir(ss)` matches the home dir for
                        // the `c`/`.`/`~` arms only. The `/`/`C` arms
                        // skip the home strip (FALLTHROUGH from above
                        // bypassed). Honor that:
                        let strip_home = matches!(tc, b'c' | b'.' | b'~');
                        let ss: &str = if strip_home && !home.is_empty() && pwd == home {
                            arg -= 1; // c:400
                            ""
                        } else if strip_home
                            && !home.is_empty()
                            && pwd.starts_with(&format!("{}/", home))
                        {
                            arg -= 1; // c:400
                            &pwd[home.len()..]
                        } else {
                            &pwd
                        };
                        // c:406-407 — `if (*ss && *ss++ == '/' && *ss)
                        // arg--;` — the leading `/` counts iff there's
                        // something after it.
                        let bytes = ss.as_bytes();
                        if bytes.len() >= 2 && bytes[0] == b'/' {
                            arg -= 1; // c:407
                        }
                        // c:408-410 — remaining `/` chars each decrement.
                        let skip_first = if !bytes.is_empty() && bytes[0] == b'/' {
                            1
                        } else {
                            0
                        };
                        for &b in &bytes[skip_first..] {
                            if b == b'/' {
                                arg -= 1; // c:410
                            }
                        }
                        if arg <= 0 {
                            test = 1; // c:411-412
                        }
                    }
                    b'?' => {
                        // c:444-446 — `if (lastval == arg) test = 1;`
                        let lv = prompt_tls::LASTVAL.with(|c| *c.borrow());
                        if lv == arg {
                            test = 1;
                        }
                    }
                    b'#' => {
                        // c:447-449 — `if (geteuid() == arg) test = 1;`
                        let euid = unsafe { libc::geteuid() } as i32;
                        if euid == arg {
                            test = 1;
                        }
                    }
                    // c:447-449 (g sibling of #).
                    b'g' => {
                        // c:447-449 — `if (getegid() == arg) test = 1;`
                        let egid = unsafe { libc::getegid() } as i32;
                        if egid == arg {
                            test = 1;
                        }
                    }
                    // c:458-465 — `l`: line/column position test. C
                    // calls `countprompt` to set `t0` then
                    // `if (t0 >= arg) test = 1;`. zshrs's prompt
                    // expansion runs offline (not bound to a terminal
                    // column), so t0 is effectively 0; arg ≤ 0
                    // satisfies the comparison and the common
                    // `%(l.X.Y)` (no arg) form gets test=1 — matching
                    // zsh when the line is not column-restricted.
                    b'l' => {
                        if 0 >= arg {
                            test = 1;
                        }
                    }
                    // c:477-479 — `L`: SHLVL >= arg.
                    b'L' => {
                        let shlvl = crate::ported::params::getsparam("SHLVL")
                            .and_then(|s| s.parse::<i32>().ok())
                            .unwrap_or(1);
                        if shlvl >= arg {
                            test = 1;
                        }
                    }
                    // c:495-496 — `_`: `test = (cmdsp >= arg)` —
                    // true when the cmdstack has at least `arg` items.
                    // Reads the per-prompt CMDSTACK snapshot
                    // (hydrated from the live parser stack at
                    // putpromptchar entry, mirroring C's read of the
                    // file-static cmdsp).
                    b'_' => {
                        let cmdsp = prompt_tls::CMDSTACK
                            .with(|c| c.borrow().len() as i32);
                        if cmdsp >= arg {
                            test = 1;
                        }
                    }
                    // c:498-499 — `!`: privasserted (root-ish).
                    // Approximate via euid == 0.
                    b'!' => {
                        let euid = unsafe { libc::geteuid() };
                        if euid == 0 {
                            test = 1;
                        }
                    }
                    // c:Src/prompt.c:451-457 — `j`: TRUTH(numjobs >= arg).
                    // Direct port:
                    //   case 'j':
                    //     for (numjobs = 0, j = 1; j <= maxjob; j++)
                    //         if (jobtab[j].stat && jobtab[j].procs &&
                    //             !(jobtab[j].stat & STAT_NOPRINT)) numjobs++;
                    //     if (numjobs >= arg) test = 1;
                    //     break;
                    // Default arg = 0 → `%(j.A.B)` (no num) matches when
                    // numjobs >= 0 which is always true, so the true-text
                    // fires unconditionally — verified vs /opt/homebrew/bin/zsh
                    // (returns "has jobs" even with 0 jobs running). Bug #601.
                    b'j' => {
                        let mut numjobs = 0i32;
                        if let Some(tab_lock) = crate::ported::jobs::JOBTAB.get() {
                            if let Ok(tab) = tab_lock.lock() {
                                let max = crate::ported::jobs::MAXJOB
                                    .get()
                                    .and_then(|m| m.lock().ok().map(|g| *g))
                                    .unwrap_or(0);
                                let mut j = 1usize;
                                while j <= max && j < tab.len() {
                                    let jb = &tab[j];
                                    if jb.stat != 0
                                        && !jb.procs.is_empty()
                                        && (jb.stat & crate::ported::zsh_h::STAT_NOPRINT) == 0
                                    {
                                        numjobs += 1;
                                    }
                                    j += 1;
                                }
                            }
                        }
                        if numjobs >= arg {
                            test = 1;
                        }
                    }
                    // c:Src/prompt.c:466-476 — `e`: funcstack depth >= arg.
                    // Direct port:
                    //   Funcstack fsptr = funcstack;
                    //   test = arg;
                    //   while (fsptr && test > 0) {
                    //       test--;
                    //       fsptr = fsptr->prev;
                    //   }
                    //   test = !test;
                    // Default arg=0 → test=0 → !0 = 1 (truthy).
                    // `%(2e.A.B)` truthy iff depth >= 2. Bug #602.
                    b'e' => {
                        let depth = crate::ported::modules::parameter::FUNCSTACK
                            .lock()
                            .ok()
                            .map(|stk| stk.len() as i32)
                            .unwrap_or(0);
                        let mut t = arg;
                        let mut remaining = depth;
                        while remaining > 0 && t > 0 {
                            t -= 1;
                            remaining -= 1;
                        }
                        if t == 0 {
                            test = 1;
                        }
                    }
                    // c:Src/prompt.c:485-487 — `v`: psvar has at least
                    // `arg` elements. Default arg=0 → always truthy.
                    // Bug #602.
                    b'v' => {
                        let psvar_len = crate::ported::params::getaparam("psvar")
                            .map(|v| v.len() as i32)
                            .unwrap_or(0);
                        if psvar_len >= arg {
                            test = 1;
                        }
                    }
                    // c:Src/prompt.c:489-493 — `V`: same as `v` BUT also
                    // checks that the indexed element is non-empty.
                    // C: `if (psvar && *psvar && arrlen_ge(psvar, arg)) {
                    //         if (*psvar[(arg ? arg : 1) - 1])
                    //             test = 1;
                    //     }`
                    // Default arg=0 → check psvar[0] non-empty. Bug #602.
                    b'V' => {
                        if let Some(psvar) = crate::ported::params::getaparam("psvar") {
                            if !psvar.is_empty() && (psvar.len() as i32) >= arg {
                                let idx = if arg > 0 { arg - 1 } else { 0 };
                                if let Some(elem) = psvar.get(idx as usize) {
                                    if !elem.is_empty() {
                                        test = 1;
                                    }
                                }
                            }
                        }
                    }
                    // c:Src/prompt.c:481-483 — `S`: shell elapsed seconds
                    // (zmonotime - shtimer) >= arg. Without `zmonotime`/
                    // `shtimer` wired, approximate via process-start time;
                    // for the bare `%(S.A.B)` form (arg=0) result is
                    // always true. Bug #602.
                    b'S' => {
                        if arg <= 0 {
                            test = 1;
                        }
                        // For arg > 0 we'd need the shtimer start; not
                        // worth approximating, leave test=0.
                    }
                    _ => {
                        // Other test chars (t, T, d, D, w) — not yet
                        // ported. test stays 0.
                    }
                }
                // c:457-460 — `if (!*bv->fm || !(sep = *++bv->fm)) return 0;`.
                bv.fm_pos += 1; // past the test char
                let sep = match bv.fm.as_bytes().get(bv.fm_pos).copied() {
                    Some(0) | None => return 0,
                    Some(c) => c,
                };
                bv.fm_pos += 1; // c:461 past the sep
                                // c:464-466 — save truncwidth, recurse for true branch.
                let otruncwidth = bv.truncwidth;
                bv.truncwidth = 0;
                let r1 = putpromptchar(bv, if test == 1 { doprint } else { 0 }, sep as i32);
                if r1 == 0 {
                    bv.truncwidth = otruncwidth;
                    return 0;
                }
                // c:469 — `!*++bv->fm` advance past the matched sep.
                bv.fm_pos += 1;
                if bv.fm_pos >= bv.fm.len() {
                    bv.truncwidth = otruncwidth;
                    return 0;
                }
                let r2 = putpromptchar(bv, if test == 0 { doprint } else { 0 }, b')' as i32);
                if r2 == 0 {
                    bv.truncwidth = otruncwidth;
                    return 0;
                }
                bv.truncwidth = otruncwidth;
                bv.fm_pos += 1; // past the `)`
                continue;
            }

            // c:489-507 — `if (!doprint) switch (*bv->fm) { … continue; }`.
            // Parse-only consume of the escape opcode.
            if doprint == 0 {
                let xc = bv.fm.as_bytes().get(bv.fm_pos).copied().unwrap_or(0);
                match xc {
                    b'[' => {
                        // c:491 — `while(idigit(*++bv->fm)); while(*++bv->fm != ']');`
                        while bv.fm_pos + 1 < bv.fm.len() && idigit(bv.fm.as_bytes()[bv.fm_pos + 1])
                        {
                            bv.fm_pos += 1;
                        }
                        while bv.fm_pos + 1 < bv.fm.len() && bv.fm.as_bytes()[bv.fm_pos + 1] != b']'
                        {
                            bv.fm_pos += 1;
                        }
                        bv.fm_pos += 1; // past the ']'
                    }
                    b'<' => {
                        // c:494 — `while(*++bv->fm != '<');`
                        while bv.fm_pos + 1 < bv.fm.len() && bv.fm.as_bytes()[bv.fm_pos + 1] != b'<'
                        {
                            bv.fm_pos += 1;
                        }
                        bv.fm_pos += 1;
                    }
                    b'>' => {
                        // c:497
                        while bv.fm_pos + 1 < bv.fm.len() && bv.fm.as_bytes()[bv.fm_pos + 1] != b'>'
                        {
                            bv.fm_pos += 1;
                        }
                        bv.fm_pos += 1;
                    }
                    b'D' => {
                        // c:500-502 — `if(bv->fm[1]=='{') while(*++bv->fm != '}');`
                        if bv.fm.as_bytes().get(bv.fm_pos + 1).copied() == Some(b'{') {
                            while bv.fm_pos + 1 < bv.fm.len()
                                && bv.fm.as_bytes()[bv.fm_pos + 1] != b'}'
                            {
                                bv.fm_pos += 1;
                            }
                            bv.fm_pos += 1;
                        }
                    }
                    _ => {} // c:506 default
                }
                bv.fm_pos += 1;
                continue;
            }

            // c:509 — `switch (*bv->fm)` — the real escape dispatch.
            let xc = bv.fm.as_bytes().get(bv.fm_pos).copied().unwrap_or(0);
            // c:Src/prompt.c — numeric prefix N on `%/` / `%~` / `%d`
            // keeps only the trailing N path components. Bug #96 in
            // docs/BUGS.md: zshrs ignored the prefix and emitted the
            // full PWD for `%1/` / `%2~` / etc. Reuse the same
            // truncation helper as `%c`/`%C` arms below.
            let trunc_to_last = |path: &str, n: usize| -> String {
                let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
                if parts.len() <= n {
                    path.to_string()
                } else {
                    parts[parts.len() - n..].join("/")
                }
            };
            match xc {
                // c:511-514 — `%~` (pwd with home-tilde, optional N
                // trailing components).
                b'~' => {
                    let pwd = prompt_tls::PWD.with(|c| c.borrow().clone());
                    let home = prompt_tls::HOME.with(|c| c.borrow().clone());
                    let mut s = if !home.is_empty() && pwd.starts_with(&home) {
                        format!("~{}", &pwd[home.len()..])
                    } else {
                        pwd
                    };
                    if arg > 0 {
                        // C `Src/prompt.c::promptpath` walks last N
                        // components of the post-tilde path. `~/foo/bar`
                        // with N=1 → `bar`; the tilde itself counts as
                        // a regular component for the walk, so `~`
                        // alone (or `~/` after strip) stays unchanged.
                        s = trunc_to_last(&s, arg as usize);
                    }
                    stradd(bv, &s);
                }
                // c:515-518 — `%d` / `%/` (pwd, no tilde, optional N
                // trailing components). Direct port of
                // `Src/prompt.c::promptpath(p, npath, 0)`:
                //   npath > 0: keep last npath components
                //   npath < 0: drop first |npath| components (keep
                //              the leading slash + remainder)
                //   npath == 0: full path
                // Bug #340 — the negative arm wasn't ported, so
                // `%-1d` returned the full path.
                b'd' | b'/' => {
                    let pwd = prompt_tls::PWD.with(|c| c.borrow().clone());
                    let s = if arg > 0 {
                        trunc_to_last(&pwd, arg as usize)
                    } else if arg < 0 {
                        // c:Src/prompt.c:144-154 — walk from modp+1
                        // forward, increment npath on each `/`; when
                        // npath reaches 0, truncate there. For
                        // "/usr/local" + npath=-1: walk past "u s r",
                        // hit '/' → npath=0, stop. Truncate at that
                        // '/' → result "/usr".
                        let mut npath = arg;
                        let bytes = pwd.as_bytes();
                        let mut end = bytes.len();
                        let mut i = 1usize; // skip leading '/'
                        while i < bytes.len() {
                            if bytes[i] == b'/' {
                                npath += 1;
                                if npath == 0 {
                                    end = i;
                                    break;
                                }
                            }
                            i += 1;
                        }
                        pwd[..end].to_string()
                    } else {
                        pwd
                    };
                    stradd(bv, &s);
                }
                // c:519-522 — `%c`/`%.` (trailing path component, tilde-home)
                b'c' | b'.' => {
                    let pwd = prompt_tls::PWD.with(|c| c.borrow().clone());
                    let home = prompt_tls::HOME.with(|c| c.borrow().clone());
                    let path = if !home.is_empty() && pwd.starts_with(&home) {
                        format!("~{}", &pwd[home.len()..])
                    } else {
                        pwd
                    };
                    let n = if arg > 0 { arg as usize } else { 1 };
                    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
                    let tail = if parts.len() <= n {
                        path
                    } else {
                        parts[parts.len() - n..].join("/")
                    };
                    stradd(bv, &tail);
                }
                // c:551-556 — `%C` (trailing path component, NO home-tilde).
                // Like `%c` but skips the home-strip step. Bug #38 in
                // docs/BUGS.md: previously fell through to the
                // literal-`%C` default.
                b'C' => {
                    let pwd = prompt_tls::PWD.with(|c| c.borrow().clone());
                    let n = if arg > 0 { arg as usize } else { 1 };
                    let parts: Vec<&str> = pwd.split('/').filter(|s| !s.is_empty()).collect();
                    let tail = if parts.len() <= n {
                        pwd
                    } else {
                        parts[parts.len() - n..].join("/")
                    };
                    stradd(bv, &tail);
                }
                // c:540 — `%n` (username)
                b'n' => {
                    let u = prompt_tls::USER.with(|c| c.borrow().clone());
                    stradd(bv, &u);
                }
                // c:541-560 — `%M` (full hostname)
                b'M' => {
                    let h = prompt_tls::HOST.with(|c| c.borrow().clone());
                    stradd(bv, &h);
                }
                // c:576-596 — `%m` (short hostname; numeric arg N
                // truncates to N segments from the LEFT for positive,
                // from the RIGHT for negative). C body reads
                // `getsparam("HOST")` and truncates around `.`.
                // Bug #38 in docs/BUGS.md: previously fell through
                // to the literal-`%m` default.
                b'm' => {
                    let h = prompt_tls::HOST.with(|c| c.borrow().clone());
                    let n = if arg == 0 { 1 } else { arg };
                    let parts: Vec<&str> = h.split('.').collect();
                    let out: String = if n > 0 {
                        // First n segments from the LEFT.
                        let take = (n as usize).min(parts.len());
                        parts[..take].join(".")
                    } else {
                        // Last |n| segments from the RIGHT.
                        let take = ((-n) as usize).min(parts.len());
                        parts[parts.len() - take..].join(".")
                    };
                    stradd(bv, &out);
                }
                // c:563-570 — `%S` (standout on) / `%s` (off)
                b'S' => {
                    let _ = tsetattrs(TXTSTANDOUT); // c:564
                                                    // c:565 — `applytextattributes(TSC_PROMPT);`. C body emits
                                                    // SGR diff into `bv->buf` framed by Inpar/Outpar markers
                                                    // (the width-ignore wrappers). Rust splits the work:
                                                    // `applytextattributes(flags)` returns the SGR diff string;
                                                    // the prompt-buffer write + Inpar/Outpar bracketing inlined
                                                    // here matching the C `tsetcap(..., TSC_PROMPT)` path
                                                    // (prompt.c:1101-1108).
                    let sgr = applytextattributes(TSC_PROMPT);
                    if !sgr.is_empty() {
                        addbufspc(bv, 1);
                        bv.buf[bv.bp] = Inpar as u8;
                        bv.bp += 1;
                        for &b in sgr.as_bytes() {
                            pputc(bv, b);
                        }
                        addbufspc(bv, 1);
                        bv.buf[bv.bp] = Outpar as u8;
                        bv.bp += 1;
                    }
                }
                b's' => {
                    let _ = tunsetattrs(TXTSTANDOUT); // c:568
                    let sgr = applytextattributes(TSC_PROMPT); // c:569
                    if !sgr.is_empty() {
                        addbufspc(bv, 1);
                        bv.buf[bv.bp] = Inpar as u8;
                        bv.bp += 1;
                        for &b in sgr.as_bytes() {
                            pputc(bv, b);
                        }
                        addbufspc(bv, 1);
                        bv.buf[bv.bp] = Outpar as u8;
                        bv.bp += 1;
                    }
                }
                // c:571-578 — `%B` (bold on) / `%b` (off)
                b'B' => {
                    let _ = tsetattrs(TXTBOLDFACE); // c:572
                    let sgr = applytextattributes(TSC_PROMPT); // c:573
                    if !sgr.is_empty() {
                        addbufspc(bv, 1);
                        bv.buf[bv.bp] = Inpar as u8;
                        bv.bp += 1;
                        for &b in sgr.as_bytes() {
                            pputc(bv, b);
                        }
                        addbufspc(bv, 1);
                        bv.buf[bv.bp] = Outpar as u8;
                        bv.bp += 1;
                    }
                }
                b'b' => {
                    let _ = tunsetattrs(TXTBOLDFACE); // c:576
                    let sgr = applytextattributes(TSC_PROMPT); // c:577
                    if !sgr.is_empty() {
                        addbufspc(bv, 1);
                        bv.buf[bv.bp] = Inpar as u8;
                        bv.bp += 1;
                        for &b in sgr.as_bytes() {
                            pputc(bv, b);
                        }
                        addbufspc(bv, 1);
                        bv.buf[bv.bp] = Outpar as u8;
                        bv.bp += 1;
                    }
                }
                // c:579-586 — `%U` (underline on) / `%u` (off)
                b'U' => {
                    let _ = tsetattrs(TXTUNDERLINE); // c:580
                    let sgr = applytextattributes(TSC_PROMPT); // c:581
                    if !sgr.is_empty() {
                        addbufspc(bv, 1);
                        bv.buf[bv.bp] = Inpar as u8;
                        bv.bp += 1;
                        for &b in sgr.as_bytes() {
                            pputc(bv, b);
                        }
                        addbufspc(bv, 1);
                        bv.buf[bv.bp] = Outpar as u8;
                        bv.bp += 1;
                    }
                }
                b'u' => {
                    let _ = tunsetattrs(TXTUNDERLINE); // c:584
                    let sgr = applytextattributes(TSC_PROMPT); // c:585
                    if !sgr.is_empty() {
                        addbufspc(bv, 1);
                        bv.buf[bv.bp] = Inpar as u8;
                        bv.bp += 1;
                        for &b in sgr.as_bytes() {
                            pputc(bv, b);
                        }
                        addbufspc(bv, 1);
                        bv.buf[bv.bp] = Outpar as u8;
                        bv.bp += 1;
                    }
                }
                // c:621-644 — `%F` (fg color, fall through to `%f` if invalid).
                // C: `atr = parsecolorchar(arg, 1)` (c:318) reads bv->fm
                // for `{NAME}` brace-arg, else calls
                // `match_colour(NULL, is_fg, arg)`. For bare `%F`
                // (no `{`, default arg=0), match_colour returns
                // `TXTFGCOLOUR | 0` — truthy, color 0 (black) →
                // `\e[30m`. Same for `%K` → `\e[40m`. Rust
                // parsecolorchar diverged (takes name string only) so
                // inline the brace-parse + match_colour(NULL,_,arg)
                // semantics here.
                b'F' | b'K' => {
                    let is_fg = xc == b'F';
                    // c:589-595 — `if (bv->fm[1] == '{') { ... }`. Parse
                    // optional `{NAME}` arg.
                    let mut color: Option<Color> = None;
                    if bv.fm.as_bytes().get(bv.fm_pos + 1).copied() == Some(b'{') {
                        let start = bv.fm_pos + 2;
                        let mut end = start;
                        while end < bv.fm.len() && bv.fm.as_bytes()[end] != b'}' {
                            end += 1;
                        }
                        if end < bv.fm.len() {
                            let name = &bv.fm[start..end];
                            color = color_from_name(name);
                            bv.fm_pos = end; // leave on `}`; outer +=1 advances past
                        }
                    } else if arg >= 0 {
                        // c:Src/prompt.c:349 — `match_colour(NULL, is_fg,
                        // arg)` returns `on | (arg << shft)`; bare `%F`
                        // has arg=0 → color 0 (black) → SGR 30.
                        color = Some(arg as Color);
                    }
                    if let Some(c) = color {
                        // c:596-599 — `tsetattrs(atr); applytextattributes(TSC_PROMPT);`
                        let attr = if is_fg {
                            zattr_set_fg_palette(0, c as u8)
                        } else {
                            zattr_set_bg_palette(0, c as u8)
                        };
                        let _ = tsetattrs(attr);
                        let sgr = applytextattributes(TSC_PROMPT);
                        if !sgr.is_empty() {
                            addbufspc(bv, 1);
                            bv.buf[bv.bp] = Inpar as u8;
                            bv.bp += 1;
                            for &b in sgr.as_bytes() {
                                pputc(bv, b);
                            }
                            addbufspc(bv, 1);
                            bv.buf[bv.bp] = Outpar as u8;
                            bv.bp += 1;
                        }
                    } else {
                        // c:600-602 fall through to lowercase variant.
                        // C's tunsetattrs/applytextattributes emits the
                        // default-color SGR (`\e[39m`/`\e[49m`) even when
                        // no color was previously set — `%F{invalid}`
                        // produces visible recovery output. zshrs's
                        // applytextattributes is a DIFF emitter so a
                        // no-change tunsetattrs returns empty; emit the
                        // default-color code explicitly here. Bug #372.
                        let mask = if is_fg { TXTFGCOLOUR } else { TXTBGCOLOUR };
                        let _ = tunsetattrs(mask);
                        let mut sgr = applytextattributes(TSC_PROMPT);
                        if sgr.is_empty() {
                            sgr = if is_fg {
                                "\x1b[39m".to_string()
                            } else {
                                "\x1b[49m".to_string()
                            };
                        }
                        addbufspc(bv, 1);
                        bv.buf[bv.bp] = Inpar as u8;
                        bv.bp += 1;
                        for &b in sgr.as_bytes() {
                            pputc(bv, b);
                        }
                        addbufspc(bv, 1);
                        bv.buf[bv.bp] = Outpar as u8;
                        bv.bp += 1;
                    }
                }
                b'f' | b'k' => {
                    // c:604-606 — bare `%f`/`%k` reset fg/bg only.
                    // C's tunsetattrs path always emits the default-
                    // color SGR for the corresponding axis. zshrs's
                    // applytextattributes is diff-only, so the
                    // no-color-set case returned empty; emit
                    // `\e[39m`/`\e[49m` explicitly. Bug #372.
                    let is_fg = xc == b'f';
                    let mask = if is_fg { TXTFGCOLOUR } else { TXTBGCOLOUR };
                    let _ = tunsetattrs(mask);
                    let mut sgr = applytextattributes(TSC_PROMPT);
                    if sgr.is_empty() {
                        sgr = if is_fg {
                            "\x1b[39m".to_string()
                        } else {
                            "\x1b[49m".to_string()
                        };
                    }
                    addbufspc(bv, 1);
                    bv.buf[bv.bp] = Inpar as u8;
                    bv.bp += 1;
                    for &b in sgr.as_bytes() {
                        pputc(bv, b);
                    }
                    addbufspc(bv, 1);
                    bv.buf[bv.bp] = Outpar as u8;
                    bv.bp += 1;
                }
                // c:625-635 — `%{` (begin dontcount span)
                b'{' => {
                    // c:626 — `if (!bv->dontcount++) { addbufspc(1); *bv->bp++ = Inpar; }`
                    if bv.dontcount == 0 {
                        addbufspc(bv, 1);
                        bv.buf[bv.bp] = Inpar as u8;
                        bv.bp += 1;
                    }
                    bv.dontcount += 1;
                }
                // c:644-651 — `%}` (end dontcount span)
                b'}' => {
                    if bv.dontcount > 0 {
                        bv.dontcount -= 1;
                        if bv.dontcount == 0 {
                            addbufspc(bv, 1);
                            bv.buf[bv.bp] = Outpar as u8;
                            bv.bp += 1;
                        }
                    }
                }
                // c:706-708 — `%#` — `# ` if root else `% `
                b'#' => {
                    let euid = unsafe { libc::geteuid() };
                    pputc(bv, if euid == 0 { b'#' } else { b'%' });
                }
                // c:709-711 — `%?` (last command status)
                b'?' => {
                    let lv = prompt_tls::LASTVAL.with(|c| *c.borrow());
                    stradd(bv, &lv.to_string());
                }
                // c:563-570 — `%j` (job count). C body:
                // ```c
                // for (numjobs = 0, j = 1; j <= maxjob; j++)
                //     if (jobtab[j].stat && jobtab[j].procs &&
                //         !(jobtab[j].stat & STAT_NOPRINT)) numjobs++;
                // bv->bp += sprintf(bv->bp, "%d", numjobs);
                // ```
                b'j' => {
                    let mut numjobs = 0i32;
                    if let Some(tab_lock) = crate::ported::jobs::JOBTAB.get() {
                        if let Ok(tab) = tab_lock.lock() {
                            let max = crate::ported::jobs::MAXJOB
                                .get()
                                .and_then(|m| m.lock().ok().map(|g| *g))
                                .unwrap_or(0);
                            // c:564 — `for (j = 1; j <= maxjob; j++)`.
                            let mut j = 1usize;
                            while j <= max && j < tab.len() {
                                let jb = &tab[j];
                                if jb.stat != 0
                                    && !jb.procs.is_empty()
                                    && (jb.stat & crate::ported::zsh_h::STAT_NOPRINT) == 0
                                {
                                    numjobs += 1; // c:567
                                }
                                j += 1;
                            }
                        }
                    }
                    stradd(bv, &numjobs.to_string()); // c:569
                }
                // c:558-562 — `%!` / `%h` (current history number). C body:
                // ```c
                // addbufspc(DIGBUFSIZE);
                // convbase(bv->bp, curhist, 10);
                // ```
                b'!' | b'h' => {
                    let n = crate::ported::hist::curhist.load(std::sync::atomic::Ordering::SeqCst);
                    stradd(bv, &n.to_string());
                }
                // c:703-770 — `%t %T %@ %* %w %W %D` — time / date dispatch
                // via strftime. The format string is fixed per-escape; for
                // `%D` followed by `{...}` the format comes from inside the
                // braces.
                //
                // C body (paraphrased):
                // ```c
                // case 'T': tmfmt = "%K:%M"; break;
                // case '*': tmfmt = "%K:%M:%S"; break;
                // case 'w': tmfmt = "%a %f"; break;
                // case 'W': tmfmt = "%m/%d/%y"; break;
                // case 'D':
                //     if (bv->fm[1]=='{') { read format from braces; }
                //     else tmfmt = "%y-%m-%d";
                //     break;
                // default: tmfmt = "%l:%M%p"; break;  // %t, %@
                // ztrftime(...);
                // ```
                b't' | b'T' | b'@' | b'*' | b'w' | b'W' | b'D' => {
                    let tmfmt: String;
                    match xc {
                        // c:715 — exact C source: `tmfmt = "%K:%M";`.
                        // %K is zsh's 24-hr-no-leading-zero extension
                        // (handled by ztrftime preprocessor at
                        // utils.rs:4279). zsh renders 9 AM as `9:06`,
                        // NOT `09:06`. Bug #619 sibling of #599.
                        b'T' => tmfmt = "%K:%M".to_string(),    // c:715
                        // c:718 — exact C source: `tmfmt = "%K:%M:%S";`.
                        b'*' => tmfmt = "%K:%M:%S".to_string(), // c:718
                        // c:721 — exact C source: `tmfmt = "%a %f";`.
                        // The `%f` extension is handled by zshrs's
                        // ztrftime preprocessor at utils.rs:4293 →
                        // `tm_mday` with no leading space (vs `%e`
                        // which pads single-digit days with a space).
                        // zsh's `%w` renders as `Thu 4` not `Thu  4`.
                        // Bug #599.
                        b'w' => tmfmt = "%a %f".to_string(),    // c:721
                        b'W' => tmfmt = "%m/%d/%y".to_string(), // c:724
                        b'D' => {
                            // c:727-746 — `%D{...}` format from braces;
                            // bare `%D` → "%y-%m-%d".
                            if bv.fm.as_bytes().get(bv.fm_pos + 1).copied() == Some(b'{') {
                                // Walk from `{` to matching `}`, honouring
                                // `\X` → X drop.
                                let bytes = bv.fm.as_bytes();
                                let mut ss = bv.fm_pos + 2; // c:729 past `{`
                                let mut collected = String::new();
                                while ss < bytes.len() && bytes[ss] != b'}' {
                                    if bytes[ss] == b'\\' && ss + 1 < bytes.len() {
                                        // c:732-733 — drop backslash, keep next.
                                        ss += 1;
                                        collected.push(bytes[ss] as char);
                                    } else {
                                        collected.push(bytes[ss] as char);
                                    }
                                    ss += 1;
                                }
                                // c:741 — `bv->fm = ss - !*ss;` — leave fm
                                // pointing AT the `}` so the post-switch
                                // `bv.fm_pos += 1` below lands one past it.
                                bv.fm_pos = ss;
                                if collected.is_empty() {
                                    bv.fm_pos += 1;
                                    continue;
                                }
                                tmfmt = collected;
                            } else {
                                tmfmt = "%y-%m-%d".to_string(); // c:748
                            }
                        }
                        // c:751 — default for %t / %@ → 12-hour clock.
                        _ => tmfmt = "%l:%M%p".to_string(),
                    }
                    // c:753-770 — `zgettime + localtime + ztrftime`. Port
                    // routes through `utils::ztrftime` which already wraps
                    // strftime + format quirks.
                    let now = std::time::SystemTime::now();
                    // c:765 — `ztrftime(buf, ..., localtime(&secs), nsec)`,
                    // so use_gmt = false.
                    let rendered = crate::ported::utils::ztrftime(&tmfmt, now, false);
                    stradd(bv, &rendered);
                }
                // c:923-929 — `%i` reads the global `lineno`. The Rust
                // mirror is `crate::ported::input::lineno` (thread-local
                // Cell<usize>, init 1). C also has a fallthrough from
                // `'I'` when inside a funcstack with FS_INSCRIPT —
                // omitted here since the funcstack-line branch is
                // already handled at the `'I'` arm. Bug #138 in
                // docs/BUGS.md.
                b'i' => {
                    // zshrs's `crate::ported::input::lineno` is stuck
                    // at function entry and doesn't track per-statement
                    // execution — read `$LINENO` instead which the
                    // executor maintains correctly through each
                    // statement. C zsh uses the same lineno global
                    // that backs $LINENO. Bug #618.
                    let ln = crate::ported::params::getsparam("LINENO")
                        .and_then(|s| s.parse::<i64>().ok())
                        .unwrap_or_else(|| {
                            crate::ported::input::lineno.with(|l| l.get()) as i64
                        });
                    stradd(bv, &ln.to_string());
                }
                // c:Src/prompt.c:889 — `%L` emits the current `$SHLVL`
                // value (shell-nesting depth). Direct port:
                // ```c
                // case 'L':
                //     addbufspc(DIGBUFSIZE);
                //     sprintf(bv->bp, "%ld", (long)shlvl);
                //     bv->bp += strlen(bv->bp);
                //     break;
                // ```
                // Read `$SHLVL` from paramtab (kept in sync with the
                // executor on shell startup) rather than the env var
                // since paramtab is the authoritative source after
                // assignments. Bug #598.
                b'L' => {
                    let shlvl = crate::ported::params::getsparam("SHLVL")
                        .and_then(|s| s.parse::<i32>().ok())
                        .or_else(|| std::env::var("SHLVL").ok().and_then(|s| s.parse().ok()))
                        .unwrap_or(1);
                    stradd(bv, &shlvl.to_string());
                }
                // c:Src/prompt.c:554-555 — `%N` reads C's `scriptname`
                //   global (`promptpath(scriptname ? scriptname :
                //   argzero, arg, 0)`). `scriptname` is updated when
                //   entering a function to the function's name
                //   (Src/exec.c:5903 `scriptname = dupstring(name);`)
                //   and restored on return (c:6064), so `%N` reflects
                //   the CURRENT executing scope — top-level → script
                //   name; inside a function → function name. zshrs
                //   has the same global at `utils::scriptname_get()`,
                //   updated at exec.rs:5585. Bug #318 family in
                //   docs/BUGS.md — earlier port read only ZSH_SCRIPT
                //   so `%N` stayed at the outer scope inside fns.
                b'N' => {
                    let nam = crate::ported::utils::scriptname_get()
                        .filter(|s: &String| !s.is_empty())
                        .or_else(|| {
                            crate::ported::params::getsparam("ZSH_SCRIPT")
                                .filter(|s| !s.is_empty())
                        })
                        .or_else(|| {
                            crate::ported::params::getsparam("ZSH_NAME")
                                .filter(|s| !s.is_empty())
                        })
                        .unwrap_or_else(|| "zsh".to_string());
                    stradd(bv, &nam);
                }
                // c:Src/prompt.c:931-940 — `%x` (file name of script
                // being executed; same surface as %N for most contexts,
                // differs inside autoloaded functions). Prefers
                // ZSH_SCRIPT, falls back to ZSH_NAME for -c mode.
                // c:Src/prompt.c:931-938 — `%x`:
                //   if (funcstack && funcstack->tp != FS_SOURCE && !IN_EVAL_TRAP())
                //     promptpath(funcstack->filename ?: "", arg, 0);
                //   else
                //     promptpath(scriptfilename ?: argzero, arg, 0);
                //
                // %x is the FILE that contains the code currently
                // being parsed/executed: the active function's
                // source file when inside a function (NOT a sourced
                // top-level), else the file-static
                // `scriptfilename`, falling back to `argzero`. The
                // SCRIPTFILENAME / FUNCSTACK_FILENAME TLS were
                // hydrated at putpromptchar entry from
                // utils::scriptfilename_get() and the live
                // funcstack — both update under bin_dot, doshfunc,
                // and the startup source_from_memory wiring. Prior
                // port read `$ZSH_SCRIPT` / `$ZSH_NAME` params,
                // which don't move on `source`, so `%x` stayed at
                // "zsh" through .zshenv / .zshrc execution.
                b'x' => {
                    let in_fn_filename = prompt_tls::FUNCSTACK_FILENAME
                        .with(|c| c.borrow().clone());
                    let nam = if let Some(fname) = in_fn_filename {
                        // Inside a function (not a sourced top-level)
                        // — use funcstack->filename. Hydration at
                        // prompt.rs:166 reads the last funcstack
                        // entry's filename; that's the closest
                        // mirror we have of the C
                        // `funcstack->filename` walk.
                        fname
                    } else {
                        prompt_tls::SCRIPTFILENAME
                            .with(|c| c.borrow().clone())
                            .or_else(|| prompt_tls::ARGEXTRA.with(|c| {
                                let s = c.borrow().clone();
                                if s.is_empty() { None } else { Some(s) }
                            }))
                            .unwrap_or_else(|| "zsh".to_string())
                    };
                    stradd(bv, &nam);
                }
                // c:Src/prompt.c:889-900 — `%e` (function-stack depth):
                //   int depth = 0;
                //   Funcstack fsptr = funcstack;
                //   while (fsptr) { depth++; fsptr = fsptr->prev; }
                //   bv->bp += sprintf(bv->bp, "%d", depth);
                // Counts the live funcstack (function calls + sourced
                // files + traps). At top level, depth = 0 → emits "0".
                b'e' => {
                    let depth = crate::ported::modules::parameter::FUNCSTACK
                        .lock()
                        .ok()
                        .map(|stk| stk.len())
                        .unwrap_or(0);
                    stradd(bv, &depth.to_string());
                }
                // c:Src/prompt.c:901-920 — `%I` (absolute source-file
                // line). When inside a function (not FS_SOURCE / FS_EVAL),
                // emit `lineno + funcstack->flineno` — `lineno` is the
                // line offset within the function body (1-based) and
                // `flineno` is the script-line offset where the function
                // body starts. When NOT in a function, FALLTHROUGH to
                // `%i` (just emit `lineno`). Bug #618.
                //
                // zshrs's `crate::ported::input::lineno` is stuck at
                // function entry and doesn't track per-statement
                // execution — read `$LINENO` (the param) instead which
                // is incremented correctly by the executor.
                b'I' => {
                    let cur_lineno: i64 = crate::ported::params::getsparam("LINENO")
                        .and_then(|s| s.parse::<i64>().ok())
                        .unwrap_or(1);
                    let in_fn_offset: Option<i64> = crate::ported::modules::parameter::FUNCSTACK
                        .lock()
                        .ok()
                        .and_then(|stk| stk.last().map(|fs| fs.flineno));
                    let abs = match in_fn_offset {
                        Some(off) => cur_lineno + off,
                        None => cur_lineno,
                    };
                    stradd(bv, &abs.to_string());
                }
                // c:Src/prompt.c:855-880 — `%_` (parser context: the
                // bottom-up cmdstack — print the last `arg` tokens in
                // forward order; `arg <= 0` (default) prints all).
                // `%-N_` prints the FIRST N tokens (top-down).
                // Tokens come from the live parser stack; names map
                // through CMDNAMES[CS_*] (c:62-71).
                //
                //   if (cmdsp) {
                //     if (arg >= 0) {                       // c:857
                //       if (arg > cmdsp || arg == 0) arg = cmdsp;
                //       for (t0 = cmdsp - arg; arg--; t0++) {
                //         stradd(cmdnames[cmdstack[t0]]);
                //         if (arg) addbufspc(1), *bv->bp++ = ' ';
                //       }
                //     } else {                              // c:867
                //       arg = -arg;
                //       if (arg > cmdsp) arg = cmdsp;
                //       for (t0 = 0; arg--; t0++) { ... }
                //     }
                //   }
                b'_' => {
                    let stack = prompt_tls::CMDSTACK.with(|c| c.borrow().clone());
                    let cmdsp = stack.len() as i32;
                    if cmdsp > 0 {
                        let (start, mut count) = if arg >= 0 {
                            let n = if arg == 0 || arg > cmdsp { cmdsp } else { arg };
                            ((cmdsp - n) as usize, n) // c:860 — `t0 = cmdsp - arg`
                        } else {
                            let n = if -arg > cmdsp { cmdsp } else { -arg };
                            (0usize, n) // c:871 — `t0 = 0`
                        };
                        let mut t0 = start;
                        while count > 0 {
                            count -= 1;
                            let idx = stack[t0] as usize;
                            if let Some(name) = CMDNAMES.get(idx) {
                                stradd(bv, name); // c:861 / c:872
                            }
                            if count > 0 {
                                stradd(bv, " "); // c:863-864 / c:874-875
                            }
                            t0 += 1;
                        }
                    }
                }
                // c:Src/prompt.c:829-854 — `%^` (parser context,
                // top-down): print the last `arg` tokens in REVERSE
                // order (newest first). `%-N^` prints the first N
                // tokens in reverse from index N-1 down to 0.
                //
                //   if (cmdsp) {
                //     if (arg >= 0) {                       // c:831
                //       if (arg > cmdsp || arg == 0) arg = cmdsp;
                //       for (t0 = cmdsp - 1; arg--; t0--) { ... }
                //     } else {                              // c:841
                //       arg = -arg;
                //       if (arg > cmdsp) arg = cmdsp;
                //       for (t0 = arg - 1; arg--; t0--) { ... }
                //     }
                //   }
                b'^' => {
                    let stack = prompt_tls::CMDSTACK.with(|c| c.borrow().clone());
                    let cmdsp = stack.len() as i32;
                    if cmdsp > 0 {
                        let (start, mut count) = if arg >= 0 {
                            let n = if arg == 0 || arg > cmdsp { cmdsp } else { arg };
                            ((cmdsp - 1) as usize, n) // c:834 — `t0 = cmdsp - 1`
                        } else {
                            let n = if -arg > cmdsp { cmdsp } else { -arg };
                            ((n - 1) as usize, n) // c:845 — `t0 = arg - 1`
                        };
                        let mut t0 = start as i32;
                        while count > 0 {
                            count -= 1;
                            if t0 < 0 || (t0 as usize) >= stack.len() {
                                break;
                            }
                            let idx = stack[t0 as usize] as usize;
                            if let Some(name) = CMDNAMES.get(idx) {
                                stradd(bv, name); // c:835 / c:846
                            }
                            if count > 0 {
                                stradd(bv, " "); // c:837-838 / c:848-849
                            }
                            t0 -= 1;
                        }
                    }
                }
                // c:777-784 — `%l` (controlling tty, trimmed of
                // `/dev/tty` or `/dev/` prefix). Bug #38 in
                // docs/BUGS.md.
                b'l' => {
                    let tty = unsafe {
                        let p = libc::ttyname(0);
                        if p.is_null() {
                            String::new()
                        } else {
                            std::ffi::CStr::from_ptr(p)
                                .to_str()
                                .unwrap_or("")
                                .to_string()
                        }
                    };
                    if tty.is_empty() {
                        stradd(bv, "()");
                    } else if let Some(rest) = tty.strip_prefix("/dev/tty") {
                        stradd(bv, rest);
                    } else if let Some(rest) = tty.strip_prefix("/dev/") {
                        stradd(bv, rest);
                    } else {
                        stradd(bv, &tty);
                    }
                }
                // c:785-792 — `%y` (controlling tty, trimmed of
                // `/dev/` only — does NOT strip `tty` prefix like
                // `%l` does).
                b'y' => {
                    let tty = unsafe {
                        let p = libc::ttyname(0);
                        if p.is_null() {
                            String::new()
                        } else {
                            std::ffi::CStr::from_ptr(p)
                                .to_str()
                                .unwrap_or("")
                                .to_string()
                        }
                    };
                    if tty.is_empty() {
                        stradd(bv, "()");
                    } else if let Some(rest) = tty.strip_prefix("/dev/") {
                        stradd(bv, rest);
                    } else {
                        stradd(bv, &tty);
                    }
                }
                // c:818-824 — `%v` (psvar[arg-1], default arg=1).
                // psvar is the `$psvar` array.
                b'v' => {
                    let n = if arg == 0 { 1 } else { arg };
                    let psvar = crate::ported::params::getsparam("psvar");
                    let arr: Vec<String> = if let Some(s) = psvar {
                        s.split(' ').map(String::from).collect()
                    } else {
                        crate::ported::exec_hooks::array("psvar").unwrap_or_default()
                    };
                    let idx: i32 = if n < 0 {
                        arr.len() as i32 + n
                    } else {
                        n - 1
                    };
                    if idx >= 0 && (idx as usize) < arr.len() {
                        stradd(bv, &arr[idx as usize]);
                    }
                }
                // c:828-830 — `%E` (clear-to-end-of-line ANSI escape).
                // C: `tsetcap(TCCLEAREOL, TSC_PROMPT);` — emit the
                // terminal's `el` capability. Use the canonical ANSI
                // `ESC [ K` which works on every modern terminal;
                // matches what `tput el` would emit. Bracketed by
                // Inpar/Outpar so the width-counter ignores it.
                b'E' => {
                    let esc = "\x1b[K";
                    addbufspc(bv, 1);
                    bv.buf[bv.bp] = Inpar as u8;
                    bv.bp += 1;
                    for &b in esc.as_bytes() {
                        pputc(bv, b);
                    }
                    addbufspc(bv, 1);
                    bv.buf[bv.bp] = Outpar as u8;
                    bv.bp += 1;
                }
                // c:894-896 — `%%` (literal percent)
                b'%' => pputc(bv, b'%'),
                // c:897 — `%)` (literal close-paren — used in %(x.t.f))
                b')' => pputc(bv, b')'),
                // c:Src/prompt.c:663-675 — `%<...<` / `%>...>` truncation
                // directives. Without numeric prefix the truncation
                // width is 0 (no truncation); with a prefix it bounds
                // the bracketed region. Walks via prompttrunc which
                // handles the matching `<`/`>` close. Without these
                // arms the unknown-escape default below consumed the
                // first `>` and left the second as a literal char,
                // producing visible `>` in the output. Bug #439.
                b'<' | b'>' => {
                    let truncchar = xc as i32;
                    let _ = prompttrunc(bv, arg, truncchar, doprint, endchar);
                }
                // c:Src/prompt.c:657-661 — `%[arg INNER ]content<endchar>`.
                // C body:
                //   if (idigit(*++bv->fm))
                //       arg = zstrtol(bv->fm, &bv->fm, 10);
                //   if (!prompttrunc(arg, ']', doprint, endchar))
                //       return *bv->fm;
                // The `*++bv->fm` advances PAST the `[` before the digit
                // check, AND the inline-digit reparse may consume more.
                // prompttrunc then does another `bv->fm++` past the
                // current char (treating it as the truncstr opener
                // char, not content). The net effect: the first byte
                // immediately after `[` (or after the inline digit run)
                // is silently skipped — see `/bin/zsh -fc 'print -nP
                // "%5[a]bcdef"'` emits `bcdef` (the `a` is skipped,
                // default `<` becomes the marker but content fits).
                b'[' => {
                    let mut local_arg = arg;
                    // c:658 — `*++bv->fm` advances past the `[` byte.
                    bv.fm_pos += 1;
                    let bytes = bv.fm.as_bytes();
                    // c:658-659 — inline digits override arg.
                    if bv.fm_pos < bytes.len() && idigit(bytes[bv.fm_pos]) {
                        let mut end = bv.fm_pos;
                        while end < bytes.len() && idigit(bytes[end]) {
                            end += 1;
                        }
                        let num: i32 = std::str::from_utf8(&bytes[bv.fm_pos..end])
                            .ok()
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0);
                        local_arg = num;
                        bv.fm_pos = end;
                    }
                    let _ = prompttrunc(bv, local_arg, b']' as i32, doprint, endchar);
                    // prompttrunc consumed the truncstr + bracketed
                    // content; the outer `bv.fm_pos += 1` below would
                    // skip a byte of whatever follows. Pre-decrement
                    // to cancel that out.
                    if bv.fm_pos > 0 {
                        bv.fm_pos -= 1;
                    }
                }
                // c:899 — null terminator inside an escape
                0 => return 0,
                // c:Src/prompt.c — unknown `%X`: C's putpromptchar
                // switch has NO default arm. Recognized cases handle
                // their own emit + break; unrecognized chars fall
                // through to the end of the switch and the loop
                // advances `bv->fm` past the unknown char with no
                // output. So `%J`, `%Z`, `%X`, etc. produce empty
                // string in zsh (test: `print -P "before%Jafter"` →
                // `beforeafter`). Bug #239 in docs/BUGS.md — the
                // previous Rust port emitted `%X` literally.
                _ => {}
            }
            // Advance past the escape opcode byte.
            bv.fm_pos += 1;
        } else {
            // c:600-607 — plain char. C: `char c = *bv->fm == Meta ?
            // *++bv->fm ^ 32 : *bv->fm; if (doprint) { addbufspc(1); pputc(c); }`.
            // The doprint guard is what makes ternary false-branches
            // walk without emitting (parse-only consume).
            if doprint != 0 {
                pputc(bv, c);
            }
            bv.fm_pos += 1;
        }
    }
}

/// Port of `static void addbufspc(int need)` from `Src/prompt.c:991`.
///
/// C body:
/// ```c
/// need *= 2;
/// if ((bv->bp - bv->buf) + need > bv->bufspc) {
///     int bo = bv->bp - bv->buf;
///     int bl = bv->bufline - bv->buf;
///     if (need & 255) need = (need | 255) + 1;
///     bv->buf = realloc(bv->buf, bv->bufspc += need);
///     bv->bp = bv->buf + bo;
///     bv->bufline = bv->buf + bl;
/// }
/// ```
///
/// Grows `bv->buf` to fit `need` more bytes (×2 for metafy worst
/// case). C uses raw realloc with a file-static `bv` pointer; Rust
/// takes `&mut buf_vars` explicitly because buf_vars has no impl
/// methods (per maintainer directive — Rule 0).
pub fn addbufspc(bv: &mut buf_vars, mut need: i32) {
    // c:991
    need = need.saturating_mul(2); // c:993
    if bv.bp as i32 + need > bv.bufspc as i32 {
        // c:994
        // c:995-996 — round up to next 256-byte boundary
        if need & 255 != 0 {
            need = (need | 255) + 1;
        }
        let new_size = bv.bufspc as i32 + need;
        bv.buf.resize(new_size as usize, 0); // c:998 realloc
        bv.bufspc = new_size as usize; // c:998 bufspc += need
                                       // bp / bufline are usize indexes (not pointers); no recompute
    }
}

/// Port of `pputc(char c)` from `Src/prompt.c:976`.
///
/// C body:
/// ```c
/// static void
/// pputc(int c)
/// {
///     if (imeta(c)) {
///         addbufspc(2);
///         *bv->bp++ = Meta;
///         c ^= 32;
///     } else {
///         addbufspc(1);
///     }
///     *bv->bp++ = c;
///     if (c == '\n' && !bv->dontcount)
///         bv->bufline = bv->bp;
/// }
/// ```
///
/// Append one byte to `bv->buf`, metafying via `Meta + (c^0x20)`
/// pair if `imeta(c)`. Tracks `bufline` (most recent newline
/// position) when not inside a `%{...%}` dontcount span.
pub fn pputc(bv: &mut buf_vars, mut c: u8) {
    // c:976
    use crate::ported::ztype_h::imeta;
    if imeta(c) {
        // c:978
        addbufspc(bv, 2); // c:979
        bv.buf[bv.bp] = Meta as u8; // c:980 *bv->bp++ = Meta
        bv.bp += 1;
        c ^= 32; // c:981
    } else {
        addbufspc(bv, 1); // c:983
    }
    bv.buf[bv.bp] = c; // c:985 *bv->bp++ = c
    bv.bp += 1;
    if c == b'\n' && bv.dontcount == 0 {
        // c:986
        bv.bufline = bv.bp;
    }
}

/// Port of `void stradd(char *d)` from `Src/prompt.c:1016`.
///
/// C body (MULTIBYTE_SUPPORT arm c:1018-1075):
/// ```c
/// ums = ztrdup(d);
/// ups = unmetafy(ums, &upslen);
/// while (upslen > 0) {
///     cnt = eol ? MB_INVALID : mbrtowc(&cc, ups, upslen, &mbs);
///     switch (cnt) {
///       case MB_INCOMPLETE: eol = 1; /* FALL THROUGH */
///       case MB_INVALID: pc = nicechar(*ups); cnt = 1; break;
///       case 0: cnt = 1; /* FALL THROUGH */
///       default: mb_charinit(); pc = wcs_nicechar(cc, NULL, NULL); break;
///     }
///     addbufspc(strlen(pc));
///     upslen -= cnt; ups += cnt;
///     while (*pc) *bv->bp++ = *pc++;
/// }
/// ```
///
/// Append `d` to `bv->buf` with display-form conversion:
/// 1. unmetafy input (canonical `crate::ported::utils::unmetafy`).
/// 2. UTF-8 decode (Rust's str::from_utf8 = mbrtowc analog).
/// 3. Per char: wcs_nicechar (or nicechar on MB_INVALID).
/// 4. addbufspc + write each byte through `pputc` (which
///    re-metafies high bytes).
pub fn stradd(bv: &mut buf_vars, d: &str) {
    // c:1016
    // c:1023-1025 — `ums = ztrdup(d); ups = unmetafy(ums, &upslen);`
    let mut raw: Vec<u8> = d.as_bytes().to_vec();
    crate::ported::utils::unmetafy(&mut raw);
    // c:1031-1071 — walk decoded bytes.
    match std::str::from_utf8(&raw) {
        Ok(decoded) => {
            // c:1058 default arm — wide char per codepoint.
            for ch in decoded.chars() {
                let pc = crate::ported::utils::wcs_nicechar(ch, None, None);
                addbufspc(bv, pc.len() as i32); // c:1063
                for &b in pc.as_bytes() {
                    pputc(bv, b); // c:1069 `*bv->bp++ = *pc++`
                }
            }
        }
        Err(_) => {
            // c:1046-1051 MB_INVALID arm — per-byte nicechar.
            for &b in &raw {
                let pc = crate::ported::utils::nicechar(b as char);
                addbufspc(bv, pc.len() as i32); // c:1063
                for &out_b in pc.as_bytes() {
                    pputc(bv, out_b); // c:1069
                }
            }
        }
    }
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

thread_local! {
    /// Substrate for the C file-static `bv->bp` prompt-buffer write
    /// cursor (see `Src/prompt.c:76-121` `struct buf_vars`). C's
    /// `putstr` writes one byte at a time into this buffer when used
    /// as a `tputs(3)` per-byte callback (e.g.
    /// `tputs(tcstr[cap], 1, putstr)` at prompt.c:538). The Rust
    /// prompt-emit pipeline builds local Strings and returns them,
    /// so this thread-local is the trampoline for code paths that
    /// follow the C callback shape verbatim. The owning expander
    /// drains the buffer after the `tputs`-equivalent call.
    pub static PUTSTR_BUF: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Direct port of `int putstr(int d)` from `Src/prompt.c:1121`.
///
/// ```c
/// /**/
/// int
/// putstr(int d)
/// {
///     addbufspc(1);
///     pputc(d);
///     return 0;
/// }
/// ```
///
/// Per-byte output callback fed to `tputs(3)` when emitting
/// terminal-capability escapes into the prompt buffer (`tsetcap`'s
/// TSC_PROMPT arm at prompt.c:538). Always returns 0 per
/// `tputs(3)`'s `int (*putc)(int)` callback contract; the byte
/// gets appended to the thread-local `PUTSTR_BUF` which the
/// caller drains.
pub fn putstr(d: i32) -> i32 {
    // c:1121-1126 — `addbufspc(1); pputc(d); return 0;`
    // The C `addbufspc(1)` grows the per-build prompt buffer by one;
    // the Rust `PUTSTR_BUF` Vec grows naturally on `push`. The C
    // `pputc(d)` writes one byte (or two when `d >= 0x83` to
    // metafy); we match: bytes < 0x83 go through verbatim, bytes
    // >= 0x83 emit `Meta + (d ^ 0x20)` per zsh metafication.
    PUTSTR_BUF.with(|b| {
        let mut buf = b.borrow_mut();
        let byte = (d & 0xff) as u8;
        if byte >= 0x83 {
            // c:976 `pputc` — metafy high bytes.
            buf.push(Meta);
            buf.push(byte ^ 0x20);
        } else {
            buf.push(byte);
        }
    });
    0 // c:1125 — `return 0;`
}

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
    } else {
        // c:Src/prompt.c:1608-1617 — `arg <= 0` (no width prefix). Walk
        // past the optional `>`-string content until the matching
        // `truncchar`. Backslash escapes the next byte. Bug #439: the
        // previous Rust port had only the `arg > 0` body, so `%>>`
        // and `%<<` (no prefix) left the second `>`/`<` as a literal
        // char in the output, while the dispatcher had already
        // consumed the first one — visible `>` / `<` artifact.
        let tchar = truncchar as u8;
        let endchar_u8 = endchar as u8;
        if bv.fm.as_bytes().get(bv.fm_pos).copied().unwrap_or(0) != endchar_u8 {
            bv.fm_pos += 1; // c:1610 — past the `<`/`>` marker
        }
        while let Some(&c) = bv.fm.as_bytes().get(bv.fm_pos) {
            if c == 0 || c == tchar {
                break;
            }
            if c == b'\\' && bv.fm.as_bytes().get(bv.fm_pos + 1).is_some() {
                bv.fm_pos += 1; // c:1613
            }
            bv.fm_pos += 1; // c:1614
        }
        // c:1616-1617 — `if (bv->truncwidth || !*bv->fm) return 0;`
        if bv.truncwidth != 0
            || bv.fm.as_bytes().get(bv.fm_pos).copied().unwrap_or(0) == 0
        {
            return 0;
        }
    }
    1 // c:1619
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

    // c:Src/prompt.c:1640-1718 — tsetcap dispatch. Observable
    // /opt/homebrew/bin/zsh output (cross-checked via od -c):
    //   - `%b` (bold off):       full reset \e[0m + re-apply ALL
    //     (no terminfo "bold off" cap; uses `me` = SGR 0).
    //   - `%u` (underline off):  selective \e[24m (terminfo `ue`).
    //   - `%s` (standout off):   selective \e[23m (terminfo `se`,
    //     when standout is mapped to italic).
    //   - `%f` (fg off):         selective \e[39m.
    //   - `%k` (bg off):         selective \e[49m.
    //   - `%B`/`%U`/`%S` (attr on): emit attr-on cap + re-apply
    //     active colors.
    // c:Src/prompt.c:1085 — tsetcap() is a no-op when `termflags &
    // (TERM_NOUP|TERM_BAD|TERM_UNKNOWN)`; every attribute transition
    // in the C body routes through tsetcap, so an unknown/bad/dumb
    // terminal emits no attribute SGRs at all (probe: TERM=dumb
    // `zsh -fc 'print -P "%Shi%s"' | od -c` → plain `hi`). Colours
    // are NOT gated — set_colour_attribute (c:Src/prompt.c:2440)
    // falls back to raw SGR colour sequences when termcap caps are
    // unavailable, so `%F`/`%K` still emit under TERM=dumb.
    let tc_ok = crate::ported::params::TERMFLAGS.load(Ordering::SeqCst)
        & (TERM_NOUP | TERM_BAD | TERM_UNKNOWN)
        == 0;
    let bold_off = tc_ok && old_b && !new_b;
    let underline_off = tc_ok && old_u && !new_u;
    let standout_off = tc_ok && old_s && !new_s;
    let attr_on = tc_ok && ((!old_b && new_b) || (!old_u && new_u) || (!old_s && new_s));
    let fg_emit_color = |attrs, out: &mut String| {
        if attrs & TXTFGCOLOUR != 0 {
            let raw = (attrs & TXT_ATTR_FG_COL_MASK) >> TXT_ATTR_FG_COL_SHIFT;
            let c = if attrs & TXT_ATTR_FG_24BIT != 0 {
                COLOR_24BIT | (raw as Color & 0x00ff_ffff)
            } else {
                raw as Color
            };
            out.push_str(&color_to_ansi(c, true));
        }
    };
    let bg_emit_color = |attrs, out: &mut String| {
        if attrs & TXTBGCOLOUR != 0 {
            let raw = (attrs & TXT_ATTR_BG_COL_MASK) >> TXT_ATTR_BG_COL_SHIFT;
            let c = if attrs & TXT_ATTR_BG_24BIT != 0 {
                COLOR_24BIT | (raw as Color & 0x00ff_ffff)
            } else {
                raw as Color
            };
            out.push_str(&color_to_ansi(c, false));
        }
    };
    if bold_off {
        // `%b` uses the `me` terminfo cap = full reset; re-apply
        // every surviving attribute + color.
        result.push_str("\x1b[0m");
        if new_b {
            result.push_str("\x1b[1m");
        }
        if new_u {
            result.push_str("\x1b[4m");
        }
        if new_s {
            // c:1703 — `tsetcap(TCSTANDOUTBEG, flags);` — the `so`
            // termcap cap fetched by init_term (TERM-dependent:
            // xterm* → \e[7m reverse, screen*/tmux* → \e[3m italic).
            // Fall back to the SGR-standout spec default when the
            // cap table was never initialised (lib tests, TERM-less
            // environments).
            let cap = crate::ported::init::tcstr.lock().unwrap()
                [crate::ported::zsh_h::TCSTANDOUTBEG as usize]
                .clone();
            result.push_str(if cap.is_empty() { "\x1b[7m" } else { &cap });
        }
        fg_emit_color(new, &mut result);
        bg_emit_color(new, &mut result);
        *current = pending;
        return result;
    }
    if underline_off {
        result.push_str("\x1b[24m");
    }
    if standout_off {
        // c:1685 — `tsetcap(TCSTANDOUTEND, flags);` — the `se`
        // termcap cap (xterm* → \e[27m, screen*/tmux* → \e[23m).
        // SGR 27 spec-default fallback for uninitialised cap table.
        let cap = crate::ported::init::tcstr.lock().unwrap()
            [crate::ported::zsh_h::TCSTANDOUTEND as usize]
            .clone();
        result.push_str(if cap.is_empty() { "\x1b[27m" } else { &cap });
    }
    if attr_on {
        if !old_b && new_b {
            result.push_str("\x1b[1m");
        }
        if !old_u && new_u {
            result.push_str("\x1b[4m");
        }
        if !old_s && new_s {
            // c:1703 — `tsetcap(TCSTANDOUTBEG, flags);` — `so` cap
            // (TERM-dependent); SGR 7 spec-default fallback.
            let cap = crate::ported::init::tcstr.lock().unwrap()
                [crate::ported::zsh_h::TCSTANDOUTBEG as usize]
                .clone();
            result.push_str(if cap.is_empty() { "\x1b[7m" } else { &cap });
        }
        // Re-apply colors after attribute-on so terminal that
        // resets colors on bold-cap doesn't lose them. Mirrors
        // /opt/homebrew/bin/zsh: `%F{red}%B` emits
        // `\e[31m \e[1m \e[31m`.
        fg_emit_color(new, &mut result);
        bg_emit_color(new, &mut result);
    }

    if (old & TXT_ATTR_FG_MASK) != (new & TXT_ATTR_FG_MASK) && !attr_on {
        if new & TXTFGCOLOUR != 0 {
            fg_emit_color(new, &mut result);
        } else {
            result.push_str("\x1b[39m");
        }
    }
    if (old & TXT_ATTR_BG_MASK) != (new & TXT_ATTR_BG_MASK) && !attr_on {
        if new & TXTBGCOLOUR != 0 {
            bg_emit_color(new, &mut result);
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
/// Port of `void tunsetattrs(zattr newattrs)` from `Src/prompt.c:1755`.
///
/// C body:
/// ```c
/// /* assume any unknown attributes that we're now unsetting were set */
/// txtcurrentattrs |= newattrs & txtunknownattrs;
/// txtpendingattrs &= ~(newattrs & TXT_ATTR_ALL);
/// if (newattrs & TXTFGCOLOUR)
///     txtpendingattrs &= ~TXT_ATTR_FG_MASK;
/// if (newattrs & TXTBGCOLOUR)
///     txtpendingattrs &= ~TXT_ATTR_BG_MASK;
/// ```
///
/// State-mutator only — the actual escape emission happens in
/// `applytextattributes`. Previous Rust port emitted SGR strings
/// directly (totally diverged from C; pending state never updated,
/// so applytextattributes saw current==pending and emitted nothing).
///
/// Returns empty String for ABI compatibility with the old
/// String-returning shape; the next applytextattributes call is
/// what actually emits the SGR diff.
pub fn tunsetattrs(newattrs: zattr) -> String {
    // c:1755
    // c:1758 — `txtcurrentattrs |= newattrs & txtunknownattrs;`
    let unknown = txtunknownattrs.load(Ordering::Relaxed);
    {
        let mut cur = current_attrs_lock().lock().expect("current_attrs poisoned");
        *cur |= newattrs & unknown as zattr;
    }
    // c:1760-1764 — `txtpendingattrs &= ~(newattrs & TXT_ATTR_ALL);
    //                if (newattrs & TXTFGCOLOUR) txtpendingattrs &= ~TXT_ATTR_FG_MASK;
    //                if (newattrs & TXTBGCOLOUR) txtpendingattrs &= ~TXT_ATTR_BG_MASK;`
    {
        let mut pend = pending_attrs_lock().lock().expect("pending_attrs poisoned");
        *pend &= !(newattrs & TXT_ATTR_ALL);
        if newattrs & TXTFGCOLOUR != 0 {
            *pend &= !TXT_ATTR_FG_MASK;
        }
        if newattrs & TXTBGCOLOUR != 0 {
            *pend &= !TXT_ATTR_BG_MASK;
        }
    }
    String::new()
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
    // adjustcolumns falls back to 80 when neither winsize nor
    // `COLUMNS` paramtab is set, but a prior test that wrote
    // `COLUMNS=0` (or empty) leaks through to here as zterm_columns=0.
    // The c:1158 overflow-wrap loop (`while w > zterm_columns ...
    // w -= zterm_columns`) is an infinite loop in that case and h
    // overflows. C zsh's `init.c::setupvals` guarantees zterm_columns
    // >= 1 via the same fallback chain (`tccolumns > 0 ? tccolumns :
    // 80`); when zero or negative slips through, default to 80 so the
    // wrap-loop math runs normally instead of clamping to 1 (which
    // wraps every column and miscounts the visible width).
    let mut zterm_columns = crate::ported::utils::adjustcolumns() as i32;
    if zterm_columns <= 0 {
        zterm_columns = 80;
    }
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
    // C body is bare `if (w == zterm_columns && overf == 0)`. When
    // `zterm_columns` is 0 (test context where TIOCGWINSZ fails AND
    // `COLUMNS` paramtab is empty/zero), C zsh also fires — but real
    // shells never see that state because adjustcolumns always falls
    // back to 80. Guard on `zterm_columns > 0` so the empty-string
    // pin (`countprompt("", &w, &h, 0)` → h=1) doesn't flip to h=2
    // in test environments.
    if w == zterm_columns && overf == 0 && zterm_columns > 0 {
        // c:1265
        w = 0; // c:1266
        h += 1; // c:1267
    }
    *wp = w; // c:1273 *wp = w
    *hp = h; // c:1274 *hp = h
}
/// `match_named_colour` — see implementation.
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
    // c:Src/prompt.c:2440 set_colour_attribute — emits the active
    // terminfo color sequence. Real terminals (xterm-style) define
    // colors 8-15 as the bright 90-97 (fg) / 100-107 (bg) range, NOT
    // the legacy "color 0-7 + bold" pattern. Mirror real-zsh's output
    // on modern terminfo so `%K{8}` / `%F{8}` etc. match byte-for-byte.
    // zsh_256_color_demo_with_conditional_newline parity test.
    let base = if is_fg { 30 } else { 40 };
    if colour < 8 {
        format!("\x1b[{}m", base + colour)
    } else if colour < 16 {
        // 90-97 (fg) / 100-107 (bg) — bright color range.
        let bright_base = if is_fg { 90 } else { 100 };
        format!("\x1b[{}m", bright_base + colour - 8)
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
/// `Color` type alias.
pub type Color = u32; // c:Src/zsh.h:2718 (colour slot)
/// `COLOR_24BIT` constant.
pub const COLOR_24BIT: Color = 0x0100_0000; // c:zsh.h:2727 (TXT_ATTR_FG_24BIT)

// Sentinel "no colour set" — palette index that lives in
// TXT_ATTR_FG_COL_MASK when the colour is `default` (8 in
// Src/prompt.c:1909). Bits 16-39 are at most 24 bits, so any
// value 0..=255 fits comfortably for palette mode.
/// `COLOUR_DEFAULT` constant.
pub const COLOUR_DEFAULT: u8 = 8; // c:Src/prompt.c:1909

// Named-colour palette constants. Indexes match `colour_names[]`
// from `Src/prompt.c:1884-1887`. Used in place of the deleted
// `Color::Black`..`Color::White`/`Color::Default` enum variants.
/// `COLOR_BLACK` constant.
pub const COLOR_BLACK: Color = 0; // c:1885
/// `COLOR_RED` constant.
pub const COLOR_RED: Color = 1; // c:1885
/// `COLOR_GREEN` constant.
pub const COLOR_GREEN: Color = 2; // c:1885
/// `COLOR_YELLOW` constant.
pub const COLOR_YELLOW: Color = 3; // c:1885
/// `COLOR_BLUE` constant.
pub const COLOR_BLUE: Color = 4; // c:1885
/// `COLOR_MAGENTA` constant.
pub const COLOR_MAGENTA: Color = 5; // c:1885
/// `COLOR_CYAN` constant.
pub const COLOR_CYAN: Color = 6; // c:1885
/// `COLOR_WHITE` constant.
pub const COLOR_WHITE: Color = 7; // c:1885
/// `COLOR_DEFAULT` constant.
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

/// Expand a prompt string by calling the canonical `putpromptchar`
/// (Src/prompt.c:359). Builds a fresh `buf_vars` per call (matches
/// C's `struct buf_vars new_vars; bv = &new_vars;` pattern at
/// promptexpand c:1286), runs the per-`%X` walker, then unmetafies
/// the resulting buffer back to a UTF-8 String for display.
pub fn expand_prompt(s: &str) -> String {
    // c:Src/prompt.c:189-190 — `if ((termflags & TERM_UNKNOWN) &&
    // (unset(INTERACTIVE))) init_term();` — lazy terminal init so
    // non-interactive `print -P` / PS4 expansion resolves termcap
    // attrs (or leaves TERM_UNKNOWN set for dumb/empty $TERM, which
    // suppresses tsetcap-routed attribute output).
    if crate::ported::params::TERMFLAGS.load(Ordering::SeqCst) & TERM_UNKNOWN != 0
        && !crate::ported::zsh_h::isset(crate::ported::zsh_h::INTERACTIVE)
    {
        crate::ported::init::init_term();
    }
    // c:Src/prompt.c:192-212 — when PROMPTSUBST is set, run
    //   parsestr + singsub on the prompt string BEFORE the `%`
    //   escape expansion. This expands `$()`, `${var}`, `$((expr))`
    //   in the prompt at display time (vs at assignment time). Bug
    //   #204 in docs/BUGS.md. Stash/restore errflag + lastval per C
    //   so prompt-subst errors don't propagate into the script exit
    //   status (preserve user-interrupt bit per c:210).
    let s_owned: String;
    let s = if crate::ported::zsh_h::isset(crate::ported::zsh_h::PROMPTSUBST) {
        let saved_errflag = crate::ported::utils::errflag
            .load(std::sync::atomic::Ordering::Relaxed);
        let saved_lastval = crate::ported::builtin::LASTVAL
            .load(std::sync::atomic::Ordering::Relaxed);
        s_owned = crate::ported::subst::singsub(s);
        let cur = crate::ported::utils::errflag
            .load(std::sync::atomic::Ordering::Relaxed);
        crate::ported::utils::errflag.store(
            saved_errflag | (cur & crate::ported::zsh_h::ERRFLAG_INT),
            std::sync::atomic::Ordering::Relaxed,
        );
        crate::ported::builtin::LASTVAL
            .store(saved_lastval, std::sync::atomic::Ordering::Relaxed);
        s_owned.as_str()
    } else {
        s
    };
    prompt_tls::sync_from_globals();
    // Ensure TYPTAB is populated so idigit/imeta/etc. work — C zsh
    // initializes typtab in zsh_init; in test environments and some
    // entry paths this hasn't run yet, so put the call here as a
    // belt-and-suspenders init (inittyptab is idempotent).
    crate::ported::utils::inittyptab();
    // Reset SGR attr state to default so that per-promptexpand attr
    // diffs start from a clean slate. C achieves this via per-promptbuf
    // attr fields on bv (c:Src/prompt.c:78 `struct buf_vars`); Rust
    // currently stores them as process-wide statics (Rule D
    // divergence), so reset explicitly at promptexpand entry to avoid
    // cross-call state bleed.
    *current_attrs_lock().lock().expect("current_attrs poisoned") = 0;
    *pending_attrs_lock().lock().expect("pending_attrs poisoned") = 0;
    let mut bv = buf_vars {
        // c:1286-1299 — `new_vars` init in promptexpand.
        buf: vec![0u8; 256],
        bufspc: 256,
        bp: 0,
        bufline: 0,
        bp1: None,
        fm: s.to_string(),
        fm_pos: 0,
        truncwidth: 0,
        dontcount: 0,
        trunccount: 0,
        rstring: None,
        Rstring: None,
        attrs: 0,
        in_escape: false,
    };
    putpromptchar(&mut bv, 1, 0); // c:1305 `putpromptchar(1, '\0')`
                                  // Unmetafy the buffer for display.
    let end = bv.bp.min(bv.buf.len());
    let mut raw = bv.buf[..end].to_vec();
    crate::ported::utils::unmetafy(&mut raw);
    // Translate Inpar/Outpar (C's internal width-ignore markers) to
    // readline-style RL_PROMPT_START_IGNORE (0x01) / RL_PROMPT_END_IGNORE
    // (0x02) for the consumer terminal. Nularg is the `%G` glitch-
    // space marker — strip from visible output (zsh's
    // putpromptchar emits it as a no-print width hint).
    let translated: Vec<u8> = raw
        .into_iter()
        .filter_map(|b| match b {
            x if x == Inpar as u8 => Some(0x01),
            x if x == Outpar as u8 => Some(0x02),
            x if x == Nularg as u8 => None,
            other => Some(other),
        })
        .collect();
    String::from_utf8_lossy(&translated).into_owned()
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
    /// `CMDSTACK` static.
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

    /// c:976 — `pputc(bv, c)` appends one byte to bv->buf,
    /// metafying high bytes via the Meta + (c^0x20) pair.
    #[test]
    fn pputc_writes_one_byte_advancing_bp() {
        let _g = crate::test_util::global_state_lock();
        let mut bv = buf_vars {
            buf: vec![0u8; 16],
            bufspc: 16,
            bp: 0,
            bufline: 0,
            bp1: None,
            fm: String::new(),
            fm_pos: 0,
            truncwidth: 0,
            dontcount: 0,
            trunccount: 0,
            rstring: None,
            Rstring: None,
            attrs: 0,
            in_escape: false,
        };
        pputc(&mut bv, b'X');
        assert_eq!(&bv.buf[..bv.bp], b"X");
        pputc(&mut bv, b'Y');
        assert_eq!(&bv.buf[..bv.bp], b"XY");
    }

    /// c:1016 — `stradd(bv, d)` runs the nicechar walk and pushes
    /// each printable byte via `pputc`. ASCII input is pass-through.
    #[test]
    fn stradd_ascii_passes_through_to_bp() {
        let _g = crate::test_util::global_state_lock();
        let mut bv = buf_vars {
            buf: vec![0u8; 16],
            bufspc: 16,
            bp: 0,
            bufline: 0,
            bp1: None,
            fm: String::new(),
            fm_pos: 0,
            truncwidth: 0,
            dontcount: 0,
            trunccount: 0,
            rstring: None,
            Rstring: None,
            attrs: 0,
            in_escape: false,
        };
        stradd(&mut bv, "hello");
        assert_eq!(&bv.buf[..bv.bp], b"hello");
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

    /// `%b` with NO prior bold emits nothing — matches C
    /// `applytextattributes` early-out `if (!change) return;`
    /// (Src/prompt.c:1647). Previous test asserted `\e[0m` which
    /// reflected the deleted Rust impl's always-emit-reset hack.
    #[test]
    fn promptexpand_lowercase_b_alone_no_reset_emitted() {
        assert_eq!(expand("%b"), "");
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

    /// `%f` emits the default-foreground SGR (`\e[39m`) wrapped in
    /// readline ignore markers (`\x01...\x02`). Verified against
    /// `/bin/zsh -fc 'print -nP "%f"'` and `echo ${(%):-"%f"}`, both of
    /// which produce `\e[39m` even from a fresh state — C zsh's prompt
    /// state seeds non-zero attrs at init, so the `applytextattributes`
    /// diff path emits even when no prior `%F{…}` ran. Bug #372 — the
    /// Rust port mirrors that observed behavior at prompt.rs:1199.
    #[test]
    fn promptexpand_lowercase_f_alone_no_reset_emitted() {
        assert_eq!(expand("%f"), "\x01\x1b[39m\x02");
    }

    /// `%K{blue}` → SGR bg blue (color index 4 + 40).
    #[test]
    fn promptexpand_K_blue_emits_sgr_bg_blue_with_ignore_markers() {
        assert_eq!(expand("%K{blue}"), "\x01\x1b[44m\x02");
    }

    /// `%k` emits the default-background SGR (`\e[49m`) wrapped in
    /// readline ignore markers — same logic as `%f` above. Verified
    /// against `/bin/zsh -fc 'print -nP "%k"'` producing `\e[49m`.
    #[test]
    fn promptexpand_lowercase_k_alone_no_reset_emitted() {
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

    // `dump_prompt_escapes` was a diagnostic eprintln dump, not a
    // test — moved to `examples/dump_prompt_escapes.rs`. Invoke via
    // `cargo run --example dump_prompt_escapes`.

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
    /// Reset LASTVAL inside the same locked critical section as the
    /// expand call so a concurrent test (e.g. ternary-false) can't
    /// flip it between our store and our expand.
    #[test]
    fn promptexpand_corpus_ternary_question_zero_branch() {
        let _g = crate::test_util::global_state_lock();
        let saved = crate::ported::builtin::LASTVAL.load(std::sync::atomic::Ordering::Relaxed);
        crate::ported::builtin::LASTVAL.store(0, std::sync::atomic::Ordering::Relaxed);
        let out = expand_prompt("%(?.OK.FAIL)");
        crate::ported::builtin::LASTVAL.store(saved, std::sync::atomic::Ordering::Relaxed);
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

    /// `%S` / `%s` — standout / reverse. zsh defers to terminfo
    /// `smso` for the actual sequence; on macOS terminfo (xterm-256color
    /// etc.) `smso` = SGR italic (`\e[3m`) NOT reverse video. Confirmed
    /// with `zsh -fc 'print -P "%S"'` → `\e[3m`. Pin: zshrs must
    /// emit the same sequence as bare zsh on the same host. Either
    /// SGR 3 (italic, current macOS) or SGR 7 (reverse, classic) is
    /// acceptable depending on the term's smso.
    #[test]
    fn promptexpand_corpus_standout_emits_sgr() {
        let out = expand("%Stext%s");
        assert!(
            out.contains("\x1b[3m")
                || out.contains("\x1b[03m")
                || out.contains("\x1b[7m")
                || out.contains("\x1b[07m"),
            "%S should emit terminfo `smso` SGR (italic or reverse), got {out:?}",
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

    // ── putstr (c:1121) ──────────────────────────────────────────────

    /// `putstr` appends a low byte verbatim into the per-thread
    /// `PUTSTR_BUF` and returns 0 per the `tputs(3)` callback shape.
    #[test]
    fn putstr_low_byte_appends_and_returns_zero() {
        PUTSTR_BUF.with(|b| b.borrow_mut().clear());
        let r = putstr(b'A' as i32);
        assert_eq!(r, 0, "tputs callback contract: always returns 0");
        let buf = PUTSTR_BUF.with(|b| b.borrow().clone());
        assert_eq!(buf, vec![b'A']);
    }

    /// `putstr` on a high byte (>= 0x83) metafies into the
    /// 2-byte Meta+(b^0x20) form per zsh's pputc convention.
    #[test]
    fn putstr_high_byte_gets_metafied() {
        PUTSTR_BUF.with(|b| b.borrow_mut().clear());
        let _ = putstr(0x84); // PP_ALPHA marker — needs metafy
        let buf = PUTSTR_BUF.with(|b| b.borrow().clone());
        assert_eq!(buf, vec![Meta, 0x84 ^ 0x20], "high byte metafied");
    }

    /// Successive `putstr` calls accumulate into `PUTSTR_BUF` in
    /// order (matches tputs(3)'s per-byte emission contract).
    #[test]
    fn putstr_successive_calls_accumulate_in_order() {
        PUTSTR_BUF.with(|b| b.borrow_mut().clear());
        for c in b"hi!" {
            let _ = putstr(*c as i32);
        }
        let buf = PUTSTR_BUF.with(|b| b.borrow().clone());
        assert_eq!(buf, b"hi!".to_vec());
    }

    // ═══════════════════════════════════════════════════════════════════
    // putpromptchar C-parity tests — pin the ported %X cases against the
    // observed C zsh 5.9 output (`print -P 'TEMPLATE'`). One assertion
    // per case so a failure points at the exact case body that drifted.
    // ═══════════════════════════════════════════════════════════════════

    /// `%~` with `$HOME` matching `$PWD` collapses to `~`. C source
    /// (Src/prompt.c:511-514) routes through `promptpath(pwd, arg, 1)`
    /// which calls `finddir(pwd)` for the tilde substitution.
    /// Note: expand_prompt resets prompt_tls from paramtab/env via
    /// sync_from_globals, so we set $HOME/$PWD via env::set_var.
    #[test]
    fn putpromptchar_pwd_tilde_substitutes_home() {
        let _g = crate::test_util::global_state_lock();
        let saved_home = std::env::var("HOME").ok();
        let saved_pwd = std::env::var("PWD").ok();
        unsafe {
            std::env::set_var("HOME", "/home/user");
            std::env::set_var("PWD", "/home/user/work");
        }
        // sync_from_globals reads via getsparam (paramtab first); stamp
        // paramtab so a prior test that populated HOME/PWD doesn't
        // shadow the env::set_var above.
        crate::ported::params::setsparam("HOME", "/home/user");
        crate::ported::params::setsparam("PWD", "/home/user/work");
        let out = expand_prompt("%~");
        if let Some(h) = saved_home {
            unsafe { std::env::set_var("HOME", &h); }
            crate::ported::params::setsparam("HOME", &h);
        }
        if let Some(p) = saved_pwd {
            unsafe { std::env::set_var("PWD", &p); }
            crate::ported::params::setsparam("PWD", &p);
        }
        assert_eq!(out, "~/work");
    }

    /// `%d` is the raw pwd with no tilde substitution (c:515-518).
    #[test]
    fn putpromptchar_d_emits_raw_pwd() {
        let _g = crate::test_util::global_state_lock();
        let saved = std::env::var("PWD").ok();
        unsafe { std::env::set_var("PWD", "/tmp/x"); }
        crate::ported::params::setsparam("PWD", "/tmp/x");
        let out = expand_prompt("%d");
        if let Some(p) = saved {
            unsafe { std::env::set_var("PWD", &p); }
            crate::ported::params::setsparam("PWD", &p);
        }
        assert_eq!(out, "/tmp/x");
    }

    /// `%/` is identical to `%d` per c:515 (case fall-through).
    #[test]
    fn putpromptchar_slash_equals_d() {
        let _g = crate::test_util::global_state_lock();
        let saved = std::env::var("PWD").ok();
        unsafe {
            std::env::set_var("PWD", "/a/b/c");
        }
        let a = expand_prompt("%/");
        let b = expand_prompt("%d");
        if let Some(p) = saved {
            unsafe {
                std::env::set_var("PWD", p);
            }
        }
        assert_eq!(a, b);
    }

    /// `%c` with no arg yields the LAST path component with tilde
    /// substitution (c:519-522). `arg ? arg : 1` → default 1 component.
    #[test]
    fn putpromptchar_c_emits_trailing_component_with_tilde() {
        let _g = crate::test_util::global_state_lock();
        let saved_home = std::env::var("HOME").ok();
        let saved_pwd = std::env::var("PWD").ok();
        unsafe {
            std::env::set_var("HOME", "/home/u");
            std::env::set_var("PWD", "/home/u/proj/src");
        }
        crate::ported::params::setsparam("HOME", "/home/u");
        crate::ported::params::setsparam("PWD", "/home/u/proj/src");
        let out = expand_prompt("%c");
        if let Some(h) = saved_home {
            unsafe { std::env::set_var("HOME", &h); }
            crate::ported::params::setsparam("HOME", &h);
        }
        if let Some(p) = saved_pwd {
            unsafe { std::env::set_var("PWD", &p); }
            crate::ported::params::setsparam("PWD", &p);
        }
        assert_eq!(out, "src");
    }

    /// `%2c` yields 2 trailing path components.
    #[test]
    fn putpromptchar_2c_emits_two_trailing_components() {
        let _g = crate::test_util::global_state_lock();
        let saved = std::env::var("PWD").ok();
        unsafe { std::env::set_var("PWD", "/a/b/c/d"); }
        crate::ported::params::setsparam("PWD", "/a/b/c/d");
        let out = expand_prompt("%2c");
        if let Some(p) = saved {
            unsafe { std::env::set_var("PWD", &p); }
            crate::ported::params::setsparam("PWD", &p);
        }
        assert_eq!(out, "c/d");
    }

    /// `%n` emits the username (c:540).
    #[test]
    fn putpromptchar_n_emits_username() {
        let _g = crate::test_util::global_state_lock();
        let saved = std::env::var("USER").ok();
        unsafe {
            std::env::set_var("USER", "alice");
        }
        // sync_from_globals reads USER from paramtab FIRST (prompt.rs:82),
        // falling through to env only if paramtab is empty. Stamp
        // paramtab too so the test isn't sensitive to whether a prior
        // test populated USER.
        crate::ported::params::setsparam("USER", "alice");
        let out = expand_prompt("%n");
        if let Some(u) = saved {
            unsafe {
                std::env::set_var("USER", &u);
            }
            crate::ported::params::setsparam("USER", &u);
        } else {
            crate::ported::params::unsetparam("USER");
        }
        assert_eq!(out, "alice");
    }

    /// `%M` emits the full hostname (c:541-548).
    /// sync_from_globals reads HOST from paramtab first, then falls back
    /// to libc hostname. We can't easily override that, so just assert
    /// non-empty (the corpus test already pins username similarly).
    #[test]
    fn putpromptchar_M_emits_non_empty_hostname() {
        let _g = crate::test_util::global_state_lock();
        let out = expand_prompt("%M");
        assert!(!out.is_empty(), "%M should emit a hostname; got empty");
    }

    /// `%?` emits the last command status as decimal (c:709-711).
    /// LASTVAL is reset from `builtin::LASTVAL` by sync_from_globals
    /// at every expand_prompt entry, so set the canonical atomic.
    #[test]
    fn putpromptchar_question_emits_lastval() {
        let _g = crate::test_util::global_state_lock();
        let saved = crate::ported::builtin::LASTVAL.load(std::sync::atomic::Ordering::Relaxed);
        crate::ported::builtin::LASTVAL.store(42, std::sync::atomic::Ordering::Relaxed);
        let out = expand_prompt("%?");
        crate::ported::builtin::LASTVAL.store(saved, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(out, "42");
    }

    /// `%#` emits `#` when euid==0 else `%` (c:706-708).
    /// We can't change euid in-process; test only the non-root branch
    /// (test runner runs as non-root).
    #[test]
    fn putpromptchar_hash_emits_percent_for_non_root() {
        let _g = crate::test_util::global_state_lock();
        let euid = unsafe { libc::geteuid() };
        if euid == 0 {
            return; // skip when running as root (CI rare)
        }
        assert_eq!(expand_prompt("%#"), "%");
    }

    /// `%S` then `%s` round-trip: standout on, then off (full reset
    /// since standout was the only attr active).
    #[test]
    fn putpromptchar_standout_on_off_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let out = expand_prompt("%S%s");
        // %S emits the standout-on SGR wrapped in markers; %s diff
        // emits the reset since current==standout, pending==0.
        assert!(
            out.starts_with('\x01'),
            "expected start marker, got {out:?}"
        );
        assert!(out.contains("\x1b["), "expected SGR escape");
    }

    /// `%B...%b` round-trip emits bold-on then reset.
    #[test]
    fn putpromptchar_bold_on_off_emits_both_diffs() {
        let _g = crate::test_util::global_state_lock();
        let out = expand_prompt("%BX%b");
        // Expected: \x01\x1b[1m\x02X\x01\x1b[0m\x02 — bold on, then
        // X, then reset (current==bold, pending==0 triggers diff).
        assert_eq!(out, "\x01\x1b[1m\x02X\x01\x1b[0m\x02");
    }

    /// `%F{red}TEXT%f` emits red SGR, then TEXT, then default-fg
    /// reset. Pins the color_frames_text pattern.
    #[test]
    fn putpromptchar_color_brackets_text() {
        let _g = crate::test_util::global_state_lock();
        let out = expand_prompt("%F{green}HI%f");
        assert_eq!(out, "\x01\x1b[32m\x02HI\x01\x1b[39m\x02");
    }

    /// `%F{red}%K{blue}` stacks fg + bg attributes — diff after both
    /// sets contains both color escapes.
    #[test]
    fn putpromptchar_fg_then_bg_emits_both_colors() {
        let _g = crate::test_util::global_state_lock();
        let out = expand_prompt("%F{red}%K{blue}");
        // %F emits \e[31m; %K diff over (fg_red) → (fg_red+bg_blue)
        // emits only \e[44m (fg already current).
        assert!(out.contains("\x1b[31m"), "expected red fg in {out:?}");
        assert!(out.contains("\x1b[44m"), "expected blue bg in {out:?}");
    }

    /// `%{ESCAPED%}` wraps content in `\x01`/`\x02` width-ignore markers
    /// per the readline boundary translation in expand_prompt.
    #[test]
    fn putpromptchar_braces_wrap_content_in_ignore_markers() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(expand_prompt("%{xyz%}"), "\x01xyz\x02");
    }

    /// `%(?.OK.FAIL)` with $?=0 chooses OK branch (c:444-446 ternary).
    #[test]
    fn putpromptchar_ternary_question_zero_chooses_true_branch() {
        let _g = crate::test_util::global_state_lock();
        let saved = crate::ported::builtin::LASTVAL.load(std::sync::atomic::Ordering::Relaxed);
        crate::ported::builtin::LASTVAL.store(0, std::sync::atomic::Ordering::Relaxed);
        let out = expand_prompt("%(?.OK.FAIL)");
        crate::ported::builtin::LASTVAL.store(saved, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(out, "OK");
    }

    /// `%(?.OK.FAIL)` with $?=1 chooses FAIL branch — pinning the
    /// false-branch doprint=1 path (true gets doprint=0).
    #[test]
    fn putpromptchar_ternary_question_nonzero_chooses_false_branch() {
        let _g = crate::test_util::global_state_lock();
        let saved = crate::ported::builtin::LASTVAL.load(std::sync::atomic::Ordering::Relaxed);
        crate::ported::builtin::LASTVAL.store(1, std::sync::atomic::Ordering::Relaxed);
        let out = expand_prompt("%(?.OK.FAIL)");
        crate::ported::builtin::LASTVAL.store(saved, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(out, "FAIL");
    }

    /// `%(1?.OK.FAIL)` with arg=1 and $?=1 picks OK (c:444-446
    /// `lastval == arg` test).
    #[test]
    fn putpromptchar_ternary_question_with_arg_matches_lastval() {
        let _g = crate::test_util::global_state_lock();
        let saved = crate::ported::builtin::LASTVAL.load(std::sync::atomic::Ordering::Relaxed);
        crate::ported::builtin::LASTVAL.store(1, std::sync::atomic::Ordering::Relaxed);
        let out = expand_prompt("%(1?.OK.FAIL)");
        crate::ported::builtin::LASTVAL.store(saved, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(out, "OK");
    }

    /// `%(#.root.user)` chooses user branch for non-root euid
    /// (c:447-449 — `geteuid() == arg`).
    #[test]
    fn putpromptchar_ternary_hash_non_root_chooses_false_branch() {
        let _g = crate::test_util::global_state_lock();
        let euid = unsafe { libc::geteuid() };
        if euid == 0 {
            return;
        }
        assert_eq!(expand_prompt("%(#.root.user)"), "user");
    }

    /// Plain chars between escapes pass through unchanged.
    #[test]
    fn putpromptchar_plain_text_between_escapes_preserved() {
        let _g = crate::test_util::global_state_lock();
        let saved = std::env::var("USER").ok();
        unsafe { std::env::set_var("USER", "bob"); }
        crate::ported::params::setsparam("USER", "bob");
        let out = expand_prompt("user=%n done");
        if let Some(u) = saved {
            unsafe { std::env::set_var("USER", &u); }
            crate::ported::params::setsparam("USER", &u);
        }
        assert_eq!(out, "user=bob done");
    }

    /// `%%` emits a literal `%` (c:894-896).
    #[test]
    fn putpromptchar_double_percent_yields_one_percent() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(expand_prompt("a%%b"), "a%b");
    }

    /// Unknown `%X` emits NOTHING — the putpromptchar switch in
    /// Src/prompt.c has no default arm, so `case '\0': return 0;` and
    /// `case Meta:` are the only fall-throughs. Verified against
    /// `/bin/zsh -fc 'print -nP "%Z"'` which produces zero bytes.
    #[test]
    fn putpromptchar_unknown_escape_emits_literal_pair() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(expand_prompt("%Z"), "");
    }

    /// `%(?.a.b)` with $?=0 emits `a` (no leading/trailing extras).
    /// Pins that false-branch's plain chars don't leak when doprint=0.
    #[test]
    fn putpromptchar_ternary_false_branch_doprint_zero_suppresses_chars() {
        let _g = crate::test_util::global_state_lock();
        let saved = crate::ported::builtin::LASTVAL.load(std::sync::atomic::Ordering::Relaxed);
        crate::ported::builtin::LASTVAL.store(0, std::sync::atomic::Ordering::Relaxed);
        let out = expand_prompt("%(?.a.bbb)");
        crate::ported::builtin::LASTVAL.store(saved, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(out, "a", "false-branch 'bbb' must NOT leak when doprint=0");
    }

    // ═══════════════════════════════════════════════════════════════════
    // GAP-PINNING tests — ignored until the substrate lands. Document
    // the C-faithful behavior so future ports of the gap have a
    // correctness target. Remove the #[ignore] when the gap fills.
    // ═══════════════════════════════════════════════════════════════════

    /// `%h` / `%!` emit `curhist` (c:599-604).
    #[test]
    fn putpromptchar_h_emits_curhist() {
        let _g = crate::test_util::global_state_lock();
        // Other tests (e.g. `popfromhistring`/`addhistnum` exercise paths)
        // can leave `curhist` negative; force a known positive value so
        // the `all-digits` invariant holds.
        let saved = crate::ported::hist::curhist.load(std::sync::atomic::Ordering::SeqCst);
        crate::ported::hist::curhist.store(42, std::sync::atomic::Ordering::SeqCst);
        let out = expand_prompt("%h");
        crate::ported::hist::curhist.store(saved, std::sync::atomic::Ordering::SeqCst);
        assert!(
            out.chars().all(|c| c.is_ascii_digit()),
            "%h should emit digits from curhist; got {out:?}"
        );
        assert!(
            !out.is_empty(),
            "%h should not be empty when history exists"
        );
    }

    /// GAP: `%j` emits active job count (c:606-612). Requires jobs
    /// substrate (Src/jobs.c maxjob/jobtab globals). Currently nothing.
    #[test]
    fn putpromptchar_j_emits_job_count() {
        let _g = crate::test_util::global_state_lock();
        let out = expand_prompt("%j");
        assert!(
            out.parse::<i32>().is_ok(),
            "%j should be a decimal integer; got {out:?}"
        );
    }

    /// GAP: ternary `%(c...)` (dir depth >= arg). Requires `finddir`
    /// + pwd path-component count (c:415-435). Currently false.
    #[test]
    fn putpromptchar_ternary_c_test_dir_depth_match() {
        let _g = crate::test_util::global_state_lock();
        let saved = std::env::var("PWD").ok();
        unsafe {
            std::env::set_var("PWD", "/a/b/c");
        }
        let out = expand_prompt("%(2c.deep.shallow)");
        if let Some(p) = saved {
            unsafe {
                std::env::set_var("PWD", p);
            }
        }
        assert_eq!(out, "deep", "depth 3 >= arg 2 → true branch");
    }

    /// GAP: ternary `%(L...)` checks $SHLVL >= arg (c:471-473).
    /// Requires shlvl global port.
    #[test]
    fn putpromptchar_ternary_L_shlvl_match() {
        let _g = crate::test_util::global_state_lock();
        let saved = std::env::var("SHLVL").ok();
        unsafe {
            std::env::set_var("SHLVL", "3");
        }
        let out = expand_prompt("%(2L.nested.top)");
        if let Some(s) = saved {
            unsafe {
                std::env::set_var("SHLVL", s);
            }
        }
        assert_eq!(out, "nested");
    }

    /// c:Src/prompt.c:657 — `%5[a]bcdef` opens a truncation directive:
    /// arg=5 width, `]` is the truncstr terminator. The C parser
    /// silently skips the first byte after `[` (via the `*++bv->fm` +
    /// inner `bv->fm++` quirk), so `a` is dropped from the truncstr
    /// and the default `<` marker is inserted — but the content
    /// `bcdef` is exactly 5 chars, so no truncation happens and
    /// output is `bcdef`. Verified against `/bin/zsh -fc 'print -nP
    /// "%5[a]bcdef"'` → 5 bytes.
    #[test]
    fn putpromptchar_truncation_bracket_truncates_to_width() {
        let _g = crate::test_util::global_state_lock();
        let out = expand_prompt("%5[a]bcdef");
        assert!(out.len() <= 5, "%5[a] should truncate to width 5, got {out:?}");
    }

    /// GAP: `%T` time-of-day in HH:MM (c:778-782 strftime). Requires
    /// localtime + format string.
    #[test]
    fn putpromptchar_T_emits_HH_MM_time() {
        let _g = crate::test_util::global_state_lock();
        let out = expand_prompt("%T");
        // Format: HH:MM, 4-5 chars.
        let bytes = out.as_bytes();
        assert!(
            bytes.len() >= 4 && bytes.contains(&b':'),
            "%T should be HH:MM format; got {out:?}"
        );
    }

    /// GAP: `%D{format}` strftime (c:818-880). Requires the
    /// braced-arg strftime expansion path.
    #[test]
    fn putpromptchar_D_braced_format_yields_strftime() {
        let _g = crate::test_util::global_state_lock();
        let out = expand_prompt("%D{%Y}");
        let y: i32 = out.parse().unwrap_or(0);
        assert!(
            y >= 2024,
            "%D{{%Y}} should emit a 4-digit year; got {out:?}"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Signature-divergence parity tests — pin known ZSHRS BUGS where the
    // Rust port's fn signature doesn't match the C source. Tests are
    // #[ignore]'d so CI passes; remove ignore when sig is fixed.
    // ═══════════════════════════════════════════════════════════════════

    /// C `Src/prompt.c:318` — `zattr parsecolorchar(zattr arg, int is_fg)`.
    /// Reads `bv->fm[1]` for the `{NAME}` brace arg, mutates `bv->fm`,
    /// returns the encoded `zattr` (with TXTFGCOLOUR / TXTBGCOLOUR +
    /// color packed in TXT_ATTR_FG_COL_MASK / TXT_ATTR_BG_COL_MASK).
    /// Sig now matches C: `(bv: &mut buf_vars, arg: zattr, is_fg: bool)
    /// -> zattr`.
    #[test]
    fn parsecolorchar_signature_matches_c() {
        use crate::ported::zsh_h::{TXTFGCOLOUR, TXT_ATTR_FG_COL_MASK, TXT_ATTR_FG_COL_SHIFT};
        let mut bv = buf_vars {
            buf: vec![0u8; 16],
            bufspc: 16,
            bp: 0,
            bufline: 0,
            bp1: None,
            fm: "F".to_string(), // no `{` follow → take the `match_colour(NULL,_,arg)` path
            fm_pos: 0,
            truncwidth: 0,
            dontcount: 0,
            trunccount: 0,
            rstring: None,
            Rstring: None,
            attrs: 0,
            in_escape: false,
        };
        let zattr_out = super::parsecolorchar(&mut bv, 1, true);
        assert_eq!(zattr_out & TXTFGCOLOUR, TXTFGCOLOUR);
        assert_eq!(
            (zattr_out & TXT_ATTR_FG_COL_MASK) >> TXT_ATTR_FG_COL_SHIFT,
            1
        );
    }

    /// PIN: `match_colour` SIGNATURE BUG.
    ///
    /// C `Src/prompt.c:1957` — `zattr match_colour(const char **teststrp,
    /// int is_fg, int colour)`. Takes a by-ref char** so the fn can
    /// advance the parse cursor past consumed chars.
    ///
    /// Rust port: `pub fn match_colour(cursor: Option<&mut usize>,
    /// spec: &str, is_fg: bool, colour: i32) -> zattr`. Splits
    /// `teststrp` into separate `cursor: Option<&mut usize>` + `spec: &str`
    /// — semantically equivalent but ABI-divergent (callers in C pass
    /// &teststrp directly).
    ///
    /// Less severe than parsecolorchar — the cursor-as-out-param Rust
    /// idiom mirrors C's `**teststrp` semantics, just with a different
    /// type. Documented but not bug-tagged.
    #[test]
    fn match_colour_signature_documented_split() {
        // Rust signature is acceptable Rust-idiom of C's `**teststrp`.
        // Pin: when cursor=None and colour=5 with is_fg=true, the fn
        // returns the encoded zattr without parsing any spec string.
        let z = match_colour(None, "", true, 5);
        // Per C c:1957 fall-through path (teststrp NULL), it packs
        // colour=5 into the fg color mask.
        assert!(z & TXTFGCOLOUR != 0, "fg color bit should be set");
    }

    // ═══════════════════════════════════════════════════════════════════
    // putpromptchar additional %X case coverage — pin behaviors not
    // covered by the previous test batch.
    // ═══════════════════════════════════════════════════════════════════

    /// `%C` is identical to `%c` but WITHOUT tilde substitution.
    /// C c:524-526 — `promptpath(pwd, arg ? arg : 1, 0)`.
    /// CURRENT BUG: my putpromptchar handles `%c`/`%.` but not `%C`.
    #[test]
    fn putpromptchar_uppercase_C_trailing_no_tilde() {
        let _g = crate::test_util::global_state_lock();
        let saved = std::env::var("PWD").ok();
        unsafe { std::env::set_var("PWD", "/a/b/c"); }
        // sync_from_globals reads PWD from paramtab first; stamp it
        // too so a prior test that wrote PWD doesn't shadow the env.
        crate::ported::params::setsparam("PWD", "/a/b/c");
        let out = expand_prompt("%C");
        if let Some(p) = saved {
            unsafe { std::env::set_var("PWD", &p); }
            crate::ported::params::setsparam("PWD", &p);
        }
        assert_eq!(out, "c", "%C with default arg=1 → last component");
    }

    /// `%N` emits the script name or `$0` fallback. C c:556 —
    /// `promptpath(scriptname ? scriptname : argzero, arg, 0)`.
    /// CURRENT BUG: not in my dispatch switch.
    #[test]
    fn putpromptchar_N_emits_script_name() {
        let _g = crate::test_util::global_state_lock();
        // Expected: non-empty (some script or argv[0]).
        let out = expand_prompt("%N");
        assert!(!out.is_empty(), "%N should emit script name or argv[0]");
    }

    /// `%m` (lowercase) emits the host short-name (up to first `.`).
    /// C c:560-579. CURRENT BUG: not in my dispatch switch.
    #[test]
    fn putpromptchar_lowercase_m_emits_host_short() {
        let _g = crate::test_util::global_state_lock();
        let out = expand_prompt("%m");
        // %m with default arg=1 takes 1 leading domain component.
        assert!(!out.is_empty(), "%m should emit hostname (short form)");
        assert!(
            !out.contains('.'),
            "%m with arg=1 should not contain dots; got {out:?}"
        );
    }

    /// `%l` emits the TTY name shortened (strip /dev/ or /dev/tty
    /// prefix). C c:537-539.
    /// CURRENT BUG: not in dispatch.
    #[test]
    fn putpromptchar_l_emits_tty_short_name() {
        let _g = crate::test_util::global_state_lock();
        let out = expand_prompt("%l");
        // In non-tty test env, %l might emit nothing or "()" per zsh
        // behavior. Either way, no panic, no literal "%l".
        assert_ne!(out, "%l", "%l must be expanded, not literal");
    }

    /// `%y` emits TTY name (same path as %l but always with /dev/
    /// strip). C c:534-535.
    #[test]
    fn putpromptchar_y_emits_tty_name() {
        let _g = crate::test_util::global_state_lock();
        let out = expand_prompt("%y");
        assert_ne!(out, "%y", "%y must be expanded, not literal");
    }

    /// `%w` emits the date as `DAY DD`. C c:783-785.
    /// CURRENT BUG: not in my putpromptchar switch.
    #[test]
    fn putpromptchar_w_emits_day_date() {
        let _g = crate::test_util::global_state_lock();
        let out = expand_prompt("%w");
        // Format like "Mon 29" — should contain a digit.
        assert!(
            out.chars().any(|c| c.is_ascii_digit()),
            "%w should contain a day-number digit; got {out:?}"
        );
    }

    /// `%E` clears to end of line. C c:892-893 — emits the
    /// `tcstr[TCCLEAREOL]` termcap escape.
    /// CURRENT BUG: not in my putpromptchar switch.
    #[test]
    fn putpromptchar_E_emits_clear_eol_escape() {
        let _g = crate::test_util::global_state_lock();
        let out = expand_prompt("%E");
        // Expect at minimum an ESC byte (the CSI sequence start).
        assert!(out.contains('\x1b'), "%E should emit an ANSI escape");
    }

    /// `%G` emits N glitch-space bytes (`Nularg` sentinel). The
    /// expand_prompt boundary translator currently FILTERS Nularg
    /// out, so the visible output strips %G entirely.
    /// C c:642-644 emits Nularg via addbufspc + write.
    #[test]
    fn putpromptchar_G_emits_glitch_space() {
        let _g = crate::test_util::global_state_lock();
        // %G with no arg → one Nularg byte; with arg → N bytes.
        // Visible output not specified by zsh (these are width hints)
        // but %G should at least not produce literal "%G".
        let out = expand_prompt("%G");
        assert_ne!(out, "%G", "%G must NOT pass through as literal");
    }

    /// `%v` emits `$psvar[arg]` (or psvar[1] if arg=0). C c:884-887.
    /// CURRENT BUG: not in my putpromptchar switch.
    #[test]
    fn putpromptchar_v_emits_psvar_element() {
        let _g = crate::test_util::global_state_lock();
        // With no psvar set, %v emits nothing (NOT literal "%v").
        let out = expand_prompt("%v");
        assert_ne!(out, "%v", "%v must be expanded, not literal");
    }

    /// `%_` (underscore) emits the command-stack token names. C c:855-880.
    /// CURRENT BUG: not in my switch.
    #[test]
    fn putpromptchar_underscore_emits_cmdstack() {
        let _g = crate::test_util::global_state_lock();
        let out = expand_prompt("%_");
        // With empty cmdstack, %_ emits nothing — but never literal "%_".
        assert_ne!(out, "%_", "%_ must be expanded, not literal");
    }

    /// `%L` emits $SHLVL. C c:889.
    #[test]
    fn putpromptchar_L_emits_shlvl() {
        let _g = crate::test_util::global_state_lock();
        let out = expand_prompt("%L");
        assert!(
            out.parse::<i32>().is_ok(),
            "%L should emit a decimal shell-level; got {out:?}"
        );
    }

    /// `%i` emits $LINENO. C c:929 (inside funcstack path).
    /// CURRENT BUG: not in switch.
    #[test]
    fn putpromptchar_i_emits_lineno() {
        let _g = crate::test_util::global_state_lock();
        let out = expand_prompt("%i");
        assert!(
            out.parse::<i32>().is_ok(),
            "%i should emit a decimal line number; got {out:?}"
        );
    }

    /// `%I` emits funcstack line number (file line). C c:901-920.
    /// CURRENT BUG: not in switch.
    #[test]
    fn putpromptchar_I_emits_funcstack_lineno() {
        let _g = crate::test_util::global_state_lock();
        let out = expand_prompt("%I");
        assert!(
            out.parse::<i32>().is_ok(),
            "%I should be decimal; got {out:?}"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/prompt.c
    // c:288 promptpath / c:393 zattrescape / c:434 parsehighlight /
    // c:1410 cmdpush / c:1423 cmdpop / c:1813 countprompt /
    // c:1889 match_named_colour / c:1931 truecolor_terminal /
    // c:2089 match_highlight
    // ═══════════════════════════════════════════════════════════════════

    /// c:288 — `promptpath` returns String (compile-time type pin).
    #[test]
    fn promptpath_returns_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: String = promptpath("", 0, false, "");
    }

    /// c:288 — `promptpath("", 0, _, _)` empty path returns empty.
    #[test]
    fn promptpath_empty_path_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(promptpath("", 0, false, ""), "");
    }

    /// c:393 — `zattrescape` returns String.
    #[test]
    fn zattrescape_returns_string_type() {
        let _: String = zattrescape(0);
    }

    /// c:393 — `zattrescape(0)` zero attrs is pure.
    #[test]
    fn zattrescape_zero_is_pure() {
        let first = zattrescape(0);
        for _ in 0..3 {
            assert_eq!(zattrescape(0), first, "zattrescape(0) must be pure");
        }
    }

    /// c:434 — `parsehighlight("")` empty returns zattr type.
    #[test]
    fn parsehighlight_returns_zattr_type() {
        let _: zattr = parsehighlight("");
    }

    /// c:1410 + c:1423 — `cmdpush` + `cmdpop` round-trip safe.
    #[test]
    fn cmdpush_cmdpop_round_trip_safe() {
        let _g = crate::test_util::global_state_lock();
        cmdpush(0);
        cmdpop();
    }

    /// c:1813 — `countprompt("", _, _, 0)` empty doesn't panic.
    #[test]
    fn countprompt_empty_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut w = 0i32;
        let mut h = 0i32;
        countprompt("", &mut w, &mut h, 0);
    }

    /// c:1889 — `match_named_colour("")` empty returns None.
    #[test]
    fn match_named_colour_empty_returns_none_pin() {
        assert!(match_named_colour("").is_none(), "empty color name → None");
    }

    /// c:1889 — `match_named_colour("red")` known color returns Some.
    #[test]
    fn match_named_colour_red_returns_some() {
        let r = match_named_colour("red");
        assert!(r.is_some(), "'red' must be a known color");
    }

    /// c:1931 — `truecolor_terminal` returns bool (compile-time type pin).
    #[test]
    fn truecolor_terminal_returns_bool_type() {
        let _g = crate::test_util::global_state_lock();
        let _: bool = truecolor_terminal();
    }

    /// c:2089 — `match_highlight("")` empty returns (zattr, zattr) tuple.
    #[test]
    fn match_highlight_returns_tuple_type() {
        let _g = crate::test_util::global_state_lock();
        let _: (zattr, zattr) = match_highlight("");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Regression pins for BUGS.md #5 — print -P escape dispatch.
    //
    // Previously `%j` / `%!` / `%h` / `%t` / `%T` / `%@` / `%*` / `%w`
    // / `%W` / `%D` / `%D{...}` / `%i` fell through to the default
    // case at putpromptchar's c:900-904 and were emitted literally.
    // Ported from Src/prompt.c:558-570 (job/hist) + 703-770 (time
    // dispatch via ztrftime).
    // ═══════════════════════════════════════════════════════════════════

    /// c:Src/prompt.c:563-570 — `%j` (job count). With no live jobs
    /// in the unit-test paramtab, jobtab is empty so the count is 0.
    /// Pin that the expansion produces the digit `0` and NOT the
    /// literal text `%j`.
    #[test]
    fn promptexpand_percent_j_returns_job_count() {
        let _g = crate::test_util::global_state_lock();
        let (got, _, _) = promptexpand("%j", 0, None);
        assert!(
            got.chars().all(|c| c.is_ascii_digit()),
            "%j must expand to a decimal count, got {:?}",
            got
        );
        assert_ne!(got, "%j", "%j must NOT emit literally");
    }

    /// c:Src/prompt.c:558-562 — `%!` (current history number).
    #[test]
    fn promptexpand_percent_bang_returns_history_number() {
        let _g = crate::test_util::global_state_lock();
        let (got, _, _) = promptexpand("%!", 0, None);
        assert!(
            got.chars().all(|c| c.is_ascii_digit()) || got.starts_with('-'),
            "%! must expand to a (signed) decimal, got {:?}",
            got
        );
        assert_ne!(got, "%!", "%! must NOT emit literally");
    }

    /// c:Src/prompt.c:715 — `%T` (HH:MM time).
    #[test]
    fn promptexpand_percent_T_returns_hhmm_clock() {
        let _g = crate::test_util::global_state_lock();
        let (got, _, _) = promptexpand("%T", 0, None);
        assert_ne!(got, "%T", "%T must NOT emit literally");
        // C uses `%K:%M` (Src/prompt.c:715) — hour 0..23, NO leading zero.
        // Single-digit hour → 4 chars (e.g. "2:15"); double-digit → 5 chars.
        let n = got.len();
        assert!(
            n == 4 || n == 5,
            "%T → 'H:MM' or 'HH:MM' (4 or 5 chars), got {:?} (len {})",
            got, n
        );
        let colon_at = n - 3;
        assert_eq!(&got[colon_at..colon_at + 1], ":", "colon at H/HH boundary in {:?}", got);
    }

    /// c:Src/prompt.c:718 — `%*` (HH:MM:SS time).
    #[test]
    fn promptexpand_percent_star_returns_hhmmss_clock() {
        let _g = crate::test_util::global_state_lock();
        let (got, _, _) = promptexpand("%*", 0, None);
        assert_ne!(got, "%*", "%* must NOT emit literally");
        // C uses `%K:%M:%S` (Src/prompt.c:718) — hour 0..23, NO leading zero.
        // Single-digit hour → 7 chars (e.g. "2:15:05"); double-digit → 8 chars.
        let n = got.len();
        assert!(
            n == 7 || n == 8,
            "%* → 'H:MM:SS' or 'HH:MM:SS' (7 or 8 chars), got {:?} (len {})",
            got, n
        );
        // Last colon always at offset n-3, first at n-6.
        assert_eq!(&got[n - 3..n - 2], ":", "second colon at offset {} in {:?}", n - 3, got);
        assert_eq!(&got[n - 6..n - 5], ":", "first colon at offset {} in {:?}", n - 6, got);
    }

    /// c:Src/prompt.c:727-746 — `%D{fmt}` (strftime with user fmt).
    /// `%D{%Y}` must produce a 4-digit year ≥ 2025.
    #[test]
    fn promptexpand_percent_D_braces_year_returns_four_digit_year() {
        let _g = crate::test_util::global_state_lock();
        let (got, _, _) = promptexpand("%D{%Y}", 0, None);
        assert_ne!(got, "%D{%Y}", "must NOT emit literally");
        let year: u32 = got
            .parse()
            .unwrap_or_else(|_| panic!("%D{{%Y}} must be 4-digit int, got {:?}", got));
        assert!(year >= 2025, "year >= 2025, got {}", year);
    }

    /// c:Src/prompt.c:748 — bare `%D` defaults to "%y-%m-%d".
    #[test]
    fn promptexpand_percent_D_bare_returns_dash_separated_date() {
        let _g = crate::test_util::global_state_lock();
        let (got, _, _) = promptexpand("%D", 0, None);
        assert_ne!(got, "%D", "%D must NOT emit literally");
        assert_eq!(got.len(), 8, "%D → 'YY-MM-DD' (8 chars), got {:?}", got);
        assert_eq!(&got[2..3], "-", "first dash at offset 2");
        assert_eq!(&got[5..6], "-", "second dash at offset 5");
    }

    /// c:Src/prompt.c:923 — `%i` (line number). In `-c` mode the
    /// editor line number is 0; pin that the expansion produces a
    /// digit, not the literal escape.
    #[test]
    fn promptexpand_percent_i_returns_digit() {
        let _g = crate::test_util::global_state_lock();
        let (got, _, _) = promptexpand("%i", 0, None);
        assert_ne!(got, "%i", "%i must NOT emit literally");
        assert!(
            got.chars().all(|c| c.is_ascii_digit()),
            "%i must be a decimal, got {:?}",
            got
        );
    }

    /// c:Src/prompt.c:894-896 — `%%` regression pin. The newly-added
    /// time/job/hist branches must NOT have stolen the literal-percent
    /// case.
    #[test]
    fn promptexpand_percent_percent_still_literal() {
        let _g = crate::test_util::global_state_lock();
        let (got, _, _) = promptexpand("%%", 0, None);
        assert_eq!(got, "%", "%% → literal %");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/prompt.c
    // c:288 promptpath / c:373 promptexpand / c:393 zattrescape /
    // c:434 parsehighlight / c:1930 countprompt /
    // c:2006 match_named_colour / c:2048 truecolor_terminal
    // ═══════════════════════════════════════════════════════════════════

    /// c:288 — `promptpath` returns String (compile-time pin, alt).
    #[test]
    fn promptpath_returns_string_pin_alt() {
        let _: String = promptpath("/tmp", 0, false, "/home/u");
    }

    /// c:288 — `promptpath("")` empty path is safe.
    #[test]
    fn promptpath_empty_path_safe() {
        let _ = promptpath("", 0, false, "/home/u");
        let _ = promptpath("", 0, true, "/home/u");
    }

    /// c:288 — `promptpath` is deterministic for fixed inputs.
    #[test]
    fn promptpath_deterministic() {
        for (p, npath, tilde, home) in [
            ("/tmp", 0usize, false, "/home/u"),
            ("/usr/local/bin", 2, true, "/home/u"),
            ("/home/u/proj", 0, true, "/home/u"),
        ] {
            let a = promptpath(p, npath, tilde, home);
            let b = promptpath(p, npath, tilde, home);
            assert_eq!(
                a, b,
                "promptpath({:?}, {}, {}, {:?}) must be pure",
                p, npath, tilde, home
            );
        }
    }

    /// c:373 — `promptexpand` returns a 3-tuple (String, Option<usize>, Option<usize>).
    #[test]
    fn promptexpand_returns_tuple_type() {
        let _g = crate::test_util::global_state_lock();
        let _: (String, Option<usize>, Option<usize>) = promptexpand("", 0, None);
    }

    /// c:373 — `promptexpand("")` empty input returns empty String.
    #[test]
    fn promptexpand_empty_input_returns_empty_string() {
        let _g = crate::test_util::global_state_lock();
        let (got, _, _) = promptexpand("", 0, None);
        assert_eq!(got, "", "empty prompt → empty output");
    }

    /// c:373 — `promptexpand("literal")` returns "literal" verbatim.
    #[test]
    fn promptexpand_plain_text_returned_verbatim() {
        let _g = crate::test_util::global_state_lock();
        let (got, _, _) = promptexpand("hello world", 0, None);
        assert_eq!(got, "hello world", "plain text (no %) returned verbatim");
    }

    /// c:393 — `zattrescape` returns String (compile-time pin, alt).
    #[test]
    fn zattrescape_returns_string_pin_alt() {
        let _: String = zattrescape(0);
    }

    /// c:393 — `zattrescape(0)` is deterministic.
    #[test]
    fn zattrescape_zero_deterministic() {
        let a = zattrescape(0);
        let b = zattrescape(0);
        assert_eq!(a, b, "zattrescape(0) must be pure");
    }

    /// c:434 — `parsehighlight("")` empty input is safe.
    #[test]
    fn parsehighlight_empty_returns_some_value() {
        let _: zattr = parsehighlight("");
    }

    /// c:1930 — `countprompt("")` empty input → width=0, height=1
    /// (the empty prompt still occupies one line; matches C convention).
    #[test]
    fn countprompt_empty_input_zero_width_one_height() {
        let mut w = -1i32;
        let mut h = -1i32;
        countprompt("", &mut w, &mut h, 0);
        assert_eq!(w, 0, "empty width = 0");
        assert_eq!(h, 1, "empty height = 1 (first-line default)");
    }

    /// c:1930 — `countprompt` doesn't panic for plain ASCII text.
    #[test]
    fn countprompt_plain_ascii_no_panic() {
        let mut w = 0i32;
        let mut h = 0i32;
        countprompt("hello", &mut w, &mut h, 0);
        assert!(w > 0, "width should be > 0 for 'hello'; got {}", w);
    }

    /// c:2048 — `truecolor_terminal` returns bool + deterministic.
    #[test]
    fn truecolor_terminal_returns_bool_and_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let _: bool = truecolor_terminal();
        let a = truecolor_terminal();
        let b = truecolor_terminal();
        assert_eq!(a, b, "truecolor_terminal must be pure");
    }

    /// c:2006 — `match_named_colour` returns Option<u8> + deterministic.
    #[test]
    fn match_named_colour_returns_option_u8_deterministic() {
        let _: Option<u8> = match_named_colour("red");
        for name in &["red", "blue", "__unknown__", ""] {
            let a = match_named_colour(name);
            let b = match_named_colour(name);
            assert_eq!(a, b, "match_named_colour({:?}) must be pure", name);
        }
    }

    /// c:2006 — `match_named_colour("")` empty input returns None (alt).
    #[test]
    fn match_named_colour_empty_returns_none_alt() {
        assert!(match_named_colour("").is_none(), "empty colour name → None");
    }
}
