//! Faithful Rust ports of free functions and file-static globals from
//! `Src/exec.c`. The wordcode-VM dispatch tree (`execlist` / `execpline`
//! / `execcmd` / `execsimple` etc.) that drives execution in C zsh is
//! NOT replicated here — zshrs runs the fusevm bytecode VM instead
//! (see `src/vm_helper.rs` + `src/fusevm_bridge.rs`).
//!
//! What lives here are the parts of `Src/exec.c` that ARE faithful
//! ports and don't depend on the C-side wordcode walker:
//!
//! - **`trap_state` / `trap_return` / `forklevel`** — file-static
//!   integer globals from `Src/exec.c:134 / :155 / :1052`, exposed as
//!   atomics shared between this module, `Src/signals.c`'s port at
//!   `src/ported/signals.rs`, and `Src/params.c`'s port at
//!   `src/ported/params.rs`.
//! - **`gethere`** (`Src/exec.c:4573`) — turn a here-document into a
//!   here-string. Called from the lexer port (`src/ported/lex.rs`).
//! - **`getoutput`** (`Src/exec.c:4712`) — command-substitution body
//!   runner. Called from the parameter-expansion port
//!   (`src/ported/subst.rs`).
//! - **`loadautofn`** + **`getfpfunc`** (`Src/exec.c:5050` / `:5260`)
//!   — `$fpath` walker + autoload file installer. Called from
//!   `bin_autoload` / `bin_functions -c` in `src/ported/builtin.rs`.
//! - **`resolvebuiltin`** (`Src/exec.c:2703`) — module-autoload guard
//!   used by the dispatch walk in `execcmd_exec`.
//! - **`execcmd_exec`** (`Src/exec.c:2900`) — head section only
//!   (locals + precommand-modifier walk through `c:3091`). The rest
//!   of the C function drives the wordcode-walker dispatch and is
//!   replaced by fusevm bytecode in `src/extensions/compile_zsh.rs`;
//!   see the WARNING block inside the function body.

use std::sync::atomic::Ordering;

use crate::fusevm_bridge::with_executor;
use crate::ported::utils::{errflag, ERRFLAG_ERROR};
use crate::ported::zsh_h::{
    BINF_BUILTIN, BINF_COMMAND, BINF_EXEC, BINF_PREFIX, IS_DASH, PM_UNDEFINED, WC_SIMPLE,
    WC_TYPESET,
};

/// Port of `int trap_state;` from `Src/exec.c:134`. Tracks whether
/// a trap handler is currently being processed and, paired with
/// `TRAP_RETURN` below, whether a `return` inside the trap should
/// promote to `TRAP_STATE_FORCE_RETURN` to unwind the trap caller.
///
/// Values: `TRAP_STATE_INACTIVE = 0`, `TRAP_STATE_PRIMED = 1`,
/// `TRAP_STATE_FORCE_RETURN = 2` (see `Src/zsh.h`).
pub static TRAP_STATE: std::sync::atomic::AtomicI32 = // c:134 (Src/exec.c)
    std::sync::atomic::AtomicI32::new(0);

/// Port of `int trap_return;` from `Src/exec.c:155`. Carries the
/// pending exit status from inside a trap; sentinel `-2` means
/// "running an EXIT/DEBUG-style trap at the current level"
/// (signals.c:1166). Promoted to the user's `return N` value by
/// `bin_return` when POSIX-trap semantics apply (builtin.c:5852).
pub static TRAP_RETURN: std::sync::atomic::AtomicI32 = // c:155 (Src/exec.c)
    std::sync::atomic::AtomicI32::new(0);

/// Port of `int forklevel;` from `Src/exec.c:1052`. Records the
/// `locallevel` at the most recent fork point (set at c:1221:
/// `forklevel = locallevel;` inside `entersubsh()`). Used by:
///   - `signals.c:808` SIGPIPE handler — `!forklevel` distinguishes
///     the top-level shell from a forked subshell.
///   - `exec.c:6146` — `if (locallevel > forklevel)` decides whether
///     a function-defined trap should fire on this subshell exit.
///   - `params.c:3724` — WARNCREATEGLOBAL nest-depth check.
///
/// Initialised to 0 (no fork has occurred yet). Set to `locallevel`
/// at every `entersubsh()` entry per c:1221.
pub static FORKLEVEL: std::sync::atomic::AtomicI32 = // c:1052 (Src/exec.c)
    std::sync::atomic::AtomicI32::new(0);

/// Convert a here-document into a here-string. Line-by-line port of
/// `gethere()` from `Src/exec.c:4569-4652`. Reads the body from the
/// input stream via `hgetc()` until the terminator line is matched,
/// returning the collected body as a string. `strp` is in/out: on
/// entry the raw terminator (possibly with token markers + leading
/// tabs); on return the munged terminator (after `quotesubst` +
/// `untokenize` and, for `REDIR_HEREDOCDASH`, leading-tab strip).
///
/// Returns `None` on out-of-memory (C `zalloc`/`realloc` failure).
/// Rust's `String` auto-grows so the OOM branch is effectively
/// unreachable, but the return type stays `Option<String>` to mirror
/// the C signature which can return NULL.
///
/// Port of `gethere(char **strp, int typ)` from `Src/exec.c:4573`.
pub fn gethere(strp: &mut String, typ: i32) -> Option<String> {
    // c:4573 (Src/exec.c)
    let mut buf: String; // c:4575 char *buf
    let mut bsiz: usize; // c:4576 int bsiz
    let mut qt: i32 = 0; // c:4576 int qt = 0
    let mut strip: i32 = 0; // c:4576 int strip = 0
                            // c:4577 — char *s, *t, *bptr, c. zshrs uses byte-offsets into
                            // `buf` for `t` and tracks `bptr` implicitly as `buf.len()` (the
                            // C `bptr++` increment is `buf.push(c)`; `bptr--` is `buf.pop()`).
                            // `s` (the loop iterator for the inull-scan) stays local to its
                            // for-loop. `c` mirrors the C `char c`.
    let mut t: usize; // c:4577 char *t
    let mut c: Option<char>; // c:4577 char c
    let mut str: String = strp.clone(); // c:4578 char *str = *strp

    // c:4580-4584 — for (s = str; *s; s++) if (inull(*s)) { qt = 1; break; }
    for s in str.bytes() {
        if crate::ported::ztype_h::inull(s) {
            // c:4581
            qt = 1; // c:4582
            break; // c:4583
        }
    }
    str = crate::ported::subst::quotesubst(&str); // c:4585
    str = crate::ported::lex::untokenize(&str); // c:4586
    if typ == crate::ported::zsh_h::REDIR_HEREDOCDASH {
        // c:4587
        strip = 1; // c:4588
                   // c:4589-4590 — while (*str == '\t') str++;
        while str.starts_with('\t') {
            str.remove(0);
        }
    }
    *strp = str.clone(); // c:4592 *strp = str

    // c:4593 — bptr = buf = zalloc(bsiz = 256);
    bsiz = 256;
    buf = String::with_capacity(bsiz);
    let _ = bsiz; // bsiz is tracked by C for zfree; Rust drops automatically

    // c:4594 — for (;;)
    loop {
        t = buf.len(); // c:4595 t = bptr

        // c:4597-4598 — while ((c = hgetc()) == '\t' && strip) ;
        loop {
            c = crate::ported::lex::hgetc();
            if !(c == Some('\t') && strip != 0) {
                break;
            }
        }

        // c:4599 — for (;;) — inner body-read loop
        loop {
            // c:4600-4613 — buffer-growth realloc dance. Rust's
            // String auto-grows; nothing to do.
            // c:4614 — if (lexstop || c == '\n') break;
            if crate::ported::lex::LEX_LEXSTOP.with(|f| f.get()) || c == Some('\n') || c.is_none() {
                break;
            }
            // c:4616 — if (!qt && c == '\\')
            if qt == 0 && c == Some('\\') {
                buf.push('\\'); // c:4617 *bptr++ = c
                c = crate::ported::lex::hgetc(); // c:4618
                if c == Some('\n') {
                    // c:4619
                    buf.pop(); // c:4620 bptr--
                    c = crate::ported::lex::hgetc(); // c:4621
                    continue; // c:4622
                }
            }
            if let Some(ch) = c {
                // c:4625 *bptr++ = c
                buf.push(ch);
            }
            c = crate::ported::lex::hgetc(); // c:4626
        }
        // c:4628 — *bptr = '\0'; (implicit — Rust String tracks len)

        // c:4629-4630 — if (!strcmp(t, str)) break;
        if &buf[t..] == str.as_str() {
            break;
        }
        // c:4631-4634 — if (lexstop) { t = bptr; break; }
        if crate::ported::lex::LEX_LEXSTOP.with(|f| f.get()) {
            t = buf.len();
            break;
        }
        // c:4635 — *bptr++ = '\n';
        buf.push('\n');
    }
    // c:4637 — *t = '\0';
    buf.truncate(t);

    // c:4638-4640 — s = buf; buf = dupstring(buf); zfree(s, bsiz);
    // The C dance frees the realloc'd block and re-allocates via the
    // string-heap allocator. Rust drops the old String when reassigned.
    buf = crate::ported::mem::dupstring(&buf);

    if qt == 0 {
        // c:4641
        // c:4642 — int ef = errflag;
        let ef = errflag.load(Ordering::Relaxed);
        // c:4644 — parsestr(&buf);
        if let Ok(parsed) = crate::ported::lex::parsestr(&buf) {
            buf = parsed;
        }
        // c:4646-4649 — if (!(errflag & ERRFLAG_ERROR)) errflag = ef | (errflag & ERRFLAG_INT);
        if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) == 0 {
            let cur = errflag.load(Ordering::Relaxed);
            errflag.store(
                ef | (cur & crate::ported::zsh_h::ERRFLAG_INT),
                Ordering::Relaxed,
            );
        }
    }
    Some(buf) // c:4651 return buf
}

/// Free-function wrapper for `getoutput()` from `Src/exec.c:4712`.
/// Runs a command-substitution body in the active executor and
/// returns its captured stdout. The C signature is `LinkList
/// getoutput(char *cmd, int qt)` but every caller in subst.rs
/// joins the list back into a string, so the Rust port collapses
/// the intermediate.
///
/// Uses `with_executor` (panics on missing VM context), not
/// `try_with_executor + unwrap_or_default()`. C `getoutput` calls
/// `execpline` directly — there's no "no shell" code path. The
/// silent-no-op pattern (return empty string when no executor) would
/// mask catastrophic state corruption as "command produced no output",
/// which is the failure mode the `subst.rs:496` warning block flags.
pub fn getoutput(cmd: &str) -> String {
    // c:4712 (Src/exec.c)
    with_executor(|exec| exec.run_command_substitution(cmd))
}

/// Direct port of `Shfunc loadautofn(Shfunc shf, int ks, int test_only,
/// int ignore_loaddir)` from `Src/exec.c:5050`. Walks `$fpath` for a
/// file named `shf->node.nam`, reads it, installs the text body on
/// the corresponding `shfunctab` entry, and clears `PM_UNDEFINED`.
///
/// C body (abridged):
///   1. `name = shf->node.nam`
///   2. `getfpfunc(name, &dir_path, NULL, 0)` → resolved file path
///   3. If !test_only && file found: parse → store eprog on
///      `shf->funcdef`; clear PM_UNDEFINED; set `shf->filename`.
///   4. Returns shf on success, NULL on failure.
///
/// Rust port: returns 0 = success, 1 = failure (matches the
/// existing call-site convention in `bin_functions -c`). Stores
/// raw file text on `ShFunc.body` (the Rust-side ShFunc in
/// `hashtable.rs:362`); the parser pass that converts text →
/// Eprog runs lazily at first call site.
/// Port of `loadautofn(Shfunc shf, int fksh, int autol, int current_fpath)` from `Src/exec.c:5682`.
pub fn loadautofn(
    shf: *mut crate::ported::zsh_h::shfunc, // c:5682 (Src/exec.c)
    _ks: i32,
    test_only: i32,
    _ignore_loaddir: i32,
) -> i32 {
    if shf.is_null() {
        return 1;
    }
    // c:5054 — `name = shf->node.nam`.
    let name = unsafe { (*shf).node.nam.clone() };
    // c:5070 — `path = getfpfunc(name, &dir_path, NULL, 0)`.
    let mut dir_path: Option<String> = None;
    let path = match getfpfunc(&name, &mut dir_path, None, 0) {
        Some(p) => p,
        None => return 1, // c:5074 not found
    };
    if test_only != 0 {
        // c:5096
        return 0; // test passes — file exists
    }
    // c:5100-5140 — read the file. C uses zopen + read + parse_string +
    // execsave; Rust port stores raw text on the ShFunc and defers
    // parse-to-Eprog until the first call.
    let body = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return 1,
    };
    // c:5142 — `shf->filename = ztrdup(dir_path)`.
    unsafe {
        (*shf).filename = dir_path.clone().or(Some(path.clone()));
    }
    // c:5148 — `shf->node.flags &= ~PM_UNDEFINED`.
    unsafe {
        (*shf).node.flags &= !(PM_UNDEFINED as i32);
    }
    // Sync the body string into the Rust-side ShFunc table so the
    // lazy-parse path can find it later.
    if let Ok(mut tab) = crate::ported::hashtable::shfunctab_lock().write() {
        if let Some(existing) = tab.get_mut(&name) {
            existing.body = Some(body);
            existing.filename = dir_path;
        } else {
            tab.add(crate::ported::zsh_h::shfunc {
                node: crate::ported::zsh_h::hashnode {
                    next: None,
                    nam: name.clone(),
                    flags: 0,
                },
                filename: dir_path,
                lineno: 0,
                funcdef: None,
                redir: None,
                sticky: None,
                body: Some(body),
            });
        }
    }
    0
}

/// Port of `getfpfunc(char *s, int *ksh, char **fdir, char **alt_path, int test_only)` from Src/exec.c:5260. Walks `$fpath` (or the
/// supplied `spec_path` slice) for a file named `name` and writes the
/// resolved directory through `*dir_path_out` (matching the C `char **dir_path`).
/// Returns `Some(file_contents_path)` on success, `None` when not found.
pub fn getfpfunc(
    name: &str,
    dir_path_out: &mut Option<String>, // c:5260 (Src/exec.c)
    spec_path: Option<&[String]>,
    _all_loaded: i32,
) -> Option<String> {
    let dirs: Vec<String> = match spec_path {
        Some(s) => s.to_vec(),
        None => std::env::var("FPATH")
            .or_else(|_| std::env::var("fpath"))
            .ok()
            .map(|v| v.split(':').map(String::from).collect())
            .unwrap_or_default(),
    };
    for dir in &dirs {
        if dir.is_empty() {
            continue;
        }
        let path = format!("{}/{}", dir, name);
        if std::path::Path::new(&path).exists() {
            *dir_path_out = Some(dir.clone());
            return Some(path);
        }
    }
    None
}

/// Port of `resolvebuiltin(const char *cmdarg, HashNode hn)` from
/// `Src/exec.c:2703`. Ensures that an autoload-stub builtin has its
/// module loaded before the caller invokes its `handlerfunc`. If the
/// stub has no handler, `ensurefeature` is asked to load the module
/// and re-lookup the builtin node. C body (abridged):
/// ```c
/// if (!((Builtin) hn)->handlerfunc) {
///     char *modname = dupstring(((Builtin) hn)->optstr);
///     (void)ensurefeature(modname, "b:", ...);
///     hn = builtintab->getnode(builtintab, cmdarg);
///     if (!hn) { lastval=1; zerr(...); return NULL; }
/// }
/// return hn;
/// ```
///
/// WARNING: zshrs's builtin table is the static `BUILTINS` array in
/// `src/ported/builtin.rs`. Module autoload (`ensurefeature` from
/// `Src/module.c`) is not yet wired through the same code path; the
/// helper exists today as `crate::ported::module::ensurefeature` for
/// the few sites that touch it. Until module-autoload is hooked up,
/// this port is the identity for builtins with a registered
/// `handlerfunc` and returns `None` for unresolved stubs (matching
/// the C return-NULL-on-failure contract).
pub fn resolvebuiltin<'a>(
    cmdarg: &str, // c:2703 (Src/exec.c)
    hn: &'a crate::ported::zsh_h::builtin,
) -> Option<&'a crate::ported::zsh_h::builtin> {
    // c:2705 — `if (!((Builtin) hn)->handlerfunc)`.
    if hn.handlerfunc.is_none() {
        // c:2706 — `modname = dupstring(((Builtin)hn)->optstr)`.
        let modname = hn.optstr.clone().unwrap_or_default();
        // c:2712-2714 — `ensurefeature(modname, "b:", ...)`. The Rust
        // module-autoload path is not yet wired; treat missing
        // handlerfunc as unresolvable.
        // c:2715-2721 — re-lookup, fail with `lastval=1` + zerr.
        crate::ported::utils::zerr(&format!(
            "autoloading module {} failed to define builtin: {}",
            modname, cmdarg
        ));
        return None; // c:2720
    }
    Some(hn) // c:2723
}

/// Dispatch decision returned by `execcmd_exec`'s head-walk port.
/// Encodes the local-variable state that the C function carries
/// through `c:2913-2916` (`is_builtin`, `is_shfunc`, `cflags`,
/// `use_defpath`) plus the precmd-modifier strip count. The fusevm
/// bytecode compiler reads this to emit the correct dispatch opcode
/// in `src/extensions/compile_zsh.rs::compile_simple`.
///
/// Not a C struct — invented to bridge the divergence between the
/// C wordcode-walker (which mutates locals + falls through to
/// invocation) and zshrs's split parse → compile → VM pipeline.
#[allow(non_camel_case_types)]
#[derive(Debug, Default, Clone)]
pub struct execcmd_dispatch {
    /// Number of `BINF_PREFIX` words to strip from the head of args.
    /// `Src/exec.c:3086 uremnode(preargs, firstnode(preargs))`.
    pub precmd_skip: usize,
    /// Set when the head (after strip) is a real builtin
    /// (`Src/exec.c:3065 is_builtin = 1`).
    pub is_builtin: bool,
    /// Set when the head (after strip) is a shell function
    /// (`Src/exec.c:3053 is_shfunc = 1`).
    pub is_shfunc: bool,
    /// `cflags` accumulator from `Src/exec.c:2915` — gathers
    /// `BINF_BUILTIN | BINF_COMMAND | BINF_EXEC | BINF_DASH |
    /// BINF_NOGLOB` bits encountered during the precommand-modifier
    /// walk (c:3062 `cflags |= hn->flags`).
    pub cflags: u32,
    /// `command -p` requested: use the default `$PATH` for lookup
    /// (`Src/exec.c:3160 use_defpath = 1`). NOT YET HONORED by the
    /// fusevm compiler — flagged for follow-up.
    pub use_defpath: bool,
    /// `command -v` / `command -V` requested: the dispatch target
    /// flips to `bin_whence` per `Src/exec.c:3149-3157`
    /// (`hn = &commandbn.node; is_builtin = 1`). The fusevm compiler
    /// reads this and emits `Op::CallBuiltin(BUILTIN_WHENCE_FROM_COMMAND)`
    /// instead of resolving the post-strip head.
    pub has_command_vv: bool,
    /// `exec -a NAME` requested: ARGV0 override per `Src/exec.c:3214-3240`.
    /// `Some(NAME)` triggers `zputenv("ARGV0=NAME")` before exec.
    pub exec_argv0: Option<String>,
    /// Empty-command branch fired with no redirs (`Src/exec.c:3372-3406`
    /// — the `else` arm of `if (redir && nonempty(redir))`). Covers
    /// bare `exec` / `noglob` / `command`. Caller emits
    /// `lastval = cmdoutval` (0 when no `$(cmd)` ran) and returns.
    /// Also fires for the `(cflags & BINF_PREFIX) && (cflags &
    /// BINF_COMMAND)` sub-case at `c:3365-3371` (bare `command`
    /// returns 0 without complaining about missing redirs).
    pub is_empty_command: bool,
}

/// Port of the head section of `execcmd_exec(Estate state,
/// Execcmd_params eparams, int input, int output, int how, int
/// last1, int close_if_forked)` from `Src/exec.c:2900`. Body
/// translated line-for-line from `c:2904-3091` covering local
/// initialisation, %job-table head detection, and the
/// precommand-modifier walk that strips `BINF_PREFIX` builtins
/// (`-`, `builtin`, `command`, `exec`, `noglob`) before dispatch.
///
/// =================== WARNING — DIVERGENCE ====================
///
/// The C function runs ~1500 lines and PERFORMS dispatch: it sets up
/// `multio` redirections, evaluates `varspc` assignments, then calls
/// `execbuiltin` / `runshfunc` / `execute` directly. zshrs DOES NOT
/// port that tail because the fusevm bytecode VM
/// (`src/extensions/compile_zsh.rs::compile_simple` +
/// `src/fusevm_bridge.rs::register_builtins`) emits dispatch opcodes
/// at compile time and the VM drives them at runtime.
///
/// This Rust port stops at `c:3091` — immediately after the
/// precmd-modifier walk — and returns the dispatch decision via
/// `execcmd_dispatch`. The fusevm compiler reads that struct to
/// decide which `Op::CallBuiltin` / `Op::CallFunction` / `Op::Exec`
/// to emit, and to compute the correct post-strip `argc`.
///
/// Code below this function's return point that lives in C
/// (`Src/exec.c:3092-4404`) is intentionally NOT ported. The
/// `BINF_COMMAND` / `BINF_EXEC` sub-modifier option-parsing block
/// (`c:3092-3275`) is a TODO — today `command -p`, `command -v/-V`,
/// `exec -a`, `exec -l`, `exec -c` are partially handled in
/// `compile_zsh.rs` / `src/ported/builtin.rs::bin_command` /
/// `bin_exec` rather than here. When those land canonically, they
/// extend this function past `c:3091`.
///
/// =============================================================
///
/// Signature adaptation: the C `Estate`/`Execcmd_params` carry the
/// wordcode iterator state — zshrs doesn't traverse wordcode, so
/// the args list arrives already-expanded as a `&[String]` (analog
/// of `preargs` after `execcmd_getargs` at `c:3028`). `type_`
/// mirrors `eparams->type` (`WC_SIMPLE` vs `WC_TYPESET`).
pub fn execcmd_exec(args: &[String], type_: u32) -> execcmd_dispatch {
    // c:2900 (Src/exec.c)

    // c:2904-2916 — locals.
    let mut hn: Option<&'static crate::ported::zsh_h::builtin> = None; // c:2904
    let mut is_shfunc = false; // c:2913
    let mut is_builtin = false; // c:2913
    let mut use_defpath = false; // c:2913
    let mut cflags: u32 = 0; // c:2915
    let mut orig_cflags: u32 = 0; // c:2915
    let _ = orig_cflags;
    // c:3263 — `char *exec_argv0 = NULL;` (declared inside the
    // BINF_EXEC arm; hoisted here so the dispatch struct can carry it
    // out after the loop terminates).
    let mut exec_argv0: Option<String> = None;
    // c:3149/3158 — `has_vV`/`has_p` flags from the BINF_COMMAND arm
    // (c:3104). Surface `has_vV` via the dispatch struct so the fusevm
    // compiler can emit `bin_whence` instead of resolving the head.
    let mut has_command_vv = false;

    // c:2962-2973 — `%job` head: rewrite `%name` → `fg|bg|disown %name`.
    // Not in scope for the compile-time dispatch walk: jobspec
    // expansion happens at runtime in fusevm; the bytecode emits a
    // direct `fg`/`bg` call when it sees a leading `%`. Flagged for
    // follow-up when the canonical port lands.

    // c:2975-2986 — AUTORESUME prefix-match against jobtab. Same
    // status as the %job head: runtime concern, deferred.

    // c:3013-3091 — precommand-modifier walk.
    let mut preargs: Vec<String> = args.to_vec(); // c:3027 newlinklist
    let mut precmd_skip: usize = 0;

    // c:3018 — `if ((type == WC_SIMPLE || type == WC_TYPESET) && args)`.
    if (type_ == WC_SIMPLE || type_ == WC_TYPESET) && !preargs.is_empty() {
        // c:3018
        // c:3029 — `while (nonempty(preargs))`.
        while precmd_skip < preargs.len() {
            // c:3029
            // c:3030 — `cmdarg = (char *) peekfirst(preargs);`.
            let cmdarg = crate::ported::lex::untokenize(&preargs[precmd_skip]);
            // c:3031 — `checked = !has_token(cmdarg)`. zshrs's fusevm
            // already performed prefork expansion on `preargs`, so
            // `has_token` is effectively false here; the C `break` on
            // unexpanded tokens is unreachable in this entry point.

            // c:3034-3035 — WC_TYPESET fast path: `getnode2` looks up
            // even disabled builtins so the reserved-word form
            // (`integer x`, `local foo`) still dispatches to the
            // typeset family. The static `BUILTINS` array doesn't
            // expose a separate disabled-bit lookup; one path covers
            // both. Effect is identical for the precmd-modifier walk.

            // c:3050-3052 — `if (!(cflags & (BINF_BUILTIN |
            // BINF_COMMAND)) && shfunctab->getnode(...))` — shell
            // function takes precedence unless a `builtin`/`command`
            // modifier preceded it.
            if (cflags & (BINF_BUILTIN | BINF_COMMAND)) == 0 {
                // c:3051
                if crate::ported::hashtable::shfunctab_lock()
                    .read()
                    .map(|t| t.iter().any(|(k, _)| k == &cmdarg))
                    .unwrap_or(false)
                {
                    is_shfunc = true; // c:3053
                    break; // c:3054
                }
            }
            // c:3056 — `builtintab->getnode(builtintab, cmdarg)`.
            let entry = crate::ported::builtin::BUILTINS
                .iter()
                .find(|b| b.node.nam == cmdarg);
            let Some(entry) = entry else {
                // c:3056-3058
                break;
            };
            hn = Some(entry);
            // c:3061-3063 — accumulate cflags.
            orig_cflags |= cflags;
            cflags &= !(BINF_BUILTIN | BINF_COMMAND);
            cflags |= entry.node.flags as u32;
            // c:3064 — `if (!(hn->flags & BINF_PREFIX))` — real
            // builtin, stop.
            if (entry.node.flags as u32 & BINF_PREFIX) == 0 {
                // c:3064
                // WARNING — DIVERGENCE: c:3068 calls `resolvebuiltin`
                // to autoload the builtin's module if its
                // `handlerfunc` is NULL. In zshrs, builtins live in
                // two places: the static `BUILTINS` table (which
                // mirrors C `handlerfunc`, often `None` for ports
                // dispatched through fusevm) AND fusevm's
                // `register_builtins` map (the actual runtime
                // dispatcher). A null `handlerfunc` in the static
                // table is NOT an autoload failure for us — it
                // means dispatch routes through fusevm. So we
                // skip the resolvebuiltin call here; the faithful
                // port remains available for future callers that
                // genuinely need module-autoload semantics.
                is_builtin = true; // c:3065
                break; // c:3077
            }
            // c:3086 — `uremnode(preargs, firstnode(preargs))`.
            precmd_skip += 1;
            // c:3087-3091 — `if (!firstnode(preargs)) { execcmd_getargs
            //   (...); if (!firstnode(preargs)) break; }`. zshrs has
            // no `execcmd_getargs` (args arrive pre-expanded); the
            // bounds-check at the top of `while precmd_skip <
            // preargs.len()` handles the empty case identically.

            // c:3092-3177 — BINF_COMMAND sub-option parsing
            // (`command -p / -v / -V`).
            if (cflags & BINF_COMMAND) != 0 && precmd_skip < preargs.len() {
                // c:3102-3104 — `LinkNode argnode, oldnode, pnode = NULL;
                //                int has_p = 0, has_vV = 0, has_other = 0;`
                let mut argnode: usize = precmd_skip; // c:3105 `argnode = firstnode(preargs);`
                let mut pnode: Option<usize> = None; // c:3102
                let mut has_p = false; // c:3104
                let mut has_vv = false; // c:3104
                let mut has_other = false; // c:3104
                // c:3107 — `while (IS_DASH(*argdata))`
                while argnode < preargs.len()
                    && IS_DASH(preargs[argnode].chars().next().unwrap_or('\0'))
                {
                    let argdata = preargs[argnode].clone(); // c:3106
                    let bytes = argdata.as_bytes();
                    // c:3108-3111 — stop on bare `-` or `--`.
                    if bytes.len() < 2 || (IS_DASH(bytes[1] as char) && bytes.len() == 2) {
                        // c:3109
                        break; // c:3111
                    }
                    // c:3112-3133 — scan flag chars.
                    for &c in &bytes[1..] {
                        // c:3112
                        match c as char {
                            'p' => {
                                // c:3114
                                has_p = true; // c:3122
                                pnode = Some(argnode); // c:3123
                            }
                            'v' | 'V' => {
                                // c:3125-3126
                                has_vv = true; // c:3127
                            }
                            _ => {
                                // c:3129
                                has_other = true; // c:3130
                            }
                        }
                    }
                    // c:3134-3138 — unknown flag → don't try, leave alone.
                    if has_other {
                        // c:3134
                        has_p = false; // c:3136
                        has_vv = false; // c:3136
                        break; // c:3137
                    }
                    // c:3140-3147 — advance to next arg.
                    argnode += 1; // c:3141 nextnode(argnode)
                    if argnode >= preargs.len() {
                        // c:3142 — execcmd_getargs (skipped: pre-expanded)
                        break; // c:3145
                    }
                }
                // c:3149-3157 — `-v`/`-V` → dispatch to whence.
                if has_vv {
                    // c:3149
                    // c:3154 `pushnode(preargs, "command")` — C re-inserts
                    // "command" so bin_whence sees it as argv[0]. zshrs
                    // surfaces this via `has_command_vv`; the fusevm
                    // compiler emits the equivalent whence call.
                    has_command_vv = true; // c:3155-3156 hn = &commandbn; is_builtin=1
                    is_builtin = true;
                    break; // c:3157
                } else if has_p {
                    // c:3158
                    use_defpath = true; // c:3160
                    if let Some(pn) = pnode {
                        // c:3165 — `uremnode(preargs, pnode)`. zshrs:
                        // remove the `-p`-bearing arg from preargs.
                        if pn < preargs.len() {
                            preargs.remove(pn);
                            // precmd_skip already accounts for the
                            // stripped `command` prefix; we just removed
                            // the `-p` flag which sat at preargs[pn].
                            // No precmd_skip change needed — the head
                            // remains where it was.
                        }
                    }
                }
                // c:3176-3177 — `--` trailing end-of-options strip.
                if argnode < preargs.len() {
                    let argdata = &preargs[argnode];
                    let b = argdata.as_bytes();
                    if b.len() == 2 && IS_DASH(b[0] as char) && IS_DASH(b[1] as char) {
                        // c:3176
                        preargs.remove(argnode); // c:3177
                    }
                }
            } else if (cflags & BINF_EXEC) != 0 && precmd_skip < preargs.len() {
                // c:3178-3275 — BINF_EXEC sub-option parsing
                // (`exec -a NAME -l -c`).
                let mut argnode: usize = precmd_skip; // c:3185
                let mut error_done = false;
                // c:3196 — `while (argdata && IS_DASH(*argdata) &&
                //                  strlen(argdata) >= 2)`
                while argnode < preargs.len() {
                    let argdata = preargs[argnode].clone();
                    let bytes = argdata.as_bytes();
                    if bytes.is_empty() || !IS_DASH(bytes[0] as char) || bytes.len() < 2 {
                        break; // c:3196 loop guard
                    }
                    let oldnode = argnode; // c:3197
                    argnode += 1; // c:3198 nextnode(oldnode)
                                  // c:3203-3208 — empty next → error.
                    if argnode >= preargs.len() {
                        // c:3203
                        crate::ported::utils::zerr(
                            // c:3204
                            "exec requires a command to execute",
                        );
                        errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed); // c:3206
                        error_done = true;
                        break; // c:3207 goto done
                    }
                    // c:3209 — `uremnode(preargs, oldnode)`.
                    preargs.remove(oldnode);
                    argnode -= 1; // re-anchor — `argnode` was the post-removed slot
                                  // c:3210-3211 — `--` stops option scan.
                    if bytes.len() == 2 && IS_DASH(bytes[0] as char) && IS_DASH(bytes[1] as char) {
                        // c:3210
                        break; // c:3211
                    }
                    // c:3212-3258 — scan flag chars after the leading `-`.
                    let mut k = 1usize;
                    while k < bytes.len() && !error_done {
                        let cmdopt = bytes[k] as char; // c:3212
                        match cmdopt {
                            'a' => {
                                // c:3214 — `-a` ARGV0 override.
                                if k + 1 < bytes.len() {
                                    // c:3216 — `-aNAME` inline form.
                                    exec_argv0 =
                                        Some(String::from_utf8_lossy(&bytes[k + 1..]).into_owned()); // c:3217
                                    k = bytes.len(); // c:3219 position past end
                                } else {
                                    // c:3220 — `-a NAME` separate form.
                                    if argnode >= preargs.len() {
                                        // c:3230
                                        crate::ported::utils::zerr(
                                            // c:3231
                                            "exec flag -a requires a parameter",
                                        );
                                        errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed); // c:3233
                                        error_done = true;
                                        break; // c:3234 goto done
                                    }
                                    exec_argv0 = Some(preargs[argnode].clone()); // c:3236
                                    preargs.remove(argnode); // c:3239
                                }
                            }
                            'c' => {
                                // c:3242
                                cflags |= crate::ported::zsh_h::BINF_CLEARENV; // c:3243
                            }
                            'l' => {
                                // c:3245
                                cflags |= crate::ported::zsh_h::BINF_DASH; // c:3246
                            }
                            _ => {
                                // c:3248
                                crate::ported::utils::zerr(
                                    // c:3249
                                    &format!("unknown exec flag -{}", cmdopt),
                                );
                                errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed); // c:3251
                                error_done = true;
                                break; // c:3256
                            }
                        }
                        k += 1;
                    }
                    if error_done {
                        break;
                    }
                }
                // c:3263-3274 — zputenv("ARGV0=NAME"). zshrs defers
                // the actual `setenv` to the fusevm compiler / external
                // exec path; we surface `exec_argv0` via the dispatch
                // struct so the caller can apply it before fork+exec.
                if let Some(ref a0) = exec_argv0 {
                    // c:3263 — `remnulargs + untokenize` then setenv.
                    let cleaned = crate::ported::lex::untokenize(a0); // c:3266-3267
                    exec_argv0 = Some(cleaned);
                }
                if error_done {
                    return execcmd_dispatch {
                        precmd_skip,
                        is_builtin,
                        is_shfunc,
                        cflags,
                        use_defpath,
                        has_command_vv,
                        exec_argv0,
                        is_empty_command: false,
                    };
                }
            }
        }
    }

    // c:3309-3406 — "Empty command" branch. When the precmd-modifier
    // walk above strips every word with nothing left to dispatch
    // (bare `exec`, bare `noglob`, bare `command`, bare `nocorrect`),
    // C falls into `if (!args || empty(args))` at c:3331. Sub-cases:
    //
    // - redir-present + do_exec       → nullexec=1 (continue to run)
    // - redir-present + varspc        → nullexec=2 (continue)
    // - redir-present + no nullcmd    → `zerr("redirection with no command")`
    //                                   lastval=1, return
    // - redir-present + SHNULLCMD     → args=[":"]
    // - redir-present + readnullcmd   → args=[readnullcmd]
    // - redir-present + default       → args=[nullcmd]
    // - NO redir + BINF_PREFIX+COMMAND → lastval=0, return (c:3365-3371)
    // - NO redir + default            → lastval=cmdoutval, return (c:3372-3406)
    //
    // zshrs's `execcmd_exec` doesn't receive `redir` (it takes `args`
    // only). The cases that DEPEND on redirs are handled by
    // `compile_zsh.rs::compile_redir` before this dispatch fires; the
    // remaining cases collapse into the single `is_empty_command`
    // flag below. Both NO-redir sub-cases produce the same observable
    // outcome (lastval=0, return without invoking anything), so a
    // single flag suffices.
    let is_empty_command = precmd_skip >= preargs.len();

    // =================== WARNING — DIVERGENCE ====================
    // c:3285+: prefork-substitution, magic_assign decision, multio
    // setup, varspc evaluation, and the actual execbuiltin /
    // runshfunc / execute call. ~1300 lines of interpreter-only
    // code, entirely replaced by fusevm bytecode dispatch in
    // `src/extensions/compile_zsh.rs::compile_simple` and the
    // opcode handlers in `src/fusevm_bridge.rs::register_builtins`.
    // The return value below feeds those compile-time decisions.
    // =============================================================

    let _ = hn;
    execcmd_dispatch {
        precmd_skip,
        is_builtin,
        is_shfunc,
        cflags,
        use_defpath,
        has_command_vv,
        exec_argv0,
        is_empty_command,
    }
}
