//! Extension-only shell builtins (no zsh C counterpart).
//!
//! Every method here implements a builtin that does NOT exist in
//! `src/zsh/Src/` — these are zshrs-specific additions: coreutils
//! drop-ins, bash-only builtins (caller, shopt, readarray), zshrs
//! features (async, await, barrier, doctor, profile, intercept),
//! contrib autoloads exposed as builtins (compdef, compinit, zmv,
//! zcalc, peach), etc.
//!
//! These methods previously lived on `ShellExecutor` in
//! `src/ported/vm_helper`. They were bulk-moved here so that
//! `src/ported/` only contains C-port code, satisfying the
//! `port_purity` discipline described in `docs/PORT.md`.

#![allow(unused_imports)]

/// Canonical name list of every extension builtin defined in this
/// module. Each entry maps to a `builtin_<NAME>` method on
/// `ShellExecutor`. Kept hand-sorted for review-friendly diffs.
///
/// Used by `lsp::dump_reflection_json` (Extensions tab in the
/// IntelliJ tool window) and `lsp::dump_reference_html`
/// (`ch-lsp-extensions` chapter in `docs/reference.html`). When you
/// add a `pub(crate) fn builtin_X` method below, ALSO add `X` here
/// (sorted) so the inventory stays in sync.
/// True when the shell's user-visible BUILTIN TABLE must omit
/// [`EXT_BUILTIN_NAMES`] — the `builtins` magic assoc
/// (`Src/Modules/parameter.c`'s `scanbuiltins` port) and the legacy
/// compctl builtin namespace (`Src/Zle/compctl.c`'s `dumphashtable`
/// port).
///
/// Two ways to turn on:
///
///   * `--zsh` strict emulation — those names do not exist in zsh, so
///     the emulated namespace must not invent them.
///   * `ZSHRS_HIDE_EXT_BUILTINS` set to any non-empty value — a
///     MEASUREMENT knob for the byte-for-byte parity harnesses, so they
///     can diff zshrs's zsh-compatible namespace against real zsh
///     without ~145 zshrs-original builtins showing up as spurious
///     diffs. It is NOT a compat mode: nothing else about the shell
///     changes, and in particular DISPATCH is untouched — `peach`,
///     `doctor`, `async` still run, and `whence -w`/`type`/`command -v`
///     still resolve them as builtins.
///
/// The env read is cached: both callers are name-enumeration hot paths
/// (`${(k)builtins}`, command-position completion). The `--zsh` half is
/// read live because the mode flag is set during argument parsing.
pub fn hide_ext_builtins() -> bool {
    static HIDE_ENV: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let env_hide = *HIDE_ENV
        .get_or_init(|| std::env::var_os("ZSHRS_HIDE_EXT_BUILTINS").is_some_and(|v| !v.is_empty()));
    env_hide || crate::IS_ZSH_MODE.load(std::sync::atomic::Ordering::Relaxed)
}

/// The module each statically-linked builtin belongs to, or `None` for
/// a `zsh/main` core builtin that is always present.
///
/// In C `builtintab` is a live hash table: a module's builtins are
/// added by `addbuiltins()` when the module loads and removed when it
/// unloads, so `builtintab` only ever holds names that are callable
/// right now. zshrs links every module statically and keeps ONE flat
/// `BUILTINS` slice (`src/ported/builtin.rs`), so the module→name
/// association that C carries implicitly in table membership has to be
/// spelled out here.
///
/// `"__zshrs_only"` marks entries in zshrs's `BUILTINS` table that have
/// no upstream counterpart at all (debug/bytecode hooks); no `zmodload`
/// can make them appear in a real zsh.
pub fn builtin_owning_module(name: &str) -> Option<&'static str> {
    match name {
        // zsh/files (Src/Modules/files.c:806-824).
        "chmod" | "chgrp" | "chown" | "ln" | "mkdir" | "mv" | "rm" | "rmdir" | "sync" => {
            Some("zsh/files")
        }
        "zf_chmod" | "zf_chgrp" | "zf_chown" | "zf_ln" | "zf_mkdir" | "zf_mv" | "zf_rm"
        | "zf_rmdir" | "zf_sync" => Some("zsh/files"),
        // zsh/zftp (Src/Modules/zftp.c).
        "zftp" => Some("zsh/zftp"),
        // zsh/net/tcp (Src/Modules/tcp.c).
        "ztcp" => Some("zsh/net/tcp"),
        // zsh/net/socket (Src/Modules/socket.c).
        "zsocket" => Some("zsh/net/socket"),
        // zsh/stat (Src/Modules/stat.c).
        "stat" | "zstat" => Some("zsh/stat"),
        // zsh/zselect (Src/Modules/zselect.c).
        "zselect" => Some("zsh/zselect"),
        // zsh/zpty (Src/Modules/zpty.c).
        "zpty" => Some("zsh/zpty"),
        // zsh/zprof (Src/Modules/zprof.c).
        "zprof" => Some("zsh/zprof"),
        // zsh/system (Src/Modules/system.c).
        "zsystem" | "syserror" | "sysopen" | "sysread" | "sysseek" | "syswrite" => {
            Some("zsh/system")
        }
        // zsh/clone (Src/Modules/clone.c).
        "clone" => Some("zsh/clone"),
        // zsh/curses (Src/Modules/curses.c).
        "zcurses" => Some("zsh/curses"),
        // zsh/db/gdbm (Src/Modules/db_gdbm.c).
        "ztie" | "zuntie" | "zgdbmpath" => Some("zsh/db/gdbm"),
        // zsh/pcre (Src/Modules/pcre.c).
        "pcre_compile" | "pcre_match" | "pcre_study" => Some("zsh/pcre"),
        // zsh/example (Src/Modules/example.c).
        "example" => Some("zsh/example"),
        // zsh/cap (Src/Modules/cap.c).
        "cap" | "getcap" | "setcap" => Some("zsh/cap"),
        // zsh/attr (Src/Modules/attr.c).
        "zgetattr" | "zsetattr" | "zdelattr" | "zlistattr" => Some("zsh/attr"),
        // zsh/datetime (Src/Modules/datetime.c).
        "strftime" => Some("zsh/datetime"),
        // zsh/param/private — Src/Modules/param_private.c:217.
        "private" => Some("zsh/param/private"),
        // zshrs-only debug / bytecode hooks with no upstream builtin.
        "hashinfo" | "mem" | "patdebug" | "nameref" | "__rust_compile" => Some("__zshrs_only"),
        // zsh/main core builtins.
        _ => None,
    }
}

/// Would C's live `builtintab` currently contain `name`?
///
/// This is THE predicate for "is this builtin name visible to a
/// namespace walk right now". Both walkers must agree or the shell
/// reports one set through `${(k)builtins}` and a different one through
/// command-position completion:
///
///   * `scanbuiltins` — the `builtins` / `dis_builtins` magic assocs
///     (port of `Src/Modules/parameter.c:816-840`);
///   * `makecomplistflags` — the compctl namespace dump
///     (port of `Src/Zle/compctl.c:3654`, which walks `builtintab`).
///
/// They had drifted: only the first applied the module gate, so
/// `rustup <TAB>` — which falls through `_default` to `compcall` — put
/// 44 names (`strftime`, `zstat`, `zpty`, `zf_chmod`, `pcre_match`, …)
/// into the match list that `${#builtins}` correctly reported as
/// absent, and that a real zsh only exposes after the owning
/// `zmodload`.
///
/// The gate is applied under `hide_ext_builtins()` — `--zsh` strict
/// emulation and the `ZSHRS_HIDE_EXT_BUILTINS` parity knob. Default
/// zshrs mode keeps the auto-load posture where every statically-linked
/// module builtin is callable without an explicit `zmodload`, and
/// dispatch is never affected either way.
/// Would C's `builtintab` hold `name` right now, given which modules are
/// loaded? This is the module gate ONLY — it says nothing about `disable`.
///
/// C adds a module's builtins to `builtintab` in `addbuiltins()` when the
/// module loads (Src/module.c:551) and removes them on unload, so a name
/// owned by an unloaded module simply is not there; names registered for
/// AUTO-loading (Src/module.c:1265 `add_autobin`, seeded by Src/init.c:1708
/// `init_bltinmods`) are present as stubs from the start. zshrs links every
/// module statically into ONE flat table, so the gate has to be applied
/// explicitly.
///
/// Every consumer must ask THIS question, or the shell answers differently
/// depending on which one you ask. `whence`/`type` used to carry their own
/// hand-maintained list of gated names, grown one bug report at a time
/// (docs/BUGS.md #28, #532, #535), which covered zsh/files, zsh/stat,
/// zsh/zselect, zsh/zpty, zsh/net/tcp, zsh/zftp and zsh/system — and nothing
/// else. So `whence -w strftime` (zsh/datetime), `pcre_compile` (zsh/pcre),
/// `clone`, `zcurses`, `ztie`, `cap`, `zgetattr`, `sysopen`, `zsocket`,
/// `example` and `zprof` all answered `builtin` where zsh answers `none`,
/// while `${+builtins[...]}` — which asks the generic question below —
/// answered 0 for the very same names in the very same shell.
pub fn module_builtin_available(name: &str) -> bool {
    // c:Src/module.c:521 — `setbuiltins` DELETES the node from `builtintab`
    // when the feature's enable bit is cleared, so a `zmodload -F MODULE
    // -b:NAME` makes the name vanish even though the module stays loaded.
    // zshrs's builtintab is immutable, so that half of the state lives in
    // `DISABLED_MODULE_BUILTINS` (see its doc comment); consult it before
    // the per-module load gate below.
    if crate::ported::module::DISABLED_MODULE_BUILTINS
        .lock()
        .map(|s| s.contains(name))
        .unwrap_or(false)
    {
        return false;
    }
    match builtin_owning_module(name) {
        // A `zsh/main` core builtin (always in the table), or a
        // zshrs-original entry that belongs to no module at all.
        None | Some("__zshrs_only") => true,
        Some(modname) => crate::ported::module::MODULESTAB
            .lock()
            .map(|t| {
                // "Actually loaded" is C's `m->u.handle && !MOD_UNLOAD`
                // (Src/module.c:1055); the static-link analogue is MOD_INIT_B
                // set and MOD_UNLOAD clear — the same criterion `getpmmodule`
                // uses to print `loaded` vs `autoloaded`. NOT `is_loaded()`,
                // which keys off MOD_LINKED and is pre-seeded for every
                // compiled-in module, so it answered "loaded" for zsh/datetime
                // and zsh/pcre and made `whence -w strftime` /
                // `${+builtins[pcre_compile]}` claim builtins that `zmodload`
                // had never brought in.
                let loaded = t.modules.get(modname).is_some_and(|md| {
                    (md.node.flags & crate::ported::zsh_h::MOD_INIT_B) != 0
                        && (md.node.flags & crate::ported::zsh_h::MOD_UNLOAD) == 0
                });
                // A name registered for AUTO-loading is a stub in builtintab
                // from the start (Src/module.c:1265 `add_autobin`).
                loaded || t.resolve_autoload_builtin(name).is_some()
            })
            .unwrap_or(false),
    }
}

pub fn builtin_in_builtintab(name: &str) -> bool {
    // The gate used to apply ONLY under `hide_ext_builtins()`, on the theory
    // that default zshrs mode should show every statically-linked module
    // builtin as available. Dispatch never agreed: `zshrs -fc 'builtin chmod'`
    // runs /bin/chmod and `whence -w chmod` says `command` with or without
    // that flag. So in default mode `${(k)builtins}` listed 40 names —
    // chmod, rm, mv, stat, zpty, zselect, sys*, cap/getcap, z*attr, zf_* —
    // that `${+builtins[$name]}` reported as unset in the SAME shell, and
    // that no `builtin NAME` call could reach. Ask the same question every
    // other consumer asks.
    //
    // The one thing this walk adds on top: zshrs-original entries that no
    // `zmodload` could ever produce in a real zsh (`hashinfo`, `mem`,
    // `patdebug`, `nameref`, `__rust_compile`) must disappear from the
    // NAMESPACE under `--zsh` / ZSHRS_HIDE_EXT_BUILTINS, even though they stay
    // perfectly available (that flag never affects dispatch). The previous
    // gate got this for free by asking `is_loaded("__zshrs_only")`, which is
    // false because no such module exists.
    if hide_ext_builtins() && matches!(builtin_owning_module(name), Some("__zshrs_only")) {
        return false;
    }
    module_builtin_available(name)
}

pub const EXT_BUILTIN_NAMES: &[&str] = &[
    "arch",
    "async",
    "await",
    "barrier",
    "base64",
    "basename",
    "caller",
    "cat",
    "cdreplay",
    "cksum",
    "comm",
    "compdef",
    "compgen",
    "compinit",
    "complete",
    "cut",
    "date",
    "dbview",
    "dircolors",
    "dirname",
    "doctor",
    "env",
    "expand",
    "expr",
    "factor",
    "find",
    "fold",
    "groups",
    "head",
    "help",
    "hostname",
    "id",
    "intercept",
    "intercept_proceed",
    "link",
    "logname",
    "mkfifo",
    "mktemp",
    "nice",
    "nl",
    "nproc",
    "paste",
    "peach",
    "pgrep",
    "pmap",
    "printenv",
    "profile",
    "provenance",
    "realpath",
    "rev",
    "run_tests",
    "seq",
    "sha256sum",
    "shuf",
    "sleep",
    "sort",
    "sum",
    "tac",
    "tail",
    "tee",
    "touch",
    "tput",
    "tr",
    "tsort",
    "tty",
    "uname",
    "unexpand",
    "uniq",
    "unlink",
    "users",
    "wc",
    "whoami",
    "yes",
    "zassert_contains",
    "zassert_dies",
    "zassert_eq",
    "zassert_err",
    "zassert_false",
    "zassert_ge",
    "zassert_gt",
    "zassert_le",
    "zassert_lt",
    "zassert_match",
    "zassert_ne",
    "zassert_near",
    "zassert_ok",
    "zassert_true",
    "zbuild",
    "ztest_run",
    "ztest_skip",
    // NOT listed here, deliberately, though both once were:
    //   * `cp` — the dispatcher runs /usr/bin/cp (`cp --recursive` fails with
    //     BSD cp's "illegal option", and `whence -w cp` says `command`), so
    //     `cp_impl` is not what answers; zsh has no `cp` builtin either, and
    //     zsh/files ships chmod/chown/ln/mkdir/mv/rm/rmdir/sync but no cp.
    //   * `add_zsh_hook` — not callable at all (`command not found` with and
    //     without -f); in zsh it is an autoloadable FUNCTION, not a builtin.
    // Listing them made `${(k)builtins}` advertise two names that no
    // `builtin NAME` call could reach and that `${+builtins[...]}` reported
    // as unset.
];

/// Body of the `compdef` shell-function stub that `compinit` installs so
/// `${+functions[compdef]}` is true (zinit's "compinit has loaded" probe)
/// and `compdef` calls resolve to a function rather than command-not-found.
/// The `BUILTIN_COMPDEF` opcode handler recognises this exact body and
/// routes to the fast native `builtin_compdef`; a genuine user/compsys
/// `compdef` function (any other body) still takes precedence. In real zsh
/// `compdef` is a shell function defined inside `compinit`; zshrs's native
/// `compinit` implements the scan in Rust and never sourced that function,
/// which left `compdef` undefined and broke zinit's compdef-replay.
pub(crate) const NATIVE_COMPDEF_MARKER: &str = ": zshrs-native-compdef-stub";

use crate::parse::Redirect;
use crate::ported::utils::{errflag, ERRFLAG_ERROR};
use crate::ported::vm_helper::ShellExecutor;
use crate::ported::vm_helper::*;
use crate::ported::zsh_h::PM_UNDEFINED;
use chrono::{Datelike, TimeZone};
use rand::seq::SliceRandom;
use rand::Rng;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::env;
use std::ffi::CStr;
use std::ffi::CString;
use std::fs::OpenOptions;
use std::io::Read as IoRead;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::ToSocketAddrs;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::path::Component::*;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

impl ShellExecutor {
    /// caller - display call stack (bash)
    /// caller [N] — bash builtin returning the location of the
    /// current frame N. With no arg or N=0: 'LINE FUNC' (or just
    /// 'LINE main' at top level). With N>0: 'LINE FUNC FILE' for
    /// frame N. Direct port of bash's bin_caller in builtins.def.
    /// Reads from the existing $funcstack array we now maintain
    /// (vm_helper:7828-7835).
    pub(crate) fn builtin_caller(&self, args: &[String]) -> i32 {
        let depth: usize = args.first().and_then(|s| s.parse().ok()).unwrap_or(0);
        let stack = self.array("funcstack").unwrap_or_default();
        // funcstack[0] is the current (innermost) frame — caller 0
        // refers to the immediate caller per bash semantics, which
        // is funcstack[0] for us. With no args, just LINE FUNC; we
        // don't track per-frame line numbers yet so emit `0` as
        // line number until the VM pipes that through.
        if stack.is_empty() {
            // bash(1), `caller`: "The return value is 0 unless the shell is
            // not executing a subroutine call". At the top level bash prints
            // NOTHING and returns 1; the previous code printed the synthetic
            // frame `0 main` and returned 0, so the common
            // `caller || echo "not in a function"` idiom took the wrong
            // branch and a stack-trace loop never terminated.
            return 1;
        }
        if depth == 0 {
            let func = stack.first().cloned().unwrap_or_else(|| "main".to_string());
            println!("0 {}", func);
            0
        } else if depth < stack.len() {
            let func = stack[depth].clone();
            // `find_function_file` was deleted with the old exec.c
            // stubs (it always returned None). Until the canonical
            // `functions_source` map is wired, fall back to "main".
            let file = "main".to_string();
            println!("0 {} {}", func, file);
            0
        } else {
            // Bash returns 1 (no frame at that depth) silently.
            1
        }
    }

    /// doctor - diagnostic report of shell health, caches, and performance
    pub(crate) fn builtin_doctor(&self, _args: &[String]) -> i32 {
        let green = |s: &str| format!("\x1b[32m{}\x1b[0m", s);
        let red = |s: &str| format!("\x1b[31m{}\x1b[0m", s);
        let yellow = |s: &str| format!("\x1b[33m{}\x1b[0m", s);
        let bold = |s: &str| format!("\x1b[1m{}\x1b[0m", s);
        let dim = |s: &str| format!("\x1b[2m{}\x1b[0m", s);
        let format_bytes = |n: u64| -> String {
            const KIB: u64 = 1024;
            const MIB: u64 = 1024 * KIB;
            const GIB: u64 = 1024 * MIB;
            if n >= GIB {
                format!("{:.2} GiB", n as f64 / GIB as f64)
            } else if n >= MIB {
                format!("{:.2} MiB", n as f64 / MIB as f64)
            } else if n >= KIB {
                format!("{:.1} KiB", n as f64 / KIB as f64)
            } else {
                format!("{} B", n)
            }
        };

        println!("{}", bold("zshrs doctor"));
        println!("{}", dim(&"=".repeat(60)));
        println!();

        // --- Environment ---
        println!("{}", bold("Environment"));
        println!("  version:    zshrs {}", env!("CARGO_PKG_VERSION"));
        println!("  pid:        {}", std::process::id());
        let cwd = env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "?".to_string());
        println!("  cwd:        {}", cwd);
        println!(
            "  shell:      {}",
            env::var("SHELL").unwrap_or_else(|_| "?".to_string())
        );
        println!("  pool size:  {}", self.worker_pool.size());
        println!(
            "  pool done:  {} tasks completed",
            self.worker_pool.completed()
        );
        println!("  pool queue: {} pending", self.worker_pool.queue_depth());
        println!();

        // --- Config ---
        println!("{}", bold("Config"));
        let config_path = crate::config::config_path();
        if config_path.exists() {
            println!("  {}  {}", green("*"), config_path.display());
        } else {
            println!(
                "  {}  {} {}",
                dim("-"),
                config_path.display(),
                dim("(using defaults)")
            );
        }
        println!();

        // --- PATH ---
        println!("{}", bold("PATH"));
        let path_var = env::var("PATH").unwrap_or_default();
        let path_dirs: Vec<&str> = path_var.split(':').filter(|s| !s.is_empty()).collect();
        let path_ok = path_dirs
            .iter()
            .filter(|d| std::path::Path::new(d).is_dir())
            .count();
        let path_missing = path_dirs.len() - path_ok;
        println!(
            "  directories: {} total, {} {}, {} {}",
            path_dirs.len(),
            path_ok,
            green("valid"),
            path_missing,
            if path_missing > 0 {
                red("missing")
            } else {
                green("missing")
            },
        );
        println!(
            "  hash table:  {} entries",
            crate::ported::hashtable::cmdnamtab_lock()
                .read()
                .map(|t| t.len())
                .unwrap_or(0)
        );
        println!();

        // --- FPATH ---
        println!("{}", bold("FPATH"));
        println!("  directories: {}", self.fpath.len());
        let fpath_ok = self.fpath.iter().filter(|d| d.is_dir()).count();
        let fpath_missing = self.fpath.len() - fpath_ok;
        if fpath_missing > 0 {
            println!("  {} {} missing fpath directories", red("!"), fpath_missing);
        }
        println!("  functions:   {} loaded", self.function_names().len());
        // Count canonical shfunctab entries with PM_UNDEFINED set.
        let autoload_count = crate::ported::hashtable::shfunctab_lock()
            .read()
            .map(|t| {
                t.iter()
                    .filter(|(_, shf)| (shf.node.flags as u32 & PM_UNDEFINED) != 0)
                    .count()
            })
            .unwrap_or(0);
        println!("  autoload:    {} pending", autoload_count);
        println!();

        // --- Caches (rkyv-mmapped) ---
        // Per docs/DESIGN_GOALS.md:13 and docs/DAEMON.md:226, the only
        // shell cache layer is rkyv-mmapped bytecode under
        // `~/.zshrs/images/` with the top-level `~/.zshrs/index.rkyv`
        // (fq_name → shard_id, generation, byte_offset). Hot lookups
        // never hit SQLite — clients mmap rkyv exclusively.
        println!("{}", bold("Caches (rkyv-mmapped)"));
        let zshrs_dir = dirs::home_dir()
            .map(|h| h.join(".zshrs"))
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp/.zshrs"));
        let index_rkyv = zshrs_dir.join("index.rkyv");
        if index_rkyv.exists() {
            let size = std::fs::metadata(&index_rkyv).map(|m| m.len()).unwrap_or(0);
            println!(
                "  index:       {} {}  {}",
                index_rkyv.display(),
                format_bytes(size),
                green("OK")
            );
        } else {
            println!(
                "  index:       {} {}",
                index_rkyv.display(),
                yellow("(absent — daemon not built shards yet)")
            );
        }
        let images_dir = zshrs_dir.join("images");
        if images_dir.is_dir() {
            let mut shards: Vec<(String, u64)> = Vec::new();
            if let Ok(rd) = std::fs::read_dir(&images_dir) {
                for entry in rd.flatten() {
                    let p = entry.path();
                    if p.extension().and_then(|e| e.to_str()) == Some("rkyv") {
                        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                        let name = p
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("")
                            .to_string();
                        shards.push((name, size));
                    }
                }
            }
            shards.sort();
            let total: u64 = shards.iter().map(|(_, s)| *s).sum();
            println!(
                "  images/:     {} shards, {} total",
                shards.len(),
                format_bytes(total)
            );
            for (name, size) in &shards {
                println!("               {} {}", format_bytes(*size), name);
            }
        } else {
            println!(
                "  images/:     {} {}",
                images_dir.display(),
                yellow("(absent)")
            );
        }

        // Recording staleness — startup replays the recorder shard and
        // ignores rc files, so an edited `.zshrc` is invisible until
        // `zshrs-recorder` re-runs. Same oracle as the `--doctor` flag +
        // startup log: newest rc mtime vs newest `*-recorder.rkyv` mtime.
        if !crate::daemon_presence::recording_present() {
            println!("  recording:   {}", dim("none (rc files sourced normally)"));
        } else {
            match crate::daemon_presence::recording_staleness() {
                Some(rc) => println!(
                    "  recording:   {} ({} is newer — run `zshrs-recorder`)",
                    yellow("STALE"),
                    rc,
                ),
                None => println!("  recording:   {}", green("up to date")),
            }
        }

        // Legacy single-file shards still in active shell-side use until
        // they migrate under images/ in the daemon hydration path.
        if let Some((count, bytes)) = crate::script_cache::stats() {
            let path = crate::script_cache::default_cache_path();
            println!(
                "  scripts:     {} entries, {}  {}",
                count,
                format_bytes(bytes as u64),
                dim(&format!("{}", path.display()))
            );
        }
        let autoload_count = crate::autoload_cache::entry_count();
        if autoload_count > 0 {
            let path = crate::autoload_cache::default_cache_path();
            println!(
                "  autoloads:   {} functions  {}",
                autoload_count,
                dim(&format!("{}", path.display()))
            );
        }

        // Bytecode coverage diagnostic: how many compsys-autoloaded
        // functions have a parsed body in the SQLite inspection mirror
        // but no compiled bytecode blob in the rkyv shard yet. A
        // healthy daemon-hydrated cache reports 0 missing.
        if let Some(cache) = self.compsys_cache() {
            if let Ok(total_bodies) = cache.count_autoloads_with_body() {
                let missing = total_bodies.saturating_sub(autoload_count);
                if missing == 0 {
                    println!(
                        "  coverage:    {}",
                        green("all autoload bodies have bytecode")
                    );
                } else {
                    println!(
                        "  coverage:    {} {}",
                        missing,
                        yellow("autoload bodies missing bytecode")
                    );
                }
            }
        }
        println!();

        // --- SQLite (read-only mirrors) ---
        // Same directory, different job: daemon-maintained copies you
        // can query with SQL or `dbview`. They are NOT the bytecode
        // cache and are NOT read when deciding cache hit/miss or when
        // running compiled code. The numbers below are inspection-only.
        println!("{}", bold("SQLite (read-only mirrors)"));
        println!(
            "  {}",
            dim("daemon-maintained; not read on cache lookup / hot path")
        );
        if let Some(cache) = self.compsys_cache() {
            let count = crate::compsys::cache_entry_count(cache);
            println!("  compsys:     {} completions  {}", count, dim("mirror"));
        } else {
            println!("  compsys:     {}", yellow("no mirror"));
        }
        if let Some(ref cache) = self.plugin_cache {
            let (plugins, functions) = cache.stats();
            println!(
                "  plugins:     {} plugins, {} functions  {}",
                plugins,
                functions,
                dim("mirror")
            );
        } else {
            println!("  plugins:     {}", yellow("no mirror"));
        }
        println!();

        // --- History ---
        // History is not a cache; it's a durable command record. Kept
        // here because the doctor previously buried it under the
        // (mis-labeled) "SQLite Caches" header.
        println!("{}", bold("History"));
        if let Some(engine) = self.history() {
            let count = engine.count().unwrap_or(0);
            println!("  entries:     {}  {}", count, green("OK"));
        } else {
            println!("  entries:     {}", yellow("not initialized"));
        }
        println!();

        // --- Shell State ---
        println!("{}", bold("Shell State"));
        println!("  aliases:     {}", self.alias_entries().len());
        println!(
            "  global:      {} aliases",
            self.global_alias_entries().len()
        );
        println!(
            "  suffix:      {} aliases",
            self.suffix_alias_entries().len()
        );
        println!(
            "  variables:   {}",
            crate::ported::params::paramtab()
                .read()
                .map(|t| t.iter().filter(|(_, p)| p.u_arr.is_none()).count())
                .unwrap_or(0)
        );
        println!(
            "  arrays:      {}",
            crate::ported::params::paramtab()
                .read()
                .map(|t| t.iter().filter(|(_, p)| p.u_arr.is_some()).count())
                .unwrap_or(0)
        );
        println!(
            "  assoc:       {}",
            crate::ported::params::paramtab_hashed_storage()
                .lock()
                .map(|m| m.len())
                .unwrap_or(0)
        );
        println!(
            "  options:     {} set",
            crate::ported::options::opt_state_snapshot()
                .iter()
                .filter(|(_, v)| **v)
                .count()
        );
        println!(
            "  traps:       {} active",
            crate::ported::builtin::traps_table()
                .lock()
                .map(|t| t.len())
                .unwrap_or(0)
        );
        // Count entries across all `<hook>_functions` arrays in paramtab.
        let hook_count: usize = [
            "chpwd",
            "precmd",
            "preexec",
            "periodic",
            "zshexit",
            "zshaddhistory",
        ]
        .iter()
        .map(|h| {
            self.array(&format!("{}_functions", h))
                .map_or(0, |a| a.len())
        })
        .sum();
        println!("  hooks:       {} registered", hook_count);
        println!();

        // --- Log ---
        println!("{}", bold("Log"));
        let log_path = crate::log::log_path();
        if log_path.exists() {
            let size = std::fs::metadata(&log_path).map(|m| m.len()).unwrap_or(0);
            println!("  {}  {} bytes", log_path.display(), size);
        } else {
            println!("  {}", dim("no log file yet"));
        }
        println!();

        // --- Profiling ---
        println!("{}", bold("Profiling"));
        println!(
            "  chrome tracing: {}",
            if crate::log::profiling_enabled() {
                green("enabled")
            } else {
                dim("disabled")
            }
        );
        println!(
            "  flamegraph:     {}",
            if crate::log::flamegraph_enabled() {
                green("enabled")
            } else {
                dim("disabled")
            }
        );
        println!(
            "  prometheus:     {}",
            if crate::log::prometheus_enabled() {
                green("enabled")
            } else {
                dim("disabled")
            }
        );
        println!();

        0
    }

    /// provenance — value lineage over bytecode execution.
    ///
    /// Usage:
    ///   provenance                  — list every tracked parameter and function
    ///   provenance NAME             — print NAME's lineage
    ///   provenance -m NAME...       — start tracking NAME
    ///   provenance -u NAME...       — stop tracking NAME, drop its lineage
    ///   provenance -j NAME          — print NAME's lineage as JSON
    ///   provenance -f ...           — act on shell functions instead of parameters
    ///   provenance -a               — track everything from here on
    ///   provenance -ua              — stop tracking everything (keeps what is recorded)
    ///   provenance -c               — clear every lineage and disarm
    ///
    /// Flags bundle: `-mf NAME` arms a function, `-jf NAME` prints one
    /// as JSON.
    ///
    /// The engine records nothing until the first `-m` or `-a` (or
    /// `[provenance] track_all` at startup), and refuses to arm at all
    /// when `[provenance] enabled = false` in `~/.zshrs/zshrs.toml` or
    /// `ZSHRS_PROVENANCE=0` is set.
    pub(crate) fn builtin_provenance(&self, args: &[String]) -> i32 {
        use crate::provenance;

        // ── flags, then names ───────────────────────────────────────
        let (mut arm, mut disarm, mut json, mut list, mut clear) =
            (false, false, false, false, false);
        let (mut funcs, mut all) = (false, false);
        let mut names: Vec<&String> = Vec::new();
        let mut rest = args.iter();
        for a in rest.by_ref() {
            if a == "--" {
                break;
            }
            if a.len() > 1 && a.starts_with('-') {
                for c in a.chars().skip(1) {
                    match c {
                        'm' => arm = true,
                        'u' => disarm = true,
                        'j' => json = true,
                        'l' => list = true,
                        'c' => clear = true,
                        'f' => funcs = true,
                        'a' => all = true,
                        _ => {
                            eprintln!("zshrs: provenance: bad option: -{}", c);
                            return 1;
                        }
                    }
                }
                continue;
            }
            names.push(a);
            break;
        }
        names.extend(rest);

        // ── the two namespaces, behind one set of verbs ─────────────
        // `-f` picks the function namespace explicitly. WITHOUT it a bare
        // name reads whichever namespace holds it, mirroring the way `-m NAME`
        // arms whichever the name actually is: arming a function with a bare
        // `provenance -m ff` and then reading it back with a bare
        // `provenance ff` has to work, or the two halves of the same spelling
        // disagree. A parameter still wins when both are tracked.
        let tracked_as_func = |name: &str| !funcs && provenance::lookup_name(name).is_none();
        let lookup = |name: &str| {
            if funcs {
                provenance::lookup_func(name)
            } else {
                provenance::lookup_name(name).or_else(|| provenance::lookup_func(name))
            }
        };
        let label = |name: &str| {
            if funcs || tracked_as_func(name) {
                format!("{}()", name)
            } else {
                name.to_string()
            }
        };
        let print_all = || {
            for name in provenance::tracked_names() {
                match provenance::lookup_name(&name) {
                    Some(node) => print!("{}", provenance::render(&name, &node)),
                    None => println!("{}", name),
                }
            }
            for name in provenance::tracked_func_names() {
                match provenance::lookup_func(&name) {
                    Some(node) => print!("{}", provenance::render(&format!("{}()", name), &node)),
                    None => println!("{}()", name),
                }
            }
            let dropped = provenance::auto_dropped();
            if dropped > 0 {
                println!(
                    "… {} more names not tracked (cap {})",
                    dropped,
                    provenance::MAX_AUTO_NAMES
                );
            }
            0
        };

        if clear {
            provenance::clear();
            return 0;
        }

        // `-a` arms everything; `-ua` stops arming new names.
        if all {
            if disarm {
                provenance::set_track_all(false);
                return 0;
            }
            if !provenance::set_track_all(true) {
                eprintln!("zshrs: provenance: disabled by config");
                return 1;
            }
            return 0;
        }

        if arm {
            if !provenance::enabled() {
                eprintln!("zshrs: provenance: disabled by config");
                return 1;
            }
            if names.is_empty() {
                eprintln!(
                    "zshrs: provenance: -m: missing {} name",
                    if funcs { "function" } else { "parameter" }
                );
                return 1;
            }
            for name in names {
                if funcs {
                    let (file, line, body) = Self::shfunc_def_site(name);
                    provenance::track_func(name, body.as_deref(), file.as_deref(), line);
                } else {
                    let current = crate::ported::params::getsparam(name);
                    // Follow the name to whatever it actually IS. Without this,
                    // `provenance -m ff` on a shell function armed a PARAMETER
                    // named `ff` — an entry that can never record, because
                    // nothing writes a parameter by that name. It then listed as
                    // a bare `ff` with no origin and no ops no matter how many
                    // times the function was defined or called, which reads as
                    // "provenance is broken" rather than "you wanted -f".
                    // An explicit `-f` still forces the function reading, and a
                    // real parameter still wins when both exist.
                    if current.is_none() && crate::ported::utils::getshfunc(name).is_some() {
                        let (file, line, body) = Self::shfunc_def_site(name);
                        provenance::track_func(name, body.as_deref(), file.as_deref(), line);
                    } else {
                        provenance::track_name(name, current.as_deref());
                    }
                }
            }
            return 0;
        }

        if disarm {
            if names.is_empty() {
                eprintln!(
                    "zshrs: provenance: -u: missing {} name",
                    if funcs { "function" } else { "parameter" }
                );
                return 1;
            }
            let mut status = 0;
            for name in names {
                let dropped = if funcs {
                    provenance::untrack_func(name)
                } else {
                    // Mirror the `-m` rule above: a bare `-u NAME` drops
                    // whichever of the two `-m NAME` would have armed.
                    provenance::untrack_name(name) || provenance::untrack_func(name)
                };
                if !dropped {
                    eprintln!("zshrs: provenance: not tracked: {}", label(name));
                    status = 1;
                }
            }
            return status;
        }

        if json {
            let Some(name) = names.first() else {
                eprintln!(
                    "zshrs: provenance: -j: missing {} name",
                    if funcs { "function" } else { "parameter" }
                );
                return 1;
            };
            return match lookup(name) {
                Some(node) => {
                    println!("{}", provenance::render_json(&label(name), &node));
                    0
                }
                None => {
                    eprintln!("zshrs: provenance: not tracked: {}", label(name));
                    1
                }
            };
        }

        if list || names.is_empty() {
            return print_all();
        }

        let mut status = 0;
        for name in names {
            match lookup(name) {
                Some(node) => print!("{}", provenance::render(&label(name), &node)),
                None => {
                    eprintln!("zshrs: provenance: not tracked: {}", label(name));
                    status = 1;
                }
            }
        }
        status
    }

    /// Defining file and line of shell function `name`, as `shfunctab`
    /// recorded them at definition time (`Src/exec.c:5383-5388`).
    /// `(None, 0)` when the function does not exist.
    /// Definition site AND current body of a shell function. The body is
    /// what `provenance -m` seeds the chain's origin with — arming an
    /// already-defined function never reaches `on_func_define`, so
    /// without it the origin can never show what the body was.
    fn shfunc_def_site(name: &str) -> (Option<String>, i64, Option<String>) {
        crate::ported::hashtable::shfunctab_lock()
            .read()
            .ok()
            .and_then(|t| {
                t.get_including_disabled(name)
                    .map(|f| (f.filename.clone(), f.lineno, f.body.clone()))
            })
            .unwrap_or((None, 0, None))
    }

    /// dbview — browse zshrs SQLite cache tables without SQL.
    ///
    /// Usage:
    ///   dbview                      — list all tables and row counts
    ///   dbview autoloads             — dump autoloads table (name, source, body len, ast len)
    ///   dbview autoloads _git        — show single row by name
    ///   dbview comps                 — dump comps table
    ///   dbview history               — recent history entries
    ///   dbview history <pattern>     — search history
    ///   dbview plugins               — plugin cache entries
    ///   dbview executables            — PATH executables cache
    ///   dbview <table> --count       — just the count
    pub(crate) fn builtin_dbview(&self, args: &[String]) -> i32 {
        let bold = |s: &str| format!("\x1b[1m{}\x1b[0m", s);
        let dim = |s: &str| format!("\x1b[2m{}\x1b[0m", s);
        let cyan = |s: &str| format!("\x1b[36m{}\x1b[0m", s);
        let green = |s: &str| format!("\x1b[32m{}\x1b[0m", s);
        let yellow = |s: &str| format!("\x1b[33m{}\x1b[0m", s);

        if args.is_empty() {
            // List all tables with row counts
            println!("{}", bold("zshrs SQLite caches"));
            println!();

            if let Some(cache) = self.compsys_cache() {
                println!("  {} {}", bold("compsys.db"), dim("(completion cache)"));
                if let Ok(n) = cache.count_table("autoloads") {
                    let bc_count = cache
                        .count_table_where("autoloads", "bytecode IS NOT NULL")
                        .unwrap_or(0);
                    println!("    autoloads:    {:>6} rows  ({} compiled)", n, bc_count);
                }
                if let Ok(n) = cache.count_table("comps") {
                    println!("    comps:        {:>6} rows", n);
                }
                if let Ok(n) = cache.count_table("services") {
                    println!("    services:     {:>6} rows", n);
                }
                if let Ok(n) = cache.count_table("patcomps") {
                    println!("    patcomps:     {:>6} rows", n);
                }
                if let Ok(n) = cache.count_table("executables") {
                    println!("    executables:  {:>6} rows", n);
                }
                if let Ok(n) = cache.count_table("zstyles") {
                    println!("    zstyles:      {:>6} rows", n);
                }
                println!();
            }

            if let Some(engine) = self.history() {
                println!("  {} {}", bold("history.db"), dim("(command history)"));
                if let Ok(n) = engine.count() {
                    println!("    entries:      {:>6} rows", n);
                }
                println!();
            }

            if let Some(ref cache) = self.plugin_cache {
                let (plugins, functions) = cache.stats();
                println!("  {} {}", bold("plugins.db"), dim("(plugin source cache)"));
                println!("    plugins:      {:>6} rows", plugins);
                println!("    functions:    {:>6} rows", functions);
                println!();
            }

            println!("  Usage: {} <table> [name] [--count]", cyan("dbview"));
            return 0;
        }

        let table = args[0].as_str();
        let filter = args.get(1).map(|s| s.as_str());
        let count_only = args.iter().any(|a| a == "--count" || a == "-c");

        match table {
            "autoloads" => {
                let Some(cache) = self.compsys_cache() else {
                    eprintln!("zshrs:dbview:1: no compsys cache");
                    return 1;
                };

                if count_only {
                    let n = cache.count_table("autoloads").unwrap_or(0);
                    println!("{}", n);
                    return 0;
                }

                if let Some(name) = filter {
                    // Single row lookup
                    match cache.get_autoload(name) {
                        Ok(Some(stub)) => {
                            println!("{}", bold(&format!("autoload: {}", name)));
                            println!("  source:   {}", stub.source);
                            println!(
                                "  body:     {} bytes",
                                stub.body.as_ref().map(|b| b.len()).unwrap_or(0)
                            );
                            match crate::autoload_cache::try_load(name) {
                                Some(blob) => {
                                    println!("  bytecode: {} {} bytes", green("YES"), blob.len())
                                }
                                None => println!("  bytecode: {}", yellow("NULL")),
                            }
                            // Show first few lines of body
                            if let Some(ref body) = stub.body {
                                println!("  preview:");
                                for (i, line) in body.lines().take(10).enumerate() {
                                    println!("    {:>3}: {}", i + 1, dim(line));
                                }
                                let total = body.lines().count();
                                if total > 10 {
                                    println!("    {} ({} more lines)", dim("..."), total - 10);
                                }
                            }
                        }
                        _ => {
                            eprintln!("zshrs:dbview:1: autoload '{}' not found", name);
                            return 1;
                        }
                    }
                    return 0;
                }

                // Dump all autoloads
                let conn = &cache.conn();
                match conn.prepare("SELECT name, source, length(body), length(bytecode) FROM autoloads ORDER BY name LIMIT 200") {
                    Ok(mut stmt) => {
                        let rows = stmt.query_map([], |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, Option<i64>>(2)?,
                                row.get::<_, Option<i64>>(3)?,
                            ))
                        });
                        if let Ok(rows) = rows {
                            println!("{:<40} {:>8} {:>8}  {}", bold("NAME"), bold("BODY"), bold("BYTECODE"), bold("SOURCE"));
                            let mut count = 0;
                            for row in rows.flatten() {
                                let (name, source, body_len, ast_len) = row;
                                let ast_str = match ast_len {
                                    Some(n) => green(&format!("{:>8}", n)),
                                    None => yellow(&format!("{:>8}", "NULL")),
                                };
                                let body_str = match body_len {
                                    Some(n) => format!("{:>8}", n),
                                    None => dim("NULL").to_string(),
                                };
                                // Truncate source path for display
                                let src_short = if source.len() > 50 {
                                    format!("...{}", &source[source.len() - 47..])
                                } else {
                                    source
                                };
                                println!("{:<40} {} {}  {}", name, body_str, ast_str, dim(&src_short));
                                count += 1;
                            }
                            println!("\n{} rows shown (LIMIT 200)", count);
                        }
                    }
                    Err(e) => {
                        eprintln!("zshrs:dbview:1: query failed: {}", e);
                        return 1;
                    }
                }
            }

            "comps" => {
                let Some(cache) = self.compsys_cache() else {
                    eprintln!("zshrs:dbview:1: no compsys cache");
                    return 1;
                };
                if count_only {
                    println!("{}", cache.count_table("comps").unwrap_or(0));
                    return 0;
                }
                let conn = cache.conn();
                let query = if let Some(pat) = filter {
                    format!("SELECT command, function FROM comps WHERE command LIKE '%{}%' ORDER BY command LIMIT 100", pat)
                } else {
                    "SELECT command, function FROM comps ORDER BY command LIMIT 100".to_string()
                };
                match conn.prepare(&query) {
                    Ok(mut stmt) => {
                        println!("{:<40} {}", bold("COMMAND"), bold("FUNCTION"));
                        let rows = stmt.query_map([], |row| {
                            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                        });
                        if let Ok(rows) = rows {
                            for row in rows.flatten() {
                                println!("{:<40} {}", row.0, cyan(&row.1));
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("zshrs:dbview:1: {}", e);
                        return 1;
                    }
                }
            }

            "executables" => {
                let Some(cache) = self.compsys_cache() else {
                    eprintln!("zshrs:dbview:1: no compsys cache");
                    return 1;
                };
                if count_only {
                    println!("{}", cache.count_table("executables").unwrap_or(0));
                    return 0;
                }
                let conn = cache.conn();
                let query = if let Some(pat) = filter {
                    format!("SELECT name, path FROM executables WHERE name LIKE '%{}%' ORDER BY name LIMIT 100", pat)
                } else {
                    "SELECT name, path FROM executables ORDER BY name LIMIT 100".to_string()
                };
                match conn.prepare(&query) {
                    Ok(mut stmt) => {
                        println!("{:<30} {}", bold("NAME"), bold("PATH"));
                        let rows = stmt.query_map([], |row| {
                            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                        });
                        if let Ok(rows) = rows {
                            for row in rows.flatten() {
                                println!("{:<30} {}", row.0, dim(&row.1));
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("zshrs:dbview:1: {}", e);
                        return 1;
                    }
                }
            }

            "history" => {
                let Some(engine) = self.history() else {
                    eprintln!("zshrs:dbview:1: no history engine");
                    return 1;
                };
                if count_only {
                    println!("{}", engine.count().unwrap_or(0));
                    return 0;
                }
                if let Some(pat) = filter {
                    if let Ok(entries) = engine.search(pat, 20) {
                        for e in entries {
                            println!(
                                "  {} {} {}",
                                dim(&e.timestamp.to_string()),
                                cyan(&e.command),
                                dim(&format!("[{}]", e.exit_code.unwrap_or(0)))
                            );
                        }
                    }
                } else if let Ok(entries) = engine.recent(20) {
                    for e in entries {
                        println!(
                            "  {} {} {}",
                            dim(&e.timestamp.to_string()),
                            cyan(&e.command),
                            dim(&format!("[{}]", e.exit_code.unwrap_or(0)))
                        );
                    }
                }
            }

            "plugins" => {
                let Some(ref cache) = self.plugin_cache else {
                    eprintln!("zshrs:dbview:1: no plugin cache");
                    return 1;
                };
                let (plugins, functions) = cache.stats();
                println!("{} plugins, {} cached functions", plugins, functions);
            }

            _ => {
                eprintln!("zshrs:dbview:1: unknown table '{}'. Available: autoloads, comps, executables, history, plugins", table);
                return 1;
            }
        }

        0
    }

    /// profile — in-process command profiling with nanosecond accuracy.
    ///
    /// Unlike `time` (which measures one command) or `zprof` (which only
    /// profiles function calls), `profile` traces every execute_command,
    /// expansion, glob, and builtin dispatch inside the block.
    ///
    /// Usage:
    ///   profile { commands }     — profile a block
    ///   profile -s 'script'     — profile a script string
    ///   profile -f func         — profile a function call
    ///   profile --clear         — clear accumulated profile data
    ///   profile --dump          — show accumulated profile data
    pub(crate) fn builtin_profile(&mut self, args: &[String]) -> i32 {
        let bold = |s: &str| format!("\x1b[1m{}\x1b[0m", s);
        let dim = |s: &str| format!("\x1b[2m{}\x1b[0m", s);
        let cyan = |s: &str| format!("\x1b[36m{}\x1b[0m", s);
        let yellow = |s: &str| format!("\x1b[33m{}\x1b[0m", s);

        if args.is_empty() {
            println!("Usage: profile {{ commands }}");
            println!("       profile -s 'script string'");
            println!("       profile -f function_name [args...]");
            println!("       profile --clear");
            println!("       profile --dump");
            return 0;
        }

        if args[0] == "--clear" {
            // Route through canonical dispatch_builtin → BUILTINS["zprof"]
            // (zprof.c:139 entry) with the `-c` flag — clears
            // CALLS/NCALLS/ARCS/NARCS tables per c:141-147.
            crate::fusevm_bridge::dispatch_builtin("zprof", vec!["-c".to_string()]);
            println!("profile data cleared");
            return 0;
        }

        if args[0] == "--dump" {
            // bin_zprof prints to stdout directly (matches C
            // zprof.c:170+ printf chain); on empty state nothing is
            // emitted, so check NCALLS for the "no data" hint.
            if crate::zprof::NCALLS.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                println!("{}", dim("no profile data"));
            } else {
                crate::fusevm_bridge::dispatch_builtin("zprof", Vec::new());
            }
            return 0;
        }

        // Determine what to profile
        let code = if args[0] == "-s" {
            // profile -s 'script string'
            if args.len() < 2 {
                eprintln!("zshrs:profile:1: -s requires a script string");
                return 1;
            }
            args[1..].join(" ")
        } else if args[0] == "-f" {
            // profile -f func_name [args...]
            if args.len() < 2 {
                eprintln!("zshrs:profile:1: -f requires a function name");
                return 1;
            }
            args[1..].join(" ")
        } else {
            // profile { commands } — args is the block body
            args.join(" ")
        };

        // Enable profiling, run, collect results
        let was_enabled = self.profiling_enabled;
        self.profiling_enabled = true;
        // Reset zprof state through canonical dispatch_builtin so the
        // module-level CALLS/NCALLS/ARCS/NARCS tables start fresh.
        crate::fusevm_bridge::dispatch_builtin("zprof", vec!["-c".to_string()]);

        let t0 = std::time::Instant::now();
        let result = self.execute_script(&code);
        let elapsed = t0.elapsed();
        let status = match result {
            Ok(s) => s,
            Err(e) => {
                eprintln!("zshrs:profile:1: {}", e);
                1
            }
        };

        // Collect timing data
        println!();
        println!("{}", bold("profile results"));
        println!("{}", dim(&"─".repeat(60)));
        let dur_str = if elapsed.as_secs() > 0 {
            format!("{:.3}s", elapsed.as_secs_f64())
        } else if elapsed.as_millis() > 0 {
            format!("{:.3}ms", elapsed.as_secs_f64() * 1000.0)
        } else {
            format!("{:.1}µs", elapsed.as_secs_f64() * 1_000_000.0)
        };
        println!("  total:     {}", cyan(&dur_str));
        println!("  status:    {}", status);
        println!();

        // Show function-level breakdown from zprof (printed inline by
        // the bin_zprof body — c:170+ printf chain). Suppress when no
        // calls were profiled.
        if crate::zprof::NCALLS.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            println!("{}", bold("function breakdown"));
            crate::fusevm_bridge::dispatch_builtin("zprof", Vec::new());
        }

        // Per-command breakdown from tracing (if tracing is at debug level)
        println!();
        println!(
            "  {} set ZSHRS_LOG=trace for per-command tracing",
            yellow("tip:")
        );
        println!(
            "  {} output: {}",
            dim("log"),
            dim(&crate::log::log_path().display().to_string())
        );

        self.profiling_enabled = was_enabled;
        status
    }

    /// intercept builtin — register AOP advice on commands.
    ///
    /// Usage:
    ///   intercept before <pattern> { code }
    ///   intercept after <pattern> { code }
    ///   intercept around <pattern> { code }
    ///   intercept list                       — show all intercepts
    ///   intercept remove <id>                — remove by ID
    ///   intercept clear                      — remove all
    pub(crate) fn builtin_intercept(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            println!("Usage: intercept <before|after|around> <pattern> {{ code }}");
            println!("       intercept list | remove <id> | clear");
            return 0;
        }

        match args[0].as_str() {
            "list" => {
                if self.intercepts.is_empty() {
                    println!("no intercepts registered");
                } else {
                    let bold = |s: &str| format!("\x1b[1m{}\x1b[0m", s);
                    let cyan = |s: &str| format!("\x1b[36m{}\x1b[0m", s);
                    println!(
                        "{:>4}  {:<8}  {:<20}  {}",
                        bold("ID"),
                        bold("KIND"),
                        bold("PATTERN"),
                        bold("CODE")
                    );
                    for i in &self.intercepts {
                        let kind = match i.kind {
                            AdviceKind::Before => "before",
                            AdviceKind::After => "after",
                            AdviceKind::Around => "around",
                        };
                        let code_preview = if i.code.len() > 40 {
                            format!("{}...", &i.code[..37])
                        } else {
                            i.code.clone()
                        };
                        println!(
                            "{:>4}  {:<8}  {:<20}  {}",
                            cyan(&i.id.to_string()),
                            kind,
                            i.pattern,
                            code_preview
                        );
                    }
                }
                0
            }
            "clear" => {
                let count = self.intercepts.len();
                self.intercepts.clear();
                println!("cleared {} intercepts", count);
                0
            }
            "remove" => {
                if args.len() < 2 {
                    eprintln!("intercept remove: requires ID");
                    return 1;
                }
                if let Ok(id) = args[1].parse::<u32>() {
                    let before = self.intercepts.len();
                    self.intercepts.retain(|i| i.id != id);
                    if self.intercepts.len() < before {
                        println!("removed intercept {}", id);
                        0
                    } else {
                        eprintln!("zshrs:intercept:1: no intercept with ID {}", id);
                        1
                    }
                } else {
                    eprintln!("intercept remove: invalid ID");
                    1
                }
            }
            "before" | "after" | "around" => {
                let kind = match args[0].as_str() {
                    "before" => AdviceKind::Before,
                    "after" => AdviceKind::After,
                    "around" => AdviceKind::Around,
                    _ => unreachable!(),
                };

                if args.len() < 3 {
                    eprintln!("intercept {}: requires <pattern> {{ code }}", args[0]);
                    return 1;
                }

                let pattern = args[1].clone();
                // Join remaining args as the code (handles { code } or 'code')
                let code = args[2..].join(" ");
                // Strip surrounding braces if present
                let code = code.trim().to_string();
                let code = if code.starts_with('{') && code.ends_with('}') {
                    code[1..code.len() - 1].trim().to_string()
                } else {
                    code
                };

                let id = self.intercepts.iter().map(|i| i.id).max().unwrap_or(0) + 1;
                self.intercepts.push(Intercept {
                    pattern,
                    kind: kind.clone(),
                    code: code.clone(),
                    id,
                });

                let kind_str = match kind {
                    AdviceKind::Before => "before",
                    AdviceKind::After => "after",
                    AdviceKind::Around => "around",
                };
                // Registration is not user-requested output. A `.zshrc` that
                // arms a handful of intercepts printed a banner line per
                // registration on every shell start, which is exactly the
                // startup chatter the project forbids. `intercept list` is
                // the way to see what is registered.
                tracing::info!(
                    id,
                    kind = kind_str,
                    pattern = %self.intercepts.last().unwrap().pattern,
                    code = %code,
                    "intercept registered"
                );
                0
            }
            _ => {
                eprintln!(
                    "intercept: unknown subcommand '{}'. Use before|after|around|list|remove|clear",
                    args[0]
                );
                1
            }
        }
    }

    /// intercept_proceed — called from around advice to execute the original command.
    pub(crate) fn builtin_intercept_proceed(&mut self, _args: &[String]) -> i32 {
        self.set_scalar("__intercept_proceed".to_string(), "1".to_string());
        // Run the original command using saved INTERCEPT_NAME/INTERCEPT_ARGS
        let cmd_name = self.scalar("INTERCEPT_NAME").unwrap_or_default();
        let args_str = self.scalar("INTERCEPT_ARGS").unwrap_or_default();
        let args: Vec<String> = if args_str.is_empty() {
            Vec::new()
        } else {
            args_str.split_whitespace().map(|s| s.to_string()).collect()
        };
        match self.run_original_command(&cmd_name, &args) {
            Ok(status) => status,
            Err(e) => {
                eprintln!("zshrs:intercept_proceed:1: {}", e);
                1
            }
        }
    }

    /// async { cmd } — run command on worker pool, return job ID immediately.
    /// Output captured in background, retrieve with `await $id`.
    ///
    /// Usage:
    ///   id=$(async 'sleep 2; echo done')
    ///   ... do other work ...
    ///   result=$(await $id)
    pub(crate) fn builtin_async(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            eprintln!("zshrs:async:1: requires a command string");
            return 1;
        }

        let code = args.join(" ");
        let id = self.next_async_id;
        self.next_async_id += 1;

        let (tx, rx) = crossbeam_channel::bounded::<(i32, String)>(1);
        let pool = std::sync::Arc::clone(&self.worker_pool);

        pool.submit(move || {
            // Execute in a subprocess to capture stdout
            let output = Command::new("sh")
                .args(["-c", &code])
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .output();
            match output {
                Ok(out) => {
                    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                    let status = out.status.code().unwrap_or(1);
                    let _ = tx.send((status, stdout));
                }
                Err(_) => {
                    let _ = tx.send((127, String::new()));
                }
            }
        });

        self.async_jobs.insert(id, rx);
        // Print the job ID so it can be captured: id=$(async 'cmd')
        println!("{}", id);
        0
    }

    /// await $id — block until async job completes, print its stdout, return its status.
    ///
    /// Usage:
    ///   id=$(async 'expensive_command')
    ///   await $id    # blocks until done, prints output
    ///   echo $?      # exit status of the async command
    pub(crate) fn builtin_await(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            eprintln!("zshrs:await:1: requires a job ID");
            return 1;
        }

        let id: u32 = match args[0].parse() {
            Ok(n) => n,
            Err(_) => {
                eprintln!("zshrs:await:1: invalid job ID '{}'", args[0]);
                return 1;
            }
        };

        let rx = match self.async_jobs.remove(&id) {
            Some(rx) => rx,
            None => {
                eprintln!("zshrs:await:1: no async job with ID {}", id);
                return 1;
            }
        };

        // Block until the job completes
        match rx.recv() {
            Ok((status, stdout)) => {
                if !stdout.is_empty() {
                    print!("{}", stdout);
                }
                self.set_last_status(status);
                status
            }
            Err(_) => {
                eprintln!("zshrs:await:1: job {} died without result", id);
                1
            }
        }
    }

    /// pmap 'cmd {}' arg1 arg2 arg3 — parallel map across worker pool.
    /// Runs `cmd` for each argument, replacing `{}` with the argument.
    /// Output is collected in order. Returns 0 if all succeed.
    ///
    /// Usage:
    ///   pmap 'gzip {}' *.log
    ///   pmap 'echo {}' a b c d
    ///   ls *.rs | pmap 'wc -l {}'
    pub(crate) fn builtin_pmap(&mut self, args: &[String]) -> i32 {
        if args.len() < 2 {
            eprintln!("zshrs:pmap:1: requires 'command {{}}' followed by arguments");
            return 1;
        }

        let template = &args[0];
        let items: Vec<String> = args[1..].to_vec();

        // Substitute `{}` with `${=__ZSHRS_P_ARG__}` ONCE — the `=`
        // flag forces word-splitting on $IFS so an item like "a b"
        // still expands to two arguments at use site. Per-item the
        // subprocess receives the item via the `__ZSHRS_P_ARG__` env
        // var so the template's expansion picks it up natively.
        const ARG_ENV: &str = "__ZSHRS_P_ARG__";
        let parametrised = template.replace("{}", &format!("${{={}}}", ARG_ENV));

        // Resolve our own binary once so every parallel subprocess
        // runs zshrs (NOT /bin/sh) — preserves zshrs-specific
        // builtins (`echo` flags, `print -P`, etc.).
        let exe = match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("zshrs:pmap:1: current_exe: {}", e);
                return 1;
            }
        };

        use rayon::prelude::*;
        // Run all items in parallel via rayon's work-stealing pool.
        // `par_iter().collect()` preserves the input order in the
        // result Vec — so pmap's documented "output collected in
        // order" guarantee holds across the parallel run. The
        // subprocess inherits our env (so $PATH etc. work) plus
        // gets ARG_ENV=item.
        let results: Vec<(i32, Vec<u8>, Vec<u8>)> = items
            .par_iter()
            .map(|item| {
                let output = Command::new(&exe)
                    .args(["--zsh", "-c", &parametrised])
                    .env(ARG_ENV, item)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .output();
                match output {
                    Ok(o) => (o.status.code().unwrap_or(1), o.stdout, o.stderr),
                    Err(_) => (1, Vec::new(), Vec::new()),
                }
            })
            .collect();

        let mut any_fail = false;
        for (status, stdout, stderr) in results {
            if !stdout.is_empty() {
                let _ = std::io::stdout().write_all(&stdout);
            }
            if !stderr.is_empty() {
                let _ = std::io::stderr().write_all(&stderr);
            }
            if status != 0 {
                any_fail = true;
            }
        }

        if any_fail {
            1
        } else {
            0
        }
    }

    /// pgrep 'pattern' arg1 arg2 ... — parallel grep/filter across worker pool.
    /// Runs the pattern command for each argument, prints args where command succeeds.
    ///
    /// Usage:
    ///   pgrep 'test -f {}' /path/a /path/b /path/c
    ///   pgrep 'grep -q TODO {}' *.rs
    pub(crate) fn builtin_pgrep(&mut self, args: &[String]) -> i32 {
        if args.len() < 2 {
            eprintln!("zshrs:pgrep:1: requires 'test_command {{}}' followed by arguments");
            return 1;
        }

        let template = &args[0];
        let items: Vec<String> = args[1..].to_vec();

        const ARG_ENV: &str = "__ZSHRS_P_ARG__";
        let parametrised = template.replace("{}", &format!("${{={}}}", ARG_ENV));

        let exe = match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("zshrs:pgrep:1: current_exe: {}", e);
                return 1;
            }
        };

        use rayon::prelude::*;
        // Parallel filter: keep input order via `.collect()`, output
        // only items whose test passed (rc=0).
        let pass_flags: Vec<bool> = items
            .par_iter()
            .map(|item| {
                let status = Command::new(&exe)
                    .args(["--zsh", "-c", &parametrised])
                    .env(ARG_ENV, item)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                match status {
                    Ok(s) => s.success(),
                    Err(_) => false,
                }
            })
            .collect();

        for (item, &passed) in items.iter().zip(pass_flags.iter()) {
            if passed {
                println!("{}", item);
            }
        }
        0
    }

    /// peach 'cmd {}' arg1 arg2 ... — parallel for-each, no output ordering.
    /// Like pmap but doesn't collect output — fire-and-forget, print as completed.
    ///
    /// Usage:
    ///   peach 'convert {} {}.png' *.svg
    ///   peach 'rsync -a {} remote:{}' dir1 dir2 dir3
    pub(crate) fn builtin_peach(&mut self, args: &[String]) -> i32 {
        if args.len() < 2 {
            eprintln!("zshrs:peach:1: requires 'command {{}}' followed by arguments");
            return 1;
        }

        let template = &args[0];
        let items: Vec<String> = args[1..].to_vec();

        const ARG_ENV: &str = "__ZSHRS_P_ARG__";
        let parametrised = template.replace("{}", &format!("${{={}}}", ARG_ENV));

        let exe = match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("zshrs:peach:1: current_exe: {}", e);
                return 1;
            }
        };

        use rayon::prelude::*;
        use std::sync::atomic::AtomicBool;
        // peach: "fire-and-forget, print as completed" (no order
        // guarantee). Inherit stdout/stderr so each subprocess
        // streams to the parent's tty as it runs. par_iter().for_each
        // runs items concurrently; failures are aggregated via an
        // atomic flag.
        let any_fail = AtomicBool::new(false);
        items.par_iter().for_each(|item| {
            let status = Command::new(&exe)
                .args(["--zsh", "-c", &parametrised])
                .env(ARG_ENV, item)
                .status();
            let ok = match status {
                Ok(s) => s.success(),
                Err(_) => false,
            };
            if !ok {
                any_fail.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        });

        if any_fail.load(std::sync::atomic::Ordering::Relaxed) {
            1
        } else {
            0
        }
    }

    /// barrier cmd1 ::: cmd2 ::: cmd3 — run commands in parallel, wait for ALL to complete.
    /// Returns the worst (highest) exit status.
    ///
    /// Usage:
    ///   barrier 'make -C proj1' ::: 'make -C proj2' ::: 'make -C proj3'
    ///   barrier 'npm test' ::: 'cargo test' ::: 'pytest'
    pub(crate) fn builtin_barrier(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            eprintln!("zshrs:barrier:1: requires commands separated by :::");
            return 1;
        }

        // Split on ::: delimiter
        let mut commands: Vec<String> = Vec::new();
        let mut current = String::new();
        for arg in args {
            if arg == ":::" {
                if !current.is_empty() {
                    commands.push(current.trim().to_string());
                    current.clear();
                }
            } else {
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(arg);
            }
        }
        if !current.is_empty() {
            commands.push(current.trim().to_string());
        }

        if commands.is_empty() {
            return 0;
        }

        // Ship all to pool
        let mut receivers = Vec::with_capacity(commands.len());
        for cmd in &commands {
            let cmd = cmd.clone();
            let rx = self.worker_pool.submit_with_result(move || {
                Command::new("sh")
                    .args(["-c", &cmd])
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::inherit())
                    .status()
                    .map(|s| s.code().unwrap_or(1))
                    .unwrap_or(127)
            });
            receivers.push(rx);
        }

        // Wait for all — return worst status
        let mut worst = 0i32;
        for rx in receivers {
            if let Ok(status) = rx.recv() {
                if status > worst {
                    worst = status;
                }
            }
        }

        self.set_last_status(worst);
        worst
    }

    /// help - display help for builtins (bash)
    pub(crate) fn builtin_help(&self, args: &[String]) -> i32 {
        if args.is_empty() {
            println!("zshrs shell builtins:");
            println!();
            println!("  alias, bg, bind, break, builtin, cd, command, continue,");
            println!("  declare, dirs, disown, echo, enable, eval, exec, exit,");
            println!("  export, false, fc, fg, getopts, hash, help, history,");
            println!("  jobs, kill, let, local, logout, popd, printf, pushd,");
            println!("  pwd, read, readonly, return, set, shift, shopt, source,");
            println!("  suspend, test, times, trap, true, type, typeset, ulimit,");
            println!("  umask, unalias, unset, wait, whence, where, which");
            println!();
            println!("Type 'help name' for more information about 'name'.");
            return 0;
        }

        let cmd = &args[0];
        match cmd.as_str() {
            "cd" => println!("cd: cd [-L|-P] [dir]\n    Change the shell working directory."),
            "echo" => println!("echo: echo [-neE] [arg ...]\n    Write arguments to standard output."),
            "export" => println!("export: export [-fn] [name[=value] ...]\n    Set export attribute for shell variables."),
            "alias" => println!("alias: alias [-p] [name[=value] ...]\n    Define or display aliases."),
            "history" => println!("history: history [-c] [-d offset] [n]\n    Display or manipulate the history list."),
            "jobs" => println!("jobs: jobs [-lnprs] [jobspec ...]\n    Display status of jobs."),
            "kill" => println!("kill: kill [-s sigspec | -n signum | -sigspec] pid | jobspec ...\n    Send a signal to a job."),
            "read" => println!("read: read [-ers] [-a array] [-d delim] [-i text] [-n nchars] [-N nchars] [-p prompt] [-t timeout] [-u fd] [name ...]\n    Read a line from standard input."),
            "set" => println!("set: set [-abefhkmnptuvxBCHP] [-o option-name] [--] [arg ...]\n    Set or unset values of shell options and positional parameters."),
            "test" | "[" => println!("test: test [expr]\n    Evaluate conditional expression."),
            "type" => println!("type: type [-afptP] name [name ...]\n    Display information about command type."),
            _ => println!("{}: no help available", cmd),
        }
        0
    }

    /// `add-zsh-hook` builtin. Direct port of the shell function at
    /// `Src/Functions/Misc/add-zsh-hook`, which maintains a paramtab
    /// array per well-known hook (`chpwd_functions`, `precmd_functions`,
    /// `preexec_functions`, `periodic_functions`, `zshexit_functions`,
    /// `zshaddhistory_functions`). This is the SHELL-LEVEL mechanism;
    /// it is distinct from the C-module hookdef chain in
    /// `src/ported/module.rs` (`addhookfunc(name, Hookfn)` writes to
    /// `hooktab` and is consumed by `runhookdef` for C-module
    /// callbacks like BEFORECOMPLETEHOOK / AFTERCOMPLETEHOOK).
    pub(crate) fn builtin_add_zsh_hook(&mut self, args: &[String]) -> i32 {
        // add-zsh-hook [-d] hook function
        if args.len() < 2 {
            eprintln!("usage: add-zsh-hook [-d] hook function");
            return 1;
        }

        let (delete, hook, func) = if args[0] == "-d" {
            if args.len() < 3 {
                eprintln!("usage: add-zsh-hook -d hook function");
                return 1;
            }
            (true, args[1].as_str(), args[2].as_str())
        } else {
            (false, args[0].as_str(), args[1].as_str())
        };

        let array_name = format!("{}_functions", hook);
        if delete {
            if let Some(mut arr) = self.array(&array_name) {
                arr.retain(|f| f != func);
                crate::ported::params::setaparam(&array_name, arr);
            }
        } else {
            let mut arr = self.array(&array_name).unwrap_or_default();
            if !arr.iter().any(|f| f == func) {
                arr.push(func.to_string());
                crate::ported::params::setaparam(&array_name, arr);
            }
        }
        0
    }

    /// Generate completion candidates
    pub(crate) fn builtin_compgen(&self, args: &[String]) -> i32 {
        let mut i = 0;
        let mut prefix = String::new();
        let mut actions = Vec::new();
        let mut wordlist = None;
        let mut globpat = None;

        while i < args.len() {
            match args[i].as_str() {
                "-W" => {
                    i += 1;
                    if i < args.len() {
                        wordlist = Some(args[i].clone());
                    }
                }
                "-G" => {
                    i += 1;
                    if i < args.len() {
                        globpat = Some(args[i].clone());
                    }
                }
                "-a" => actions.push("alias"),
                "-b" => actions.push("builtin"),
                "-c" => actions.push("command"),
                "-d" => actions.push("directory"),
                "-e" => actions.push("export"),
                "-f" => actions.push("file"),
                "-j" => actions.push("job"),
                "-k" => actions.push("keyword"),
                "-u" => actions.push("user"),
                "-v" => actions.push("variable"),
                s if !s.starts_with('-') => prefix = s.to_string(),
                s => {
                    // bash compgen has many flags. Reject unknown
                    // ones rather than silently dropping. -F func and
                    // -C cmd aren't yet wired but they're real flags;
                    // accept as no-op pending impl.
                    if matches!(s, "-F" | "-C" | "-S" | "-P" | "-X" | "-o") {
                        // Take the following arg.
                        if i + 1 < args.len() {
                            i += 1;
                        }
                    } else if matches!(s, "-r" | "-A" | "-D" | "-E" | "-I") {
                        // Multi-letter or single-arg flags accepted as no-op.
                    } else {
                        eprintln!("zshrs:compgen:1: bad option: {}", s);
                        return 1;
                    }
                }
            }
            i += 1;
        }

        let mut results = Vec::new();

        // Generate based on actions
        for action in actions {
            match action {
                "alias" => {
                    let mut names: Vec<String> =
                        self.alias_entries().into_iter().map(|(k, _)| k).collect();
                    names.sort();
                    for name in names {
                        if name.starts_with(&prefix) {
                            results.push(name);
                        }
                    }
                }
                "builtin" => {
                    // Use the canonical BUILTIN_NAMES (derived from
                    // src/ported/builtin.rs:BUILTINS, which is the 1:1
                    // port of `Src/builtin.c:40-137 builtins[]`) so
                    // every wired builtin shows up in completion.
                    let mut names: Vec<&str> = BUILTIN_NAMES.iter().map(|s| s.as_str()).collect();
                    // zshrs extension builtins dispatch in-process too;
                    // include them so `compgen -b doc`/`compgen -b peach`
                    // resolve names the C-port BUILTINS table lacks.
                    names.extend(crate::ext_builtins::EXT_BUILTIN_NAMES.iter().copied());
                    names.sort();
                    names.dedup();
                    for name in names {
                        if name.starts_with(&prefix) {
                            results.push(name.to_string());
                        }
                    }
                }
                "directory" => {
                    // Sort dir entries for stable completion order.
                    if let Ok(entries) = std::fs::read_dir(".") {
                        let mut names: Vec<String> = entries
                            .flatten()
                            .filter(|e| e.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
                            .map(|e| e.file_name().to_string_lossy().into_owned())
                            .collect();
                        names.sort();
                        for name in names {
                            if name.starts_with(&prefix) {
                                results.push(name);
                            }
                        }
                    }
                }
                "file" => {
                    if let Ok(entries) = std::fs::read_dir(".") {
                        let mut names: Vec<String> = entries
                            .flatten()
                            .map(|e| e.file_name().to_string_lossy().into_owned())
                            .collect();
                        names.sort();
                        for name in names {
                            if name.starts_with(&prefix) {
                                results.push(name);
                            }
                        }
                    }
                }
                "variable" => {
                    // Sort for deterministic completion-candidate
                    // order (was HashMap iteration random, so
                    // \`compgen -v\` listings flickered).
                    let mut names: Vec<String> =
                        if let Ok(tab) = crate::ported::params::paramtab().read() {
                            tab.iter()
                                .filter(|(_, pm)| pm.u_arr.is_none())
                                .map(|(k, _)| k.clone())
                                .collect()
                        } else {
                            Vec::new()
                        };
                    names.sort();
                    for name in names {
                        if name.starts_with(&prefix) {
                            results.push(name);
                        }
                    }
                    let mut env_names: Vec<String> = std::env::vars().map(|(k, _)| k).collect();
                    env_names.sort();
                    for name in env_names {
                        if name.starts_with(&prefix) && !results.contains(&name) {
                            results.push(name);
                        }
                    }
                }
                _ => {}
            }
        }

        // Handle wordlist
        if let Some(words) = wordlist {
            for word in words.split_whitespace() {
                if word.starts_with(&prefix) {
                    results.push(word.to_string());
                }
            }
        }

        // Handle glob pattern
        if let Some(_pattern) = globpat {
            let full_pattern = format!("{}*", prefix);
            if let Ok(paths) = glob::glob(&full_pattern) {
                for path in paths.flatten() {
                    results.push(path.to_string_lossy().to_string());
                }
            }
        }

        results.sort();
        results.dedup();
        for r in results {
            println!("{}", r);
        }
        0
    }

    /// Define completion spec for a command
    pub(crate) fn builtin_complete(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            // List all completion specs, sorted by command name so
            // 'complete' (no args) outputs deterministically.
            let mut cmds: Vec<&String> = self.completions.keys().collect();
            cmds.sort();
            for cmd in cmds {
                let spec = self.completions.get(cmd).unwrap();
                let mut parts = vec!["complete".to_string()];
                for action in &spec.actions {
                    parts.push(format!("-{}", action));
                }
                if let Some(ref w) = spec.wordlist {
                    parts.push("-W".to_string());
                    parts.push(format!("'{}'", w));
                }
                if let Some(ref f) = spec.function {
                    parts.push("-F".to_string());
                    parts.push(f.clone());
                }
                if let Some(ref c) = spec.command {
                    parts.push("-C".to_string());
                    parts.push(c.clone());
                }
                parts.push(cmd.clone());
                println!("{}", parts.join(" "));
            }
            return 0;
        }

        let mut spec = CompSpec::default();
        let mut commands = Vec::new();
        let mut i = 0;

        while i < args.len() {
            match args[i].as_str() {
                "-W" => {
                    i += 1;
                    if i < args.len() {
                        spec.wordlist = Some(args[i].clone());
                    }
                }
                "-F" => {
                    i += 1;
                    if i < args.len() {
                        spec.function = Some(args[i].clone());
                    }
                }
                "-C" => {
                    i += 1;
                    if i < args.len() {
                        spec.command = Some(args[i].clone());
                    }
                }
                "-G" => {
                    i += 1;
                    if i < args.len() {
                        spec.globpat = Some(args[i].clone());
                    }
                }
                "-P" => {
                    i += 1;
                    if i < args.len() {
                        spec.prefix = Some(args[i].clone());
                    }
                }
                "-S" => {
                    i += 1;
                    if i < args.len() {
                        spec.suffix = Some(args[i].clone());
                    }
                }
                "-a" => spec.actions.push("a".to_string()),
                "-b" => spec.actions.push("b".to_string()),
                "-c" => spec.actions.push("c".to_string()),
                "-d" => spec.actions.push("d".to_string()),
                "-e" => spec.actions.push("e".to_string()),
                "-f" => spec.actions.push("f".to_string()),
                "-j" => spec.actions.push("j".to_string()),
                "-r" => {
                    // Remove completion spec
                    i += 1;
                    while i < args.len() {
                        self.completions.remove(&args[i]);
                        i += 1;
                    }
                    return 0;
                }
                s if !s.starts_with('-') => commands.push(s.to_string()),
                _ => {}
            }
            i += 1;
        }

        for cmd in commands {
            self.completions.insert(cmd, spec.clone());
        }
        0
    }

    /// compdef - register completion functions for commands
    /// Usage: compdef _git git
    ///        compdef _docker docker docker-compose
    ///        compdef -d git  # delete
    pub(crate) fn builtin_compdef(&mut self, args: &[String]) -> i32 {
        // PFA-SMR aspect: emit one `compdef` event with the completion
        // function name + the joined command list it's bound to.
        // `compdef _git git gita gitb` → name="_git", value="git gita gitb".
        #[cfg(feature = "recorder")]
        if crate::recorder::is_enabled() {
            let mut positional: Vec<&str> = Vec::new();
            for a in args {
                if a.starts_with('-') || a.starts_with('+') {
                    continue;
                }
                positional.push(a.as_str());
            }
            if positional.len() >= 2 {
                let ctx = self.recorder_ctx();
                let func = positional[0];
                let cmds = positional[1..].join(" ");
                crate::recorder::emit_compdef(func, &cmds, ctx);
            }
        }
        // The runtime `compdef` entry point lives in `compinit.rs`
        // and owns a process-wide `CompdefState` published back into
        // the shell-side `_comps` / `_services` / `_patcomps` /
        // `_postpatcomps` / `_compautos` arrays. The legacy
        // `compsys_cache` SQLite path is no longer the source of
        // truth.
        crate::compsys::ported::compinit::compdef(args)
    }

    /// compinit - initialize the completion system
    /// Scans fpath for completion functions and registers them
    #[tracing::instrument(level = "info", skip(self))]
    pub(crate) fn builtin_compinit(&mut self, args: &[String]) -> i32 {
        tracing::debug!(target: "compsys_args", ?args, "builtin_compinit ENTER");
        // compinit sh:523 — `for _i_dir in $fpath`, and sh:455's
        // `compaudit` likewise walks `$fpath`. Upstream reads the
        // PARAMETER at call time, so the `fpath=( … )` line every
        // .zshrc runs immediately before `compinit` is what gets
        // scanned.
        //
        // `self.fpath` is not that: it is seeded once at startup from
        // the inherited `$FPATH` env var (vm_helper.rs:1174/1287) and
        // never resynced, so an `fpath=( … )` assignment was invisible
        // here. Whenever the parent process exported FPATH the two
        // happened to agree and the bug stayed hidden; with FPATH unset
        // — which is the normal case for `zsh -f`, for a login shell
        // that builds fpath in .zshrc, and for the parity harness's
        // child env (scripts/comptab_parity.py `child_env`) — the scan
        // got ZERO directories. That is not a silent no-op: the worker
        // still completes and `set_comps_bulk` writes its empty result
        // over the cache, so `$_comps` came back empty, `_dispatch`
        // resolved every command to `-default-`, and completion was
        // dead shell-wide until something re-populated the cache.
        //
        // Resync from the live parameter, keeping the env-derived value
        // when `$fpath` is unset (module/plugin callers that never set
        // the array).
        self.fpath = crate::compsys::ported::shared::compinit_scan_dirs(&self.fpath);
        // Parse options
        // -C: use cache if valid (skip fpath scan)
        // -D: don't dump (don't write .zcompdump)
        // -d file: specify dump file
        // -u: use insecure dirs anyway  -i: silently ignore insecure dirs
        // -q: quiet
        let mut quiet = false;
        let mut no_dump = false;
        let mut dump_file: Option<String> = None;
        let mut use_cache = false;
        let mut ignore_insecure = false;
        let mut use_insecure = false;

        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "-q" => quiet = true,
                "-C" => use_cache = true,
                "-D" => no_dump = true,
                "-d" => {
                    i += 1;
                    if i < args.len() {
                        dump_file = Some(args[i].clone());
                    }
                }
                "-u" => use_insecure = true,
                "-i" => ignore_insecure = true,
                // -f: force re-dump even when dumpfile is current.
                // -w: warn about old / suspicious files (man compinit).
                // Both are real zsh flags; previously rejected by the
                // unknown-flag arm because they weren't enumerated.
                "-f" | "-w" => {} // accepted; semantic wiring is no-op
                s if s.starts_with('-') && s.len() > 1 => {
                    // compinit -X errors in zsh ("bad option") rather
                    // than silently no-op'ing. Without this, typos
                    // like \`compinit -B\` (bash convention) would
                    // proceed normally and only fail later via the
                    // missing flag's effect.
                    let bad: String = s[1..].chars().take(1).collect();
                    eprintln!("zshrs:compinit:1: bad option: -{}", bad);
                    return 1;
                }
                _ => {}
            }
            i += 1;
        }

        // compinit sh:116-197 — the `typeset -g…` block at the top of
        // compinit's body. Unconditional upstream, so it must run
        // before every branch below (including the `-C` cache-hit
        // early return and the insecure-directory bail-out), not just
        // on the fresh-scan path in `compinit::compinit()`.
        crate::compsys::ported::compinit::declare_compinit_globals(dump_file.as_deref());

        // compinit sh:201 — `: $funcstack` ("Loading it now ensures that
        // the `funcstack' parameter is always correct"). Reaching for a
        // `zsh/parameter` parameter loads that module, so a real zsh lists
        // it in `zmodload -L` after any compinit.
        crate::compsys::ported::compinit::touch_funcstack_param();

        // compinit sh:455 `autoload -RUz compaudit` and sh:481
        // `autoload -RUz compdump compinstall` ("Make sure compdump is
        // available, even if we aren't going to use it"), plus sh:578
        // `autoload -RUz compinit compaudit` at the very end. These are
        // unconditional lines in compinit's own body — they fire on
        // every path, dump-hit or fresh-scan alike, so a real zsh always
        // finishes compinit with all three names in `${(k)functions}`.
        // zshrs derived its autoload stubs purely from the fpath/cache
        // scan, and none of the three carries a `#compdef` / `#autoload`
        // header, so they were the only names compinit itself guarantees
        // that zshrs was missing. Registered before the audit call so
        // the insecure-directory early return below still leaves
        // `compaudit` callable, exactly as sh:455 does.
        crate::compsys::ported::compinit::register_autoload_stubs([
            "compaudit",   // sh:455
            "compdump",    // sh:481
            "compinstall", // sh:481
        ]);

        // compinit sh:515-518 — `-C` clears `_i_check` (sh:102-104), and
        // with the check off an existing dump file is sourced outright:
        //
        //   else
        //     builtin . "$_comp_dumpfile"
        //     _i_done=yes
        //   fi
        //
        // `_i_done` then skips the entire sh:523-550 `$fpath` scan, so on
        // that path the dump — not the scan — is what defines every
        // completer name the session ends up with. zshrs substitutes its
        // own cache for the dump's `_comps`/`_services`/… payload, but the
        // dump's `autoload` lines have no substitute: compdump lists every
        // defined `_*` function that has a file in `$fpath`
        // (compdump:113), which includes headerless helpers no
        // `#compdef`/`#autoload` scan can see. On this host that was 12
        // names — `_command_names`, `_parameters`, `_megacomplete`,
        // `_complete_hist`, `__zpwr_aliases`, … — every one of them a
        // `# -*- mode: sh -*-`-headed file, plus `_zemacs`, whose file has
        // since left `$fpath` entirely. Missing stubs are observable well
        // beyond `${(k)functions}`: the legacy `compcall` namespace census
        // `_default` falls back to counts them, and `_tmux` derives its
        // sub-command list from `${(M)${(k)functions}:#_tmux-*}`.
        //
        // Only the `-C` branch is mirrored. The checked branch (sh:492-514)
        // loads the dump solely when its recorded file count and
        // `$ZSH_VERSION` both still match; zshrs does not evaluate that
        // condition and rescans instead — the same thing a real zsh does
        // whenever the dump is stale.
        // sh:497-498/516-517 — sourcing the dump sets `_i_done=yes`, and
        // sh:523 (`if [[ -z "$_i_done" ]]`) then skips the whole $fpath scan.
        // So on the dump path the dump is the SOLE source of names; anything
        // else that contributes is a zshrs-only divergence.
        let mut dump_sourced = false;
        let mut dump_tables = None;
        if use_cache {
            if let Some(dump) = crate::ported::params::getsparam("_comp_dumpfile") {
                let dump = std::path::PathBuf::from(dump);
                // `-C` skips zsh's security check, and sh:492-514's checked
                // branch is the only place upstream compares the dump against
                // reality. zshrs materialises its bundled function tree into
                // ~/.zshrs/functions out of band, so a dump written before an
                // upgrade lists none of it and `-C` would trust it forever:
                // every zshrs builtin then completed as if nothing shipped
                // (`_comps[zjob]` empty with _zjob sitting on fpath). Treat
                // the dump as stale when the bundle stamp is newer, which is
                // what a real zsh does whenever its own staleness check trips.
                let bundle_newer = (|| {
                    let stamp = crate::bundled_functions::functions_dir()?
                        .join(".zshrs-bundle-version");
                    let s = std::fs::metadata(&stamp).ok()?.modified().ok()?;
                    let d = std::fs::metadata(&dump).ok()?.modified().ok()?;
                    Some(s > d)
                })()
                .unwrap_or(false);
                if dump.is_file() {
                    let names = crate::compsys::ported::compinit::dump_autoload_names(&dump);
                    let added = crate::compsys::ported::compinit::register_autoload_stubs(&names);
                    dump_sourced = true;
                    tracing::info!(
                        added,
                        total = names.len(),
                        dump = %dump.display(),
                        "compinit: autoload stubs from dump"
                    );
                    // The other half of sh:494: the five association tables.
                    // Same `_i_done` argument as the autoload names above —
                    // sh:501 skips the $fpath scan, so the dump alone defines
                    // `$_comps` and friends. Reading them from zshrs's SQLite
                    // cache instead let a partially-built cache silently
                    // replace the dump (1849 `_comps` keys vs the dump's
                    // 51745 on this host), which drops `$_comps[zpwr]`,
                    // `$_comps[cargo]`, … and routes those commands to
                    // `-default-` file completion.
                    dump_tables = crate::compsys::ported::compinit::dump_assoc_tables(&dump);
                    // A dump older than the bundled tree cannot list zshrs's
                    // own completers (`_zjob`, `_ztag`, `_zcache`, …), but
                    // DISCARDING the whole dump for that — which is what the
                    // old `!bundle_newer` gate did — replaced it with the
                    // SQLite cache, and the cache is a DIFFERENT table, not a
                    // superset.  Measured on this host, fpath pinned to
                    // `zsh -f`'s, only the dump's mtime varied:
                    //   cache path: n=51577  X=_X  7z=_7z    zjob=_zjob
                    //   dump path : n=51708  X=_X  7z=_7zip  zjob=
                    //   real zsh  : n=51708  X=_X  7z=_7zip  zjob=
                    // i.e. the dump path is byte-identical to zsh, while the
                    // cache path is missing 163 of zsh's keys, carries 32 of
                    // its own, and resolves 555 MORE keys to a DIFFERENT
                    // completer (`7z` → `_7z` instead of the distribution's
                    // `_7zip`) — a difference the net 131-key delta hides.
                    // Keep sh:494's dump authoritative and add only what it
                    // cannot know about, with sh:519's `compdef -na`
                    // first-claim-wins so the dump wins every key it carries.
                    if bundle_newer {
                        if let Some(t) = dump_tables.as_mut() {
                            let added =
                                crate::compsys::ported::compinit::merge_bundled_registrations(t);
                            tracing::info!(
                                added,
                                dump = %dump.display(),
                                "compinit: dump predates the bundled tree — overlaid bundled registrations"
                            );
                        }
                    }
                }
            }
        }

        // sh:434 `if [[ -n "$_i_check" ]]` — `-C` clears `_i_check` (sh:84
        // `(( $+_i_opth[-C] )) && _i_check=`), so the cached path never
        // audits.  `-u` sets `_i_fail=use` (sh:88), which makes compaudit
        // itself return 0 before flagging anything.
        if !use_cache && !use_insecure && !self.posix_mode {
            // sh:436 `if ! eval compaudit`.  This MUST be the faithful port,
            // not `plugin_cache::compaudit_cached`: that helper's
            // `check_dir_security` short-circuits `if uid == 0 || uid == euid
            // { return true }` (plugin_cache.rs:812-814), calling any
            // caller-owned directory secure whatever its mode, while
            // compaudit sh:125's qualifier list
            // `(N-f:g+w:,-f:o+w:,-^${_i_owners})` treats the commas as
            // ALTERNATIVES — group-writable OR other-writable OR untrusted
            // owner each flag it.  A user-owned 0777 fpath directory is
            // therefore insecure to zsh and "secure" to the helper, which
            // made every branch below dead code for the ordinary case:
            // zshrs registered and RAN a completer out of a world-writable
            // directory where zsh aborts initialization outright.
            //   zsh  : compinit: initialization aborted / rc=1 / _comps[zz01] empty
            //   zshrs: rc=0 / _comps[zz01]=_zz01
            if let Err(audit) = crate::compsys::ported::compaudit::compaudit(&self.fpath) {
                // sh:437 `if [[ -n "$_i_q" ]]` — compaudit only sets its
                // description when it actually flagged something.
                if !audit.is_empty() {
                    if !ignore_insecure {
                        // sh:438-451 — upstream's default `ask` arm prompts
                        // with `read -q` and takes this branch when the answer
                        // is "no" (or when there is no terminal: a
                        // non-interactive zsh prints "initialization aborted"
                        // and returns 1).  zshrs has no prompt here, so it
                        // always takes the abort arm.
                        if !quiet {
                            eprintln!("zshrs:compinit:1: insecure directories:");
                            for d in &audit.insecure_dirs {
                                eprintln!("  {}", d.display());
                            }
                            eprintln!(
                                "zshrs:compinit:1: run with -i to ignore or -u to use anyway"
                            );
                        }
                        return 1;
                    }
                    // sh:452 `fpath=(${fpath:|_i_wdirs})` — element
                    // subtraction, so a flagged PARENT directory that is not
                    // itself an `$fpath` entry removes nothing, exactly as
                    // upstream.  This also subsumes sh:506's
                    // `(( $_i_wdirs[(I)$_i_dir] )) && continue`, because every
                    // scan below reads `self.fpath`.
                    self.fpath.retain(|d| !audit.insecure_dirs.contains(d));
                    let fp: Vec<String> = self
                        .fpath
                        .iter()
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect();
                    crate::ported::params::setaparam("fpath", fp);
                }
                // sh:456 `typeset -g _comp_secure=yes`
                let _ = crate::ported::params::setsparam("_comp_secure", "yes");
            }
        }

        // Install the `compdef` function stub SYNCHRONOUSLY (before the
        // native scan is shipped to the worker pool, which returns
        // immediately). zsh's `compinit` defines `compdef` as a shell
        // function; zshrs does the scan in Rust and never installed it,
        // so `${+functions[compdef]}` stayed 0 and zinit's compdef-replay
        // aborted with "compinit function hasn't been loaded" while direct
        // `compdef` calls hit command-not-found. Only install our stub when
        // no real compdef function already exists (don't clobber a user or
        // --zsh fpath definition); the marker body routes to the fast
        // native impl in the BUILTIN_COMPDEF handler.
        if !self.function_exists("compdef") {
            crate::ported::modules::parameter::setfunction(
                "compdef",
                NATIVE_COMPDEF_MARKER.to_string(),
                0,
            );
        }

        // ZSH COMPAT MODE: Use traditional zsh algorithm (fpath scan, .zcompdump, no SQLite)
        if self.zsh_compat {
            return self.compinit_compat(quiet, no_dump, dump_file, use_cache);
        }

        // ZSHRS MODE: Use SQLite cache with function bodies

        // Main-thread widget/keybinding setup runs on EVERY compinit path,
        // BEFORE any early return. The `-C` cached branch below returned
        // without ever rebinding the completion widgets, so a cache-hit
        // compinit (zpwr's default) left TAB on the INTERNAL completer —
        // `ls -<TAB>` completed FILES instead of options while the fresh-
        // scan path worked. These installs need no scan results (sh:542
        // binds only _main_complete; the -k table is static).
        crate::compsys::ported::compinit::install_standard_complete_widgets();
        crate::compsys::ported::compinit::maybe_rebind_tab_for_expand();
        crate::compsys::ported::compinit::install_standard_comp_keybindings();

        // sh:493-496 + sh:501 — `-C` with an existing dump: `builtin .
        // "$_comp_dumpfile"` then `_i_done=yes`, and `if [[ -z "$_i_done" ]]`
        // skips the entire sh:504-528 `$fpath` scan. The dump is the ONLY
        // thing that defines the five association tables on this path, so it
        // takes precedence over zshrs's SQLite cache — which tracks a
        // different refresh schedule and can hold a partial scan.
        //
        // This branch precedes the cache branch deliberately: an
        // out-of-date-but-`cache_is_valid` cache must not shadow the dump,
        // and a cache that fails `cache_is_valid` must not fall through to
        // the worker-pool rescan sh:501 says never happens.
        if let Some(tables) = dump_tables {
            tracing::info!(
                comps = tables.comps.len(),
                services = tables.services.len(),
                patcomps = tables.patcomps.len(),
                postpatcomps = tables.postpatcomps.len(),
                compautos = tables.compautos.len(),
                "compinit: association tables from dump"
            );
            self.set_assoc("_comps".to_string(), tables.comps);
            self.set_assoc("_services".to_string(), tables.services);
            self.set_assoc("_patcomps".to_string(), tables.patcomps);
            self.set_assoc("_postpatcomps".to_string(), tables.postpatcomps);
            self.set_assoc("_compautos".to_string(), tables.compautos);
            return 0;
        }

        // Try to use existing cache if -C and cache is valid
        if use_cache {
            if let Some(cache) = self.compsys_cache() {
                if crate::compsys::cache_is_valid(cache, &self.fpath) {
                    // Load from cache instead of rescanning
                    if let Ok(result) = crate::compsys::load_from_cache(cache) {
                        if !quiet {
                            tracing::info!(
                                comps = result.comps.len(),
                                "compinit: using cached completions"
                            );
                        }
                        // compinit sh:337/sh:541 — every registered completer is
                        // `autoload -rUz`'d, so `${(k)functions}` lists all of
                        // them. Completers depend on that: `_tmux` derives its
                        // sub-command list from `${(M)${(k)functions}:#_tmux-*}`
                        // (_tmux sh:1967), which silently lost the `_tmux-*`
                        // helpers in $fpath while this table stayed empty.
                        //
                        // …but ONLY when no dump was sourced above. sh:523's
                        // `[[ -z "$_i_done" ]]` makes the dump authoritative on
                        // the `-C` path: a real zsh ends up with exactly the
                        // names the dump lists, no more. zshrs's SQLite cache is
                        // refreshed independently of the shared `.zcompdump`, so
                        // running both sources added every completer the cache
                        // had seen since the dump was last written (`_john` from
                        // site-functions, on this host) — functions reference zsh
                        // does not define. Keep the cache as the name source only
                        // for the no-dump case, where sh:523-550 would have
                        // scanned $fpath.
                        if !dump_sourced {
                            let stub_t0 = std::time::Instant::now();
                            if let Ok(names) = cache.list_autoload_names() {
                                let added =
                                    crate::compsys::ported::compinit::register_autoload_stubs(
                                        &names,
                                    );
                                tracing::info!(
                                    added,
                                    total = names.len(),
                                    ms = stub_t0.elapsed().as_millis() as u64,
                                    "compinit: autoload stubs"
                                );
                            }
                        }

                        self.set_assoc("_comps".to_string(), result.comps.into_iter().collect());
                        self.set_assoc(
                            "_services".to_string(),
                            result.services.into_iter().collect(),
                        );
                        self.set_assoc(
                            "_patcomps".to_string(),
                            result.patcomps.into_iter().collect(),
                        );
                        // sh:116/121 — `_postpatcomps` and `_compautos`
                        // are populated from the same dump/scan as the
                        // three above (compdump sh:112-131 writes all
                        // five). Publishing only three left
                        // `${(k)_postpatcomps}` empty on the cache-hit
                        // path even though the cache held the entries.
                        self.set_assoc(
                            "_postpatcomps".to_string(),
                            result.postpatcomps.into_iter().collect(),
                        );
                        self.set_assoc(
                            "_compautos".to_string(),
                            result.compautos.into_iter().collect(),
                        );

                        return 0;
                    }
                }
            }
        }

        // Ship compinit to worker pool — no ad-hoc thread spawn.
        // The heavy work (scan + SQLite write) runs on a pool thread.
        // Results are merged into shell state lazily via drain_compinit_bg().
        let fpath = self.fpath.clone();
        let fpath_count = fpath.len();
        let pool_size = self.worker_pool.size();
        let (tx, rx) = std::sync::mpsc::channel();
        let bg_start = std::time::Instant::now();
        tracing::info!(
            fpath_dirs = fpath_count,
            worker_pool = pool_size,
            "compinit: shipping to worker pool"
        );
        // The cache-identity stamp must record the `$fpath` this build is
        // scanning, and the closure below is `move` — capture it by value
        // rather than borrowing `self` into a 'static thread.
        let stamp_fpath: Vec<std::path::PathBuf> = self.fpath.clone();
        self.worker_pool.submit(move || {
            tracing::debug!("compinit-bg: thread started");
            let cache_path = crate::compsys::cache::default_cache_path();
            if let Some(parent) = cache_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            // Serialise the whole rebuild. The build-aside + `rename`
            // below is atomic for a READER, but two BUILDERS still each
            // install a complete database over the other's: whichever
            // renames last wins and the loser's entire scan is thrown
            // away, along with any registration only it had seen. Up to
            // 16 of the user's shells share this file, so two of them
            // starting cold at once is the ordinary case. Blocking and
            // exclusive, on a side file the rename does not replace, the
            // same shape `script_cache` and `autoload_cache` already use
            // for their shards.
            let _rebuild_lock = crate::compsys::cache::acquire_rebuild_lock();
            // Whoever held the lock before us may have just installed a
            // cache this binary can use. Re-check under the lock instead
            // of rebuilding what is already there: the check that sent us
            // down this branch ran before we waited.
            //
            // ONLY for `compinit -C`. Without `-C`, sh:504-528 scans
            // `$fpath` unconditionally, and this one file is shared by
            // every shell on the machine no matter what each one's
            // `$fpath` holds — so accepting a neighbour's cache here
            // publishes the NEIGHBOUR's registrations. Measured: eight
            // shells rebuilding at once, each with one completion unique
            // to its own `$fpath`, all eight came back holding shell 8's
            // unique entry and none of their own.
            if use_cache {
            if let Ok(existing) = crate::compsys::cache::CompsysCache::open(&cache_path) {
                if crate::compsys::cache_is_valid(&existing, &stamp_fpath) {
                    if let Ok(result) = crate::compsys::load_from_cache(&existing) {
                        tracing::info!(
                            path = %cache_path.display(),
                            comps = result.comps.len(),
                            "compinit: another shell installed a usable cache while we waited for the rebuild lock"
                        );
                        let _ = tx.send(CompInitBgResult {
                            result,
                            cache: existing,
                        });
                        return;
                    }
                }
            }
            }

            // Build into a private file and move it into place when it is
            // finished, the same crash-safe shape `compdump` already uses
            // for the dump (compdump.rs:175-188, sh:21-23).
            //
            // The old code deleted the live db (plus its -shm/-wal) FIRST
            // and then spent seconds refilling it. Up to 16 of the user's
            // shells share this one file, so every shell that started
            // inside that window found either no cache at all or one with
            // a handful of rows — and `cache_is_valid`'s old `count > 0`
            // accepted the partial — leaving `_comps` with a fraction of
            // its registrations and `_dispatch` with no completer for any
            // command. `rename(2)` is atomic within a filesystem, so a
            // concurrent reader now opens either the whole previous cache
            // or the whole new one, never a half-written file.
            let build_path = cache_path.with_file_name(format!(
                "{}.building.{}",
                cache_path
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "compsys.db".to_string()),
                std::process::id()
            ));
            // Leftovers from a build this pid was killed during. The path
            // is pid-private, so nothing else can be reading them.
            for stale in [
                build_path.display().to_string(),
                format!("{}-shm", build_path.display()),
                format!("{}-wal", build_path.display()),
            ] {
                let _ = std::fs::remove_file(stale);
            }

            let mut cache = match crate::compsys::cache::CompsysCache::open(&build_path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("compinit: failed to create cache: {}", e);
                    return;
                }
            };

            let result = match crate::compsys::build_cache_from_fpath(&fpath, &mut cache) {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!("compinit: scan failed: {}", e);
                    return;
                }
            };

            tracing::info!(
                functions = result.files_scanned,
                comps = result.comps.len(),
                dirs = result.dirs_scanned,
                ms = result.scan_time_ms,
                "compinit: background scan complete"
            );

            // No bytecode pre-warm here. Autoload chunks are cached
            // write-through by the loader itself (`vm_helper`'s autoload
            // arm → `autoload_cache::try_save_one`), keyed on the resolved
            // fpath directory plus a digest of the exact definition text.
            // A speculative pre-warm on
            // this worker thread cannot produce those chunks: it would have
            // to parse 46k bodies against process-global lexer state that
            // the interactive main thread is using concurrently — which is
            // what corrupted the prompt into a stuck PS2 when the pre-warm
            // was enabled — and the chunks it built were the bare file body
            // compiled as a top-level script, not the definition program the
            // loader installs.
            // Stamp completeness LAST, so `cache_is_valid` can tell a
            // finished cache from one that is still filling, then close the
            // connection: SQLite checkpoints the WAL into the db and removes
            // the -wal/-shm side files on the last close, which is what makes
            // the single-file rename below carry the whole cache.
            if !crate::compsys::ported::compinit::stamp_cache_complete(&cache, &stamp_fpath) {
                tracing::error!("compinit: could not stamp cache as complete; not installing it");
                return;
            }
            drop(cache);

            // The outgoing file's side files must go before the rename: they
            // are named after `cache_path`, so leaving them would pair a WAL
            // from the OLD database with the NEW one. The freshly built cache
            // has none of its own after the clean close above.
            for stale in [
                format!("{}-shm", cache_path.display()),
                format!("{}-wal", cache_path.display()),
            ] {
                let _ = std::fs::remove_file(stale);
            }
            if let Err(e) = std::fs::rename(&build_path, &cache_path) {
                tracing::error!(error = %e, "compinit: could not install rebuilt cache");
                let _ = std::fs::remove_file(&build_path);
                return;
            }

            let cache = match crate::compsys::cache::CompsysCache::open(&cache_path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("compinit: failed to reopen installed cache: {}", e);
                    return;
                }
            };

            let _ = tx.send(CompInitBgResult { result, cache });
        });

        // (Widget + #compdef -k installs run for ALL paths near the top of
        // this fn, before the cached-branch early return.)

        // zsh CONTRACT: compinit RETURNS with $_comps populated — the scan
        // work is parallel (rayon fan-out on the pool inside compinit()),
        // the COMPLETION of that work is not deferred. The previous lazy
        // merge (compinit_pending + drain) left $_comps empty until some
        // later drain call — `_dispatch` resolved comp="" for every command
        // and option completion was dead shell-wide. Block on the result
        // here; warm starts take the cache path above and never reach this.
        match rx.recv() {
            Ok(bg) => {
                let comps = bg.result.comps.len();
                crate::compsys::ported::compinit::apply_keybindings(&bg.result);
                // sh:333/sh:541 — every completer compinit registers is also
                // `autoload -rUz`'d, from BOTH scan arms: `compdef -na` does it
                // for each `#compdef` file and the `#autoload` arm does it
                // directly. This branch is the one a cold `compinit` (no `-C`)
                // takes, and it published the five association tables without
                // ever registering a stub, so the shell came back with
                // `$#_comps` = 1822 and `${#${(M)${(k)functions}:#_*}}` = 0
                // where the same scan under reference zsh ends at 9048.
                // `_main_complete` was among the missing names, so the
                // completion widgets compinit binds at sh:556-560 all called an
                // undefined function and tab inserted nothing.
                let mut stubs = crate::compsys::ported::compinit::register_autoload_stubs(
                    crate::compsys::ported::compinit::autoload_stub_names(&bg.result),
                );
                // `autoload_stub_names` reads `result.files`, which only a
                // fresh `$fpath` scan fills — the background thread also
                // returns a `load_from_cache` result (compinit.rs) when
                // another shell installs a usable cache while this one waits
                // for the rebuild lock, and that leaves `files` empty. The
                // `autoloads` table holds the names that scan produced.
                if bg.result.files.is_empty() {
                    if let Ok(names) = bg.cache.list_autoload_names() {
                        stubs +=
                            crate::compsys::ported::compinit::register_autoload_stubs(&names);
                    }
                }
                tracing::info!(stubs, "compinit: autoload stubs from scan");
                self.set_assoc("_comps".to_string(), bg.result.comps.into_iter().collect());
                self.set_assoc(
                    "_services".to_string(),
                    bg.result.services.into_iter().collect(),
                );
                self.set_assoc(
                    "_patcomps".to_string(),
                    bg.result.patcomps.into_iter().collect(),
                );
                // sh:116/121 — `typeset -gHA` declares all five, and the
                // scan fills all five (compdump.rs writes all five). The
                // cache-hit branch above already published the other two;
                // this branch stopped at three, so a fresh-scan shell had
                // `_postpatcomps` unset and `_dispatch`'s post pass nothing
                // to walk — every `#compdef -P` completer (`_dir_list`,
                // `_urls`, `_locales`, `_gcc`, `_python`, …) dead.
                self.set_assoc(
                    "_postpatcomps".to_string(),
                    bg.result.postpatcomps.into_iter().collect(),
                );
                self.set_assoc(
                    "_compautos".to_string(),
                    bg.result.compautos.into_iter().collect(),
                );
                self.compsys_cache = std::cell::OnceCell::from(Some(bg.cache));
                tracing::info!(
                    wall_ms = bg_start.elapsed().as_millis() as u64,
                    comps,
                    "compinit: scan complete, _comps populated"
                );
                0
            }
            Err(_) => {
                tracing::error!("compinit: scan worker died without sending results");
                1
            }
        }
    }

    /// cdreplay - replay deferred compdef calls (zinit turbo mode)
    /// Usage: cdreplay [-q]
    pub(crate) fn builtin_cdreplay(&mut self, args: &[String]) -> i32 {
        let quiet = args.contains(&"-q".to_string());

        if self.deferred_compdefs.is_empty() {
            return 0;
        }

        let deferred = std::mem::take(&mut self.deferred_compdefs);
        let count = deferred.len();

        // One publish for the whole replay. Each `compdef` otherwise
        // read-modify-writes the entire `_comps` hash (there is no
        // single-key assoc setter in `params.rs`), which a turbo-mode
        // replay of hundreds of registrations would pay hundreds of times
        // over ~50k entries. The resulting `_comps` is identical.
        crate::compsys::ported::compinit::compdef_batch(|| {
            for compdef_args in deferred {
                crate::compsys::ported::compinit::compdef(&compdef_args);
            }
        });

        if !quiet {
            eprintln!("cdreplay: replayed {} compdef calls", count);
        }

        0
    }

    // zgetattr/zsetattr/zdelattr/zlistattr - extended attributes
    // builtin_zattr deleted — zero callers. Was a Rust-only dispatcher
    // wrapping the 4 zattr bin_* free ported; callers should use
    // canonical dispatch_builtin("zgetattr"/etc, args) which goes
    // through execbuiltin → BUILTINS entry (attr.c:NNN ports) with
    // optstr parsing built in.
}

/// promptinit autoload — seeds `$prompt_themes` array + default
/// `$prompt_theme` scalar. Free function (not on ShellExecutor)
/// per "no Rust state mirrors" rule; writes paramtab directly.
pub(crate) fn promptinit(_args: &[String]) -> i32 {
    crate::ported::params::setaparam(
        "prompt_themes",
        vec![
            "adam1".to_string(),
            "adam2".to_string(),
            "bart".to_string(),
            "bigfade".to_string(),
            "clint".to_string(),
            "default".to_string(),
            "elite".to_string(),
            "elite2".to_string(),
            "fade".to_string(),
            "fire".to_string(),
            "minimal".to_string(),
            "off".to_string(),
            "oliver".to_string(),
            "pws".to_string(),
            "redhat".to_string(),
            "restore".to_string(),
            "suse".to_string(),
            "walters".to_string(),
            "zefram".to_string(),
        ],
    );
    crate::ported::params::setsparam("prompt_theme", "default");
    0
}

/// prompt autoload — switches to a prompt theme. Free function
/// per "no Rust state mirrors". Reads/writes paramtab directly;
/// reaches for ShellExecutor only via with_executor for the
/// theme-application step (which mutates exec.options /
/// PS1/RPS1 via set_scalar).
pub(crate) fn prompt(args: &[String]) -> i32 {
    if args.is_empty() {
        let theme = crate::ported::params::getsparam("prompt_theme")
            .unwrap_or_else(|| "default".to_string());
        println!("Current prompt theme: {}", theme);
        return 0;
    }
    let apply = |theme: &str, preview: bool| {
        crate::fusevm_bridge::with_executor(|exec| {
            exec.apply_prompt_theme(theme, preview);
        });
    };
    match args[0].as_str() {
        "-l" | "--list" => {
            // promptinit:119 `l) print Currently available prompt themes:` —
            // the exact wording, matching promptinit:75's own `-l` help text.
            // A paraphrase ("Available prompt themes:") breaks any script or
            // completion that greps this listing.
            println!("Currently available prompt themes:");
            if let Ok(tab) = crate::ported::params::paramtab().read() {
                if let Some(pm) = tab.get("prompt_themes") {
                    if let Some(themes) = &pm.u_arr {
                        for t in themes {
                            println!("  {}", t);
                        }
                    }
                }
            }
        }
        "-p" | "--preview" => {
            apply(args.get(1).map(|s| s.as_str()).unwrap_or("default"), true);
        }
        "-h" | "--help" => {
            println!("prompt [options] [theme]");
            println!("  -l, --list     List available themes");
            println!("  -p, --preview  Preview a theme");
            println!("  -s, --setup    Set up a theme");
        }
        _ => {
            let theme = if args[0].starts_with('-') {
                args.get(1).map(|s| s.as_str()).unwrap_or("default")
            } else {
                args[0].as_str()
            };
            apply(theme, false);
        }
    }
    0
}

impl ShellExecutor {
    pub(crate) fn builtin_cat(&self, args: &[String]) -> i32 {
        // coreutils cat(1) port: adds -E (show $ at line end),
        // -T (show TAB as ^I), -A (= -vET), -b (number nonempty),
        // -s (squeeze blank lines), -v (show non-printing as ^X).

        let mut number_all = false;
        let mut number_nonempty = false;
        let mut show_ends = false;
        let mut show_tabs = false;
        let mut show_nonprint = false;
        let mut squeeze_blank = false;
        let mut files: Vec<&str> = Vec::new();

        for arg in args {
            match arg.as_str() {
                "-n" => number_all = true,
                "-b" => number_nonempty = true,
                "-E" => show_ends = true,
                "-T" => show_tabs = true,
                "-v" => show_nonprint = true,
                "-A" => {
                    show_ends = true;
                    show_tabs = true;
                    show_nonprint = true;
                }
                "-e" => {
                    show_ends = true;
                    show_nonprint = true;
                }
                "-t" => {
                    show_tabs = true;
                    show_nonprint = true;
                }
                "-s" => squeeze_blank = true,
                "-" => files.push("-"),
                a if a.starts_with('-') && a.len() > 1 => {
                    for c in a[1..].chars() {
                        match c {
                            'n' => number_all = true,
                            'b' => number_nonempty = true,
                            'E' => show_ends = true,
                            'T' => show_tabs = true,
                            'v' => show_nonprint = true,
                            's' => squeeze_blank = true,
                            'A' => {
                                show_ends = true;
                                show_tabs = true;
                                show_nonprint = true;
                            }
                            'e' => {
                                show_ends = true;
                                show_nonprint = true;
                            }
                            't' => {
                                show_tabs = true;
                                show_nonprint = true;
                            }
                            // coreutils cat errors on unknown short
                            // flag letters (esp. inside combined forms
                            // like \`-nX\`). Old \`_ => {}\` swallowed.
                            _ => {
                                eprintln!("cat: unrecognized option: '-{}'", c);
                                return 1;
                            }
                        }
                    }
                }
                _ => files.push(arg),
            }
        }

        if files.is_empty() {
            files.push("-");
        }

        // Decorate one chunk per cat semantics. Returns the
        // transformed string with -T / -v / -E applied.
        let decorate = |s: &str| -> String {
            if !show_tabs && !show_nonprint && !show_ends {
                return s.to_string();
            }
            let mut out = String::with_capacity(s.len());
            for c in s.chars() {
                if c == '\t' {
                    if show_tabs {
                        out.push_str("^I");
                    } else {
                        out.push('\t');
                    }
                    continue;
                }
                if c == '\n' {
                    out.push(c);
                    continue;
                }
                if show_nonprint && (c.is_control() || (c as u32) >= 0x80) {
                    let code = c as u32;
                    if code < 0x20 {
                        out.push('^');
                        out.push((b'@' + code as u8) as char);
                    } else if code == 0x7f {
                        out.push_str("^?");
                    } else if code < 0x80 {
                        out.push(c);
                    } else {
                        // M- prefix for high-bit chars.
                        out.push_str("M-");
                        let lo = code & 0x7f;
                        if lo < 0x20 {
                            out.push('^');
                            out.push((b'@' + lo as u8) as char);
                        } else {
                            out.push(char::from_u32(lo).unwrap_or('?'));
                        }
                    }
                } else {
                    out.push(c);
                }
            }
            out
        };

        let mut stdout = io::stdout().lock();
        let mut line_num = 1usize;
        let mut prev_blank = false;
        let any_decoration = number_all
            || number_nonempty
            || show_ends
            || show_tabs
            || show_nonprint
            || squeeze_blank;

        for file in files {
            let result: io::Result<()> = (|| {
                if !any_decoration {
                    // Fast path: copy bytes through.
                    if file == "-" {
                        let stdin = io::stdin();
                        let mut handle = stdin.lock();
                        io::copy(&mut handle, &mut stdout)?;
                    } else {
                        let mut f = std::fs::File::open(file).inspect_err(|e| {
                            eprintln!(
                                "cat: {}: {}",
                                file,
                                crate::ported::compat::strerror(e.raw_os_error().unwrap_or(0))
                            );
                        })?;
                        io::copy(&mut f, &mut stdout)?;
                    }
                    return Ok(());
                }

                let reader: Box<dyn BufRead> = if file == "-" {
                    Box::new(BufReader::new(io::stdin()))
                } else {
                    let f = std::fs::File::open(file).inspect_err(|e| {
                        eprintln!(
                            "cat: {}: {}",
                            file,
                            crate::ported::compat::strerror(e.raw_os_error().unwrap_or(0))
                        );
                    })?;
                    Box::new(BufReader::new(f))
                };

                for line in reader.lines() {
                    let line = match line {
                        Ok(l) => l,
                        Err(_) => break,
                    };
                    let is_blank = line.is_empty();
                    if squeeze_blank && is_blank && prev_blank {
                        continue;
                    }
                    prev_blank = is_blank;

                    let decorated = decorate(&line);
                    let suffix = if show_ends { "$" } else { "" };
                    if number_all || (number_nonempty && !is_blank) {
                        writeln!(stdout, "{:6}\t{}{}", line_num, decorated, suffix)?;
                        line_num += 1;
                    } else {
                        // -b skips blank-line numbering; both that and the
                        // unnumbered branch print the decorated text only.
                        writeln!(stdout, "{}{}", decorated, suffix)?;
                    }
                }
                Ok(())
            })();

            if result.is_err() {
                return 1;
            }
        }
        0
    }

    pub(crate) fn builtin_head(&self, args: &[String]) -> i32 {
        // -n N: keep first N lines. -n -N: keep all BUT the last N
        // lines (coreutils extension). Negative is encoded by a
        // 'skip_last' tail count.
        let mut lines = 10usize;
        let mut skip_last_lines: Option<usize> = None;
        // Some(N) when -c N was given — switches to byte-count mode.
        let mut bytes: Option<usize> = None;
        let mut skip_last_bytes: Option<usize> = None;
        // -q / -v override the default 'header iff >1 file' rule.
        let mut force_quiet = false;
        let mut force_verbose = false;
        let mut files: Vec<&str> = Vec::new();
        let mut i = 0;

        // Parse a count that may be negative; returns (positive_count,
        // is_skip_last).
        let parse_count = |s: &str| -> (usize, bool) {
            if let Some(rest) = s.strip_prefix('-') {
                (rest.parse().unwrap_or(0), true)
            } else if let Some(rest) = s.strip_prefix('+') {
                (rest.parse().unwrap_or(0), false)
            } else {
                (s.parse().unwrap_or(0), false)
            }
        };

        while i < args.len() {
            let arg = &args[i];
            if arg == "-n" && i + 1 < args.len() {
                i += 1;
                let (n, neg) = parse_count(&args[i]);
                if neg {
                    skip_last_lines = Some(n);
                } else {
                    lines = n;
                }
            } else if let Some(after) = arg.strip_prefix("-n") {
                let (n, neg) = parse_count(after);
                if neg {
                    skip_last_lines = Some(n);
                } else {
                    lines = n;
                }
            } else if arg == "-c" && i + 1 < args.len() {
                i += 1;
                let (n, neg) = parse_count(&args[i]);
                if neg {
                    skip_last_bytes = Some(n);
                } else {
                    bytes = Some(n);
                }
            } else if arg.starts_with("-c") && arg.len() > 2 {
                let (n, neg) = parse_count(&arg[2..]);
                if neg {
                    skip_last_bytes = Some(n);
                } else {
                    bytes = Some(n);
                }
            } else if arg == "-q" || arg == "--quiet" || arg == "--silent" {
                force_quiet = true;
            } else if arg == "-v" || arg == "--verbose" {
                force_verbose = true;
            } else if arg.starts_with('-')
                && arg.len() > 1
                && arg[1..].chars().all(|c| c.is_ascii_digit())
            {
                lines = arg[1..].parse().unwrap_or(10);
            } else if !arg.starts_with('-') || arg == "-" {
                files.push(arg);
            } else if arg == "--" {
                // end of options — collect rest as files
                i += 1;
                while i < args.len() {
                    files.push(&args[i]);
                    i += 1;
                }
                break;
            } else {
                // coreutils head rejects unknown flags. Silent
                // fall-through made `head -X foo` print foo's first
                // 10 lines while losing the -X signal.
                eprintln!("head: unrecognized option: '{}'", arg);
                return 1;
            }
            i += 1;
        }

        if files.is_empty() {
            files.push("-");
        }

        // coreutils: header on iff >1 file. -q forces off, -v forces on.
        let show_headers = if force_quiet {
            false
        } else if force_verbose {
            true
        } else {
            files.len() > 1
        };
        let stdout = std::io::stdout();
        let mut out = stdout.lock();

        for (idx, file) in files.iter().enumerate() {
            if show_headers {
                if idx > 0 {
                    let _ = writeln!(out);
                }
                let _ = writeln!(out, "==> {} <==", file);
            }

            if let Some(skip) = skip_last_bytes {
                // -c -N: read everything, drop last N bytes.
                let mut reader: Box<dyn Read> = if *file == "-" {
                    Box::new(std::io::stdin())
                } else {
                    match std::fs::File::open(file) {
                        Ok(f) => Box::new(f),
                        Err(e) => {
                            eprintln!(
                                "head: {}: {}",
                                file,
                                crate::ported::compat::strerror(e.raw_os_error().unwrap_or(0))
                            );
                            return 1;
                        }
                    }
                };
                let mut buf = Vec::new();
                let _ = reader.read_to_end(&mut buf);
                let end = buf.len().saturating_sub(skip);
                let _ = out.write_all(&buf[..end]);
                continue;
            }

            if let Some(n) = bytes {
                // -c N: byte-count mode. Read up to N bytes and write.
                let mut reader: Box<dyn Read> = if *file == "-" {
                    Box::new(std::io::stdin())
                } else {
                    match std::fs::File::open(file) {
                        Ok(f) => Box::new(f),
                        Err(e) => {
                            eprintln!(
                                "head: {}: {}",
                                file,
                                crate::ported::compat::strerror(e.raw_os_error().unwrap_or(0))
                            );
                            return 1;
                        }
                    }
                };
                let mut buf = vec![0u8; n];
                let mut total = 0usize;
                while total < n {
                    match reader.read(&mut buf[total..]) {
                        Ok(0) => break,
                        Ok(k) => total += k,
                        Err(_) => break,
                    }
                }
                let _ = out.write_all(&buf[..total]);
                continue;
            }

            let reader: Box<dyn BufRead> = if *file == "-" {
                Box::new(BufReader::new(std::io::stdin()))
            } else {
                match std::fs::File::open(file) {
                    Ok(f) => Box::new(BufReader::new(f)),
                    Err(e) => {
                        eprintln!(
                            "head: {}: {}",
                            file,
                            crate::ported::compat::strerror(e.raw_os_error().unwrap_or(0))
                        );
                        return 1;
                    }
                }
            };

            if let Some(skip) = skip_last_lines {
                // -n -N: collect all lines, emit all except the last
                // N. Direct port of coreutils head -n -N.
                let all: Vec<String> = reader.lines().map_while(Result::ok).collect();
                let end = all.len().saturating_sub(skip);
                for line in &all[..end] {
                    let _ = writeln!(out, "{}", line);
                }
            } else {
                for line in reader.lines().take(lines) {
                    match line {
                        Ok(l) => {
                            let _ = writeln!(out, "{}", l);
                        }
                        Err(_) => break,
                    }
                }
            }
        }
        0
    }

    pub(crate) fn builtin_tail(&self, args: &[String]) -> i32 {
        let mut lines = 10usize;
        // Some(N) when -c N was given — switches to byte-count mode.
        let mut bytes: Option<usize> = None;
        // -n +N / -c +N: start at line/byte N (1-based) instead of
        // tailing the last N. coreutils extension.
        let mut start_line: Option<usize> = None;
        let mut start_byte: Option<usize> = None;
        let mut force_quiet = false;
        let mut force_verbose = false;
        let mut files: Vec<&str> = Vec::new();
        let mut i = 0;

        let parse_count = |s: &str| -> (usize, bool) {
            // Returns (count, from_start_flag).
            if let Some(rest) = s.strip_prefix('+') {
                (rest.parse().unwrap_or(0), true)
            } else if let Some(rest) = s.strip_prefix('-') {
                (rest.parse().unwrap_or(0), false)
            } else {
                (s.parse().unwrap_or(0), false)
            }
        };

        while i < args.len() {
            let arg = &args[i];
            if arg == "-n" && i + 1 < args.len() {
                i += 1;
                let (n, from_start) = parse_count(&args[i]);
                if from_start {
                    start_line = Some(n);
                } else {
                    lines = n;
                }
            } else if let Some(after) = arg.strip_prefix("-n") {
                let (n, from_start) = parse_count(after);
                if from_start {
                    start_line = Some(n);
                } else {
                    lines = n;
                }
            } else if arg == "-c" && i + 1 < args.len() {
                i += 1;
                let (n, from_start) = parse_count(&args[i]);
                if from_start {
                    start_byte = Some(n);
                } else {
                    bytes = Some(n);
                }
            } else if arg.starts_with("-c") && arg.len() > 2 {
                let (n, from_start) = parse_count(&arg[2..]);
                if from_start {
                    start_byte = Some(n);
                } else {
                    bytes = Some(n);
                }
            } else if arg == "-q" || arg == "--quiet" || arg == "--silent" {
                force_quiet = true;
            } else if arg == "-v" || arg == "--verbose" {
                force_verbose = true;
            } else if arg.starts_with('-')
                && arg.len() > 1
                && arg[1..].chars().all(|c| c.is_ascii_digit())
            {
                lines = arg[1..].parse().unwrap_or(10);
            } else if !arg.starts_with('-') || arg == "-" {
                files.push(arg);
            } else if arg == "--" {
                i += 1;
                while i < args.len() {
                    files.push(&args[i]);
                    i += 1;
                }
                break;
            } else if arg == "-f" || arg == "--follow" {
                // -f (follow): not yet wired through; accept as no-op
                // for compat. coreutils-style \`tail -f\` would need a
                // separate streaming loop.
            } else {
                eprintln!("tail: unrecognized option: '{}'", arg);
                return 1;
            }
            i += 1;
        }

        if files.is_empty() {
            files.push("-");
        }

        // coreutils: header on iff >1 file. -q forces off, -v forces on.
        let show_headers = if force_quiet {
            false
        } else if force_verbose {
            true
        } else {
            files.len() > 1
        };

        for (idx, file) in files.iter().enumerate() {
            if show_headers {
                if idx > 0 {
                    println!();
                }
                println!("==> {} <==", file);
            }

            let mut reader: Box<dyn BufRead> = if *file == "-" {
                Box::new(BufReader::new(std::io::stdin()))
            } else {
                match std::fs::File::open(file) {
                    Ok(f) => Box::new(BufReader::new(f)),
                    Err(e) => {
                        eprintln!(
                            "tail: {}: {}",
                            file,
                            crate::ported::compat::strerror(e.raw_os_error().unwrap_or(0))
                        );
                        return 1;
                    }
                }
            };

            if let Some(start) = start_byte {
                // -c +N: emit from byte N (1-based) onwards.
                let mut buf = Vec::new();
                let _ = reader.read_to_end(&mut buf);
                let s = start.saturating_sub(1).min(buf.len());
                let stdout = std::io::stdout();
                let _ = stdout.lock().write_all(&buf[s..]);
                continue;
            }

            if let Some(n) = bytes {
                // Byte-count mode: read everything into a buffer
                // (tail needs the END), keep last n bytes. Simple
                // approach matches BSD tail -c.
                let mut buf = Vec::new();
                let _ = reader.read_to_end(&mut buf);
                let start = buf.len().saturating_sub(n);
                let stdout = std::io::stdout();
                let _ = stdout.lock().write_all(&buf[start..]);
                continue;
            }

            if let Some(start) = start_line {
                // -n +N: emit from line N (1-based) onwards.
                // Streams without buffering the whole file.
                for (i, line) in reader.lines().map_while(Result::ok).enumerate() {
                    if i + 1 >= start {
                        println!("{}", line);
                    }
                }
                continue;
            }

            let mut ring: VecDeque<String> = VecDeque::with_capacity(lines);
            for line in reader.lines().map_while(Result::ok) {
                if ring.len() == lines {
                    ring.pop_front();
                }
                ring.push_back(line);
            }
            for line in ring {
                println!("{}", line);
            }
        }
        0
    }

    pub(crate) fn builtin_wc(&self, args: &[String]) -> i32 {
        let mut count_lines = false;
        let mut count_words = false;
        let mut count_bytes = false;
        let mut count_chars = false;
        // -L / --max-line-length: width of the longest input line.
        let mut count_max = false;
        let mut files: Vec<&str> = Vec::new();

        for arg in args {
            match arg.as_str() {
                "-l" => count_lines = true,
                "-w" => count_words = true,
                // coreutils wc(1): -c counts BYTES, -m counts unicode
                // codepoints. Was conflating them — broke wc on
                // multi-byte input where the user expected -m to
                // give char counts smaller than the byte count.
                "-c" => count_bytes = true,
                "-m" => count_chars = true,
                "-L" | "--max-line-length" => count_max = true,
                a if a.starts_with('-') => {
                    for c in a[1..].chars() {
                        match c {
                            'l' => count_lines = true,
                            'w' => count_words = true,
                            'c' => count_bytes = true,
                            'm' => count_chars = true,
                            'L' => count_max = true,
                            // coreutils wc errors on unknown short
                            // flags. \`wc -lXw foo\` previously counted
                            // lines+words while ignoring -X.
                            _ => {
                                eprintln!("wc: unrecognized option: '-{}'", c);
                                return 1;
                            }
                        }
                    }
                }
                _ => files.push(arg),
            }
        }

        if !count_lines && !count_words && !count_bytes && !count_chars && !count_max {
            count_lines = true;
            count_words = true;
            count_bytes = true;
        }

        if files.is_empty() {
            files.push("-");
        }

        let mut total_lines = 0usize;
        let mut total_words = 0usize;
        let mut total_bytes = 0usize;
        let mut total_chars = 0usize;
        let mut total_max = 0usize;

        for file in &files {
            let reader: Box<dyn BufRead> = if *file == "-" {
                Box::new(BufReader::new(std::io::stdin()))
            } else {
                match std::fs::File::open(file) {
                    Ok(f) => Box::new(BufReader::new(f)),
                    Err(e) => {
                        eprintln!(
                            "wc: {}: {}",
                            file,
                            crate::ported::compat::strerror(e.raw_os_error().unwrap_or(0))
                        );
                        return 1;
                    }
                }
            };

            let mut lines = 0usize;
            let mut words = 0usize;
            let mut bytes = 0usize;
            let mut chars = 0usize;
            let mut max_line: usize = 0;

            // POSIX wc: -l counts NEWLINE characters (a final line
            // without `\n` adds no line), -c counts raw bytes. The
            // previous `reader.lines()` loop stripped `\n`/`\r\n`
            // and re-added a flat `+1` per line — `printf "\r" |
            // wc -c` reported 2 (1 content byte + phantom newline)
            // where wc(1) reports 1, and CRLF input undercounted.
            // Stream raw bytes: count `\n` for lines, UTF-8 lead
            // bytes (non-0x80-continuation) for -m chars, ASCII
            // isspace transitions for words.
            let mut rdr = reader;
            let mut in_word = false;
            let mut cur_line_len = 0usize;
            let mut chunk = [0u8; 65536];
            loop {
                let n = match rdr.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(_) => break,
                };
                bytes += n;
                for &b in &chunk[..n] {
                    let is_lead = (b & 0xC0) != 0x80;
                    if is_lead {
                        chars += 1;
                    }
                    if b == b'\n' {
                        lines += 1;
                        if cur_line_len > max_line {
                            max_line = cur_line_len;
                        }
                        cur_line_len = 0;
                    } else if is_lead {
                        cur_line_len += 1;
                    }
                    if matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
                        in_word = false;
                    } else if !in_word {
                        in_word = true;
                        words += 1;
                    }
                }
            }
            if cur_line_len > max_line {
                max_line = cur_line_len;
            }

            total_lines += lines;
            total_words += words;
            total_bytes += bytes;
            total_chars += chars;
            if max_line > total_max {
                total_max = max_line;
            }

            let mut out = String::new();
            if count_lines {
                out.push_str(&format!("{:8}", lines));
            }
            if count_words {
                out.push_str(&format!("{:8}", words));
            }
            if count_bytes {
                out.push_str(&format!("{:8}", bytes));
            }
            if count_chars {
                out.push_str(&format!("{:8}", chars));
            }
            if count_max {
                out.push_str(&format!("{:8}", max_line));
            }
            if *file != "-" {
                out.push_str(&format!(" {}", file));
            }
            // BSD wc (what zsh uses on macOS) preserves the 8-char
            // right-aligned padding even on stdin output. trim_start
            // here was stripping it; output then differed from zsh.
            println!("{}", out);
        }

        if files.len() > 1 {
            let mut out = String::new();
            if count_lines {
                out.push_str(&format!("{:8}", total_lines));
            }
            if count_words {
                out.push_str(&format!("{:8}", total_words));
            }
            if count_bytes {
                out.push_str(&format!("{:8}", total_bytes));
            }
            if count_chars {
                out.push_str(&format!("{:8}", total_chars));
            }
            if count_max {
                out.push_str(&format!("{:8}", total_max));
            }
            out.push_str(" total");
            println!("{}", out.trim_start());
        }
        0
    }

    pub(crate) fn builtin_basename(&self, args: &[String]) -> i32 {
        // coreutils basename(1) port. Adds:
        // - -a / --multiple: every operand is a path (suffix not
        //   consumed positionally).
        // - -s SUFFIX: implies -a, supplies suffix to strip.
        // - -z / --zero: NUL-terminate output.
        if args.is_empty() {
            eprintln!("basename: missing operand");
            return 1;
        }
        let mut multiple = false;
        let mut suffix: Option<String> = None;
        let mut zero = false;
        let mut positional: Vec<&str> = Vec::new();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-a" | "--multiple" => multiple = true,
                "-s" | "--suffix" => {
                    if let Some(s) = iter.next() {
                        suffix = Some(s.clone());
                        multiple = true;
                    }
                }
                "-z" | "--zero" => zero = true,
                "--" => {} // accept end-of-options
                s if s.starts_with("-s") && s.len() > 2 => {
                    suffix = Some(s[2..].to_string());
                    multiple = true;
                }
                s if s.starts_with('-') && s.len() > 1 => {
                    // coreutils basename rejects unknown flags. Old
                    // silent-ignore made `basename -Z foo` succeed
                    // returning `foo` while losing the -Z signal.
                    eprintln!("basename: unrecognized option: '{}'", s);
                    return 1;
                }
                s => positional.push(s),
            }
        }
        if positional.is_empty() {
            eprintln!("basename: missing operand");
            return 1;
        }
        // Without -a / -s, the legacy 2-arg form: NAME [SUFFIX]
        // applies the second operand as a suffix to strip from the
        // first.
        let term = if zero { '\0' } else { '\n' };
        let strip_suffix = |name: &mut String, suf: &str| {
            if name.ends_with(suf) && name.len() > suf.len() {
                let new_len = name.len() - suf.len();
                name.truncate(new_len);
            }
        };
        let basename = |path: &str| -> String {
            let trimmed = path.trim_end_matches('/');
            let t = if trimmed.is_empty() { path } else { trimmed };
            std::path::Path::new(t)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string())
        };
        if multiple {
            for p in &positional {
                let mut name = basename(p);
                if let Some(ref s) = suffix {
                    strip_suffix(&mut name, s);
                }
                print!("{}{}", name, term);
            }
        } else {
            let path = positional[0];
            let arg_suffix = positional.get(1).copied();
            let mut name = basename(path);
            if let Some(s) = arg_suffix {
                strip_suffix(&mut name, s);
            }
            print!("{}{}", name, term);
        }
        0
    }

    pub(crate) fn builtin_dirname(&self, args: &[String]) -> i32 {
        // coreutils dirname(1) port. Strip flags before walking
        // operands, support -z / --zero (NUL-terminate output
        // instead of newline).
        if args.is_empty() {
            eprintln!("dirname: missing operand");
            return 1;
        }
        let mut zero = false;
        let mut paths: Vec<&str> = Vec::new();
        for arg in args {
            match arg.as_str() {
                "-z" | "--zero" => zero = true,
                "--" => {} // accept end-of-options
                s if s.starts_with('-') && s.len() > 1 => {
                    // coreutils dirname rejects unknown flags with
                    // \"unrecognized option\" exit 1. Silent-ignore
                    // masked typos like \`dirname -Z foo\` (typo of -z).
                    eprintln!("dirname: unrecognized option: '{}'", s);
                    return 1;
                }
                s => paths.push(s),
            }
        }
        if paths.is_empty() {
            eprintln!("dirname: missing operand");
            return 1;
        }
        let term = if zero { '\0' } else { '\n' };
        for path in paths {
            // POSIX dirname: trailing '/' chars on a non-root path
            // collapse; '/foo' → '/'; 'foo' → '.'.
            let trimmed: &str = if path.is_empty() {
                "."
            } else {
                let t = path.trim_end_matches('/');
                if t.is_empty() {
                    "/"
                } else {
                    t
                }
            };
            let dir = std::path::Path::new(trimmed)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| ".".to_string());
            let out = if dir.is_empty() { ".".to_string() } else { dir };
            print!("{}{}", out, term);
        }
        0
    }

    pub(crate) fn builtin_touch(&self, args: &[String]) -> i32 {
        // coreutils touch(1) port: -a/-m, -c (no create), -r REF
        // (copy times from REF), -t [[CC]YY]MMDDhhmm[.SS] (POSIX
        // timestamp, local time). -d / -h are rejected as
        // unrecognized — they need date-string parsing through
        // reverse_strftime.

        if args.is_empty() {
            eprintln!("touch: missing file operand");
            return 1;
        }

        let mut atime_only = false;
        let mut mtime_only = false;
        let mut no_create = false;
        let mut reference: Option<String> = None;
        let mut stamp: Option<String> = None;
        let mut files: Vec<&str> = Vec::new();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-a" => atime_only = true,
                "-m" => mtime_only = true,
                "-c" | "--no-create" => no_create = true,
                "-r" | "--reference" => {
                    if let Some(r) = iter.next() {
                        reference = Some(r.clone());
                    }
                }
                "-t" => match iter.next() {
                    Some(t) => stamp = Some(t.clone()),
                    None => {
                        eprintln!("touch: option requires an argument: '-t'");
                        return 1;
                    }
                },
                "--" => {} // accept; remaining are files
                s if s.starts_with('-') && s.len() > 1 => {
                    // -ac, -am combos: walk chars.
                    for c in s[1..].chars() {
                        match c {
                            'a' => atime_only = true,
                            'm' => mtime_only = true,
                            'c' => no_create = true,
                            // coreutils touch errors on unknown flag
                            // letters (esp. inside combined forms like
                            // \`-amX\`). Old \`_ => {}\` swallowed.
                            _ => {
                                eprintln!("touch: unrecognized option: '-{}'", c);
                                return 1;
                            }
                        }
                    }
                }
                _ => files.push(arg),
            }
        }

        // -t [[CC]YY]MMDDhhmm[.SS] — POSIX touch(1) timestamp,
        // interpreted in local time. YY 69-99 → 19YY, 00-68 → 20YY
        // (POSIX rule); 8-digit form defaults to the current year.
        fn parse_t_stamp(s: &str) -> Option<filetime::FileTime> {
            let (main, ss) = match s.split_once('.') {
                Some((m, sec)) => (m, sec.parse::<u32>().ok()?),
                None => (s, 0u32),
            };
            if !main.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let (year, rest): (i32, &str) = match main.len() {
                12 => (main[..4].parse().ok()?, &main[4..]),
                10 => {
                    let yy: i32 = main[..2].parse().ok()?;
                    (if yy >= 69 { 1900 + yy } else { 2000 + yy }, &main[2..])
                }
                8 => (chrono::Local::now().year(), main),
                _ => return None,
            };
            let month: u32 = rest[..2].parse().ok()?;
            let day: u32 = rest[2..4].parse().ok()?;
            let hour: u32 = rest[4..6].parse().ok()?;
            let min: u32 = rest[6..8].parse().ok()?;
            let dt = chrono::Local
                .with_ymd_and_hms(year, month, day, hour, min, ss)
                .single()?;
            Some(filetime::FileTime::from_unix_time(dt.timestamp(), 0))
        }

        // Determine the target times: from -t STAMP, -r REF, or now.
        let (target_atime, target_mtime) = if let Some(ref st) = stamp {
            match parse_t_stamp(st) {
                Some(ft) => (ft, ft),
                None => {
                    eprintln!("touch: out of range or illegal time specification: {}", st);
                    return 1;
                }
            }
        } else if let Some(ref refpath) = reference {
            match std::fs::metadata(refpath) {
                Ok(meta) => (
                    filetime::FileTime::from_last_access_time(&meta),
                    filetime::FileTime::from_last_modification_time(&meta),
                ),
                Err(e) => {
                    eprintln!("touch: {}: {}", refpath, e);
                    return 1;
                }
            }
        } else {
            let ft = filetime::FileTime::from_system_time(std::time::SystemTime::now());
            (ft, ft)
        };

        let mut status = 0;
        for file in files {
            let path = std::path::Path::new(file);
            if !path.exists() {
                if no_create {
                    continue;
                }
                if let Err(e) = OpenOptions::new()
                    .create(true)
                    .truncate(false) // touch only updates mtime; never truncates
                    .write(true)
                    .open(path)
                {
                    eprintln!("touch: {}: {}", file, e);
                    status = 1;
                    continue;
                }
            }
            // Write times: -a → atime only, -m → mtime only,
            // neither → both.
            let result = if atime_only && !mtime_only {
                filetime::set_file_atime(path, target_atime)
            } else if mtime_only && !atime_only {
                filetime::set_file_mtime(path, target_mtime)
            } else {
                filetime::set_file_times(path, target_atime, target_mtime)
            };
            if let Err(e) = result {
                eprintln!("touch: {}: {}", file, e);
                status = 1;
            }
        }
        status
    }

    pub(crate) fn builtin_realpath(&self, args: &[String]) -> i32 {
        // coreutils realpath(1) port. Adds -q (quiet), -m (no-exist
        // check, logical resolution), -s (no symlink resolution),
        // and the implicit default (-e: every component must exist).
        if args.is_empty() {
            eprintln!("realpath: missing operand");
            return 1;
        }
        let mut quiet = false;
        let mut allow_missing = false;
        let mut no_symlinks = false;
        let mut paths: Vec<&str> = Vec::new();
        for arg in args {
            match arg.as_str() {
                "-q" | "--quiet" => quiet = true,
                "-m" | "--canonicalize-missing" => allow_missing = true,
                "-s" | "--strip" | "--no-symlinks" => no_symlinks = true,
                "-e" | "--canonicalize-existing" => {
                    // Default behaviour; flag accepted for portability.
                }
                "-L" | "--logical" => no_symlinks = true,
                "-P" | "--physical" => no_symlinks = false,
                s if s.starts_with('-') => {
                    // coreutils realpath rejects unknown flags with
                    // \"unrecognized option\" exit 1.
                    eprintln!("realpath: unrecognized option: '{}'", s);
                    return 1;
                }
                _ => paths.push(arg.as_str()),
            }
        }

        // Logical normalize: collapse `.` and `..` components without
        // following symlinks. Used by -m / -s. Direct port of
        // coreutils canonicalize_filename_mode in the LOGICAL case.
        let logical_normalize = |p: &std::path::Path| -> std::path::PathBuf {
            let mut abs: std::path::PathBuf = if p.is_absolute() {
                std::path::PathBuf::new()
            } else {
                std::env::current_dir().unwrap_or_default()
            };
            for comp in p.components() {
                match comp {
                    Prefix(_) | RootDir => abs.push(comp.as_os_str()),
                    CurDir => {}
                    ParentDir => {
                        abs.pop();
                    }
                    Normal(c) => abs.push(c),
                }
            }
            abs
        };

        let mut status = 0;
        for path in &paths {
            let p = std::path::Path::new(path);
            let result: Result<std::path::PathBuf, std::io::Error> = if allow_missing || no_symlinks
            {
                Ok(logical_normalize(p))
            } else {
                std::fs::canonicalize(p)
            };
            match result {
                Ok(abs) => println!("{}", abs.display()),
                Err(e) => {
                    if !quiet {
                        eprintln!("realpath: {}: {}", path, e);
                    }
                    status = 1;
                }
            }
        }
        status
    }

    pub(crate) fn builtin_sort(&self, args: &[String]) -> i32 {
        // coreutils sort(1) port — adds case-fold (-f), field
        // selection (-k N), custom separator (-t C) on top of the
        // existing -n / -r / -u handling.

        let mut reverse = false;
        let mut numeric = false;
        let mut unique = false;
        let mut fold = false;
        // -h / --human-numeric-sort: 1K / 5M / 2G suffix-aware compare.
        let mut human_numeric = false;
        // -V / --version-sort: natural (1, 2, 10) instead of (1, 10, 2).
        let mut version_sort = false;
        // -R / --random-sort: random shuffle.
        let mut random_sort = false;
        // -z / --zero-terminated: input records separated by NUL.
        let mut zero_term = false;
        // -b / --ignore-leading-blanks: strip leading whitespace
        // before comparing.
        let mut ignore_blanks = false;
        // -d / --dictionary-order: only [a-zA-Z0-9 \\t] are significant
        // for comparison; everything else folds to nothing.
        let mut dictionary = false;
        // -c / --check: verify input is sorted; don't write output.
        let mut check_only = false;
        // -k FIELD: sort by field N (1-based, N-M range).
        let mut key_start: Option<usize> = None;
        let mut key_end: Option<usize> = None;
        // -t C: field separator. Default is run-of-whitespace.
        let mut sep: Option<char> = None;
        let mut files: Vec<&str> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            match arg.as_str() {
                "-r" | "--reverse" => reverse = true,
                "-n" | "--numeric-sort" => numeric = true,
                "-u" | "--unique" => unique = true,
                "-f" | "--ignore-case" => fold = true,
                "-h" | "--human-numeric-sort" => human_numeric = true,
                "-V" | "--version-sort" => version_sort = true,
                "-R" | "--random-sort" => random_sort = true,
                "-z" | "--zero-terminated" => zero_term = true,
                "-b" | "--ignore-leading-blanks" => ignore_blanks = true,
                "-d" | "--dictionary-order" => dictionary = true,
                "-c" | "--check" => check_only = true,
                "-k" if i + 1 < args.len() => {
                    i += 1;
                    if let Some((a, b)) = args[i].split_once(',') {
                        key_start = a.split('.').next().and_then(|s| s.parse().ok());
                        key_end = b.split('.').next().and_then(|s| s.parse().ok());
                    } else {
                        key_start = args[i].split('.').next().and_then(|s| s.parse().ok());
                    }
                }
                "-t" if i + 1 < args.len() => {
                    i += 1;
                    sep = args[i].chars().next();
                }
                a if a.starts_with("-k") && a.len() > 2 => {
                    let s = &a[2..];
                    if let Some((aa, bb)) = s.split_once(',') {
                        key_start = aa.split('.').next().and_then(|s| s.parse().ok());
                        key_end = bb.split('.').next().and_then(|s| s.parse().ok());
                    } else {
                        key_start = s.split('.').next().and_then(|s| s.parse().ok());
                    }
                }
                a if a.starts_with("-t") && a.len() > 2 => {
                    sep = a.chars().nth(2);
                }
                a if a.starts_with('-') && a.len() > 1 => {
                    for c in a[1..].chars() {
                        match c {
                            'r' => reverse = true,
                            'n' => numeric = true,
                            'u' => unique = true,
                            'f' => fold = true,
                            'h' => human_numeric = true,
                            'V' => version_sort = true,
                            'R' => random_sort = true,
                            'z' => zero_term = true,
                            'b' => ignore_blanks = true,
                            'd' => dictionary = true,
                            'c' => check_only = true,
                            // coreutils sort errors on unknown short
                            // flags. Old `_ => {}` masked typos like
                            // `sort -X` (treating it as a no-op).
                            _ => {
                                eprintln!("sort: unrecognized option: '-{}'", c);
                                return 1;
                            }
                        }
                    }
                }
                _ => files.push(arg),
            }
            i += 1;
        }

        let mut lines: Vec<String> = Vec::new();
        if zero_term {
            // Read raw bytes, split on NUL.
            let mut buf = Vec::new();
            if files.is_empty() {
                let _ = std::io::stdin().read_to_end(&mut buf);
            } else {
                for file in &files {
                    match std::fs::File::open(file) {
                        Ok(mut f) => {
                            let _ = f.read_to_end(&mut buf);
                        }
                        Err(e) => {
                            eprintln!("sort: {}: {}", file, e);
                            return 1;
                        }
                    }
                }
            }
            for chunk in buf.split(|b| *b == 0) {
                if chunk.is_empty() {
                    continue;
                }
                lines.push(String::from_utf8_lossy(chunk).into_owned());
            }
        } else if files.is_empty() {
            let stdin = std::io::stdin();
            for line in stdin.lock().lines().map_while(Result::ok) {
                lines.push(line);
            }
        } else {
            for file in files {
                match std::fs::File::open(file) {
                    Ok(f) => {
                        for line in BufReader::new(f).lines().map_while(Result::ok) {
                            lines.push(line);
                        }
                    }
                    Err(e) => {
                        eprintln!("sort: {}: {}", file, e);
                        return 1;
                    }
                }
            }
        }

        // Extract the sort key from a line per -k/-t/-b/-d. Returns
        // the selected fields joined back with the separator (or just
        // the line when -k is absent), then optionally trimmed
        // (-b) and dictionary-filtered (-d).
        let extract_key = |line: &str| -> String {
            let raw = match key_start {
                Some(s) if s >= 1 => {
                    let start = s - 1;
                    let parts: Vec<&str> = match sep {
                        Some(c) => line.split(c).collect(),
                        None => line.split_whitespace().collect(),
                    };
                    let end = key_end
                        .map(|e| e.saturating_sub(1).min(parts.len().saturating_sub(1)))
                        .unwrap_or_else(|| parts.len().saturating_sub(1));
                    if start >= parts.len() {
                        String::new()
                    } else {
                        let sep_str = sep
                            .map(|c| c.to_string())
                            .unwrap_or_else(|| " ".to_string());
                        parts[start..=end].join(&sep_str)
                    }
                }
                _ => line.to_string(),
            };
            let blanks_stripped: String = if ignore_blanks {
                raw.trim_start().to_string()
            } else {
                raw
            };
            if dictionary {
                // Keep alnum + space/tab; drop everything else for
                // comparison. Direct port of coreutils sort -d.
                blanks_stripped
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == ' ' || *c == '\t')
                    .collect()
            } else {
                blanks_stripped
            }
        };

        // -h: parse '5K', '2.5M', '1G' etc. into an f64 with the
        // suffix multiplier. Direct port of coreutils sort -h.
        fn human_value(s: &str) -> f64 {
            let s = s.trim();
            // Strip optional leading '+'/'-' and a numeric prefix.
            let mut end = 0;
            let mut seen_dot = false;
            for (i, c) in s.char_indices() {
                if i == 0 && (c == '+' || c == '-') {
                    end = c.len_utf8();
                    continue;
                }
                if c.is_ascii_digit() {
                    end = i + c.len_utf8();
                } else if c == '.' && !seen_dot {
                    seen_dot = true;
                    end = i + c.len_utf8();
                } else {
                    break;
                }
            }
            let num: f64 = s[..end].parse().unwrap_or(0.0);
            let suffix = s[end..].chars().next();
            let mult = match suffix {
                Some('K') | Some('k') => 1_024.0,
                Some('M') => 1_048_576.0,
                Some('G') => 1_073_741_824.0,
                Some('T') => 1_099_511_627_776.0,
                Some('P') => 1_125_899_906_842_624.0,
                _ => 1.0,
            };
            num * mult
        }
        // -V: split into runs of (non-digit, digit) and compare
        // pairwise. Direct port of coreutils sort -V.
        fn version_compare(a: &str, b: &str) -> std::cmp::Ordering {
            let mut ai = a.chars().peekable();
            let mut bi = b.chars().peekable();
            loop {
                // Compare leading non-digit prefixes lexically.
                let (mut as_pre, mut bs_pre) = (String::new(), String::new());
                while let Some(&c) = ai.peek() {
                    if c.is_ascii_digit() {
                        break;
                    }
                    as_pre.push(c);
                    ai.next();
                }
                while let Some(&c) = bi.peek() {
                    if c.is_ascii_digit() {
                        break;
                    }
                    bs_pre.push(c);
                    bi.next();
                }
                let pre_cmp = as_pre.cmp(&bs_pre);
                if pre_cmp != std::cmp::Ordering::Equal {
                    return pre_cmp;
                }
                // Compare the digit run as integers.
                let mut as_num = String::new();
                let mut bs_num = String::new();
                while let Some(&c) = ai.peek() {
                    if !c.is_ascii_digit() {
                        break;
                    }
                    as_num.push(c);
                    ai.next();
                }
                while let Some(&c) = bi.peek() {
                    if !c.is_ascii_digit() {
                        break;
                    }
                    bs_num.push(c);
                    bi.next();
                }
                if as_num.is_empty() && bs_num.is_empty() {
                    return std::cmp::Ordering::Equal;
                }
                let an: u128 = as_num.parse().unwrap_or(0);
                let bn: u128 = bs_num.parse().unwrap_or(0);
                let num_cmp = an.cmp(&bn);
                if num_cmp != std::cmp::Ordering::Equal {
                    return num_cmp;
                }
            }
        }
        // Locale-collating compare, mirroring src/ported/sort.rs:246-253
        // (truncate at first NUL, then libc::strcoll).
        fn strcoll_cmp(a: &str, b: &str) -> std::cmp::Ordering {
            #[cfg(unix)]
            {
                let cstr_head = |s: &str| -> std::ffi::CString {
                    let bs = s.as_bytes();
                    let n = bs.iter().position(|&x| x == 0).unwrap_or(bs.len());
                    std::ffi::CString::new(&bs[..n])
                        .unwrap_or_else(|_| std::ffi::CString::new(vec![0u8]).expect("nul"))
                };
                let ca = cstr_head(a);
                let cb = cstr_head(b);
                let r = unsafe { libc::strcoll(ca.as_ptr(), cb.as_ptr()) };
                r.cmp(&0)
            }
            #[cfg(not(unix))]
            {
                a.cmp(b)
            }
        }
        let cmp_keys = |a: &str, b: &str| -> std::cmp::Ordering {
            let ka = extract_key(a);
            let kb = extract_key(b);
            if version_sort {
                version_compare(&ka, &kb)
            } else if human_numeric {
                human_value(&ka)
                    .partial_cmp(&human_value(&kb))
                    .unwrap_or(std::cmp::Ordering::Equal)
            } else if numeric {
                // sort(1) -n converts via strtod: the LEADING numeric
                // prefix counts, trailing text is ignored ("5.txt" →
                // 5, "10.txt" → 10). Whole-token parse() failed on
                // any suffix and collapsed everything to 0.0.
                let numeric_prefix = |s: &str| -> f64 {
                    let s = s.trim_start();
                    let mut end = 0;
                    let mut seen_dot = false;
                    for (i, c) in s.char_indices() {
                        if i == 0 && (c == '+' || c == '-') {
                            end = c.len_utf8();
                        } else if c.is_ascii_digit() {
                            end = i + c.len_utf8();
                        } else if c == '.' && !seen_dot {
                            seen_dot = true;
                        } else {
                            break;
                        }
                    }
                    // A bare sign or dot with no digits parses as 0,
                    // matching strtod's no-conversion result.
                    s[..end].parse().unwrap_or(0.0)
                };
                let na = numeric_prefix(&ka);
                let nb = numeric_prefix(&kb);
                na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal)
            } else if fold {
                strcoll_cmp(&ka.to_lowercase(), &kb.to_lowercase())
            } else {
                // sort(1) compares with strcoll under LC_COLLATE
                // (POSIX: "comparisons ... based on the collating
                // sequence of the current locale"). Byte-order
                // ka.cmp(&kb) diverged from /usr/bin/sort whenever
                // LC_ALL/LC_COLLATE is a non-C locale ("X" < "baz"
                // bytewise, "baz" < "X" in en_US.UTF-8). setlocale
                // (LC_ALL, "") runs at startup (vm_helper.rs), so
                // strcoll sees the user's locale — same pattern as
                // the zsh-sort port at src/ported/sort.rs:253.
                strcoll_cmp(&ka, &kb)
            }
        };

        // -c / --check: report whether input is sorted; never write
        // sorted output. Direct port of coreutils sort -c. Returns 1
        // (and prints diagnostic) on first out-of-order pair.
        if check_only {
            for w in lines.windows(2) {
                if cmp_keys(&w[0], &w[1]) == std::cmp::Ordering::Greater {
                    eprintln!("sort: -:?: disorder: {}", w[1]);
                    return 1;
                }
            }
            return 0;
        }

        if random_sort {
            // -R: shuffle. coreutils -R is a deterministic shuffle
            // keyed by an MD5 of the line, but a Fisher-Yates with
            // thread_rng is the standard approximation used by
            // sort-port crates.
            let mut rng = rand::thread_rng();
            lines.shuffle(&mut rng);
        } else {
            lines.sort_by(|a, b| cmp_keys(a, b));
        }

        if reverse {
            lines.reverse();
        }
        if unique {
            lines.dedup();
        }

        let term = if zero_term { '\0' } else { '\n' };
        for line in lines {
            print!("{}{}", line, term);
        }
        0
    }

    pub(crate) fn builtin_find(&self, args: &[String]) -> i32 {
        find_impl(args)
    }

    pub(crate) fn builtin_uniq(&self, args: &[String]) -> i32 {
        let mut count = false;
        let mut repeated = false;
        let mut unique_only = false;
        let mut ignore_case = false;
        // -z / --zero-terminated: input/output records separated by
        // NUL instead of \\n. coreutils extension; useful with
        // 'find -print0 | sort -z | uniq -z'.
        let mut zero_term = false;
        // -f N / --skip-fields=N: skip the first N whitespace-
        // separated fields when comparing.
        let mut skip_fields: usize = 0;
        // -s N / --skip-chars=N: skip N chars after the field-skip
        // before comparing.
        let mut skip_chars: usize = 0;
        let mut files: Vec<&str> = Vec::new();

        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-c" | "--count" => count = true,
                "-d" | "--repeated" => repeated = true,
                "-u" | "--unique" => unique_only = true,
                "-i" | "--ignore-case" => ignore_case = true,
                "-z" | "--zero-terminated" => zero_term = true,
                "-f" | "--skip-fields" => {
                    if let Some(n) = iter.next() {
                        skip_fields = n.parse().unwrap_or(0);
                    }
                }
                "-s" | "--skip-chars" => {
                    if let Some(n) = iter.next() {
                        skip_chars = n.parse().unwrap_or(0);
                    }
                }
                s if s.starts_with("-f") && s.len() > 2 => {
                    skip_fields = s[2..].parse().unwrap_or(0);
                }
                s if s.starts_with("-s") && s.len() > 2 => {
                    skip_chars = s[2..].parse().unwrap_or(0);
                }
                a if !a.starts_with('-') => files.push(a),
                "-" => files.push("-"),
                s => {
                    // coreutils uniq rejects unknown flags. Old `_ => {}`
                    // accepted any -X letter silently.
                    eprintln!("uniq: unrecognized option: '{}'", s);
                    return 1;
                }
            }
        }

        let reader: Box<dyn BufRead> = if files.is_empty() || files[0] == "-" {
            Box::new(BufReader::new(std::io::stdin()))
        } else {
            match std::fs::File::open(files[0]) {
                Ok(f) => Box::new(BufReader::new(f)),
                Err(e) => {
                    eprintln!("uniq: {}: {}", files[0], e);
                    return 1;
                }
            }
        };

        let mut prev: Option<String> = None;
        let mut cnt = 0usize;
        let key = |s: &str| -> String {
            // -f / -s: drop leading fields then leading chars before
            // comparing. Field separator is whitespace.
            let mut tail = s;
            for _ in 0..skip_fields {
                let trimmed = tail.trim_start();
                let after_field = trimmed
                    .find(|c: char| c.is_whitespace())
                    .map(|i| &trimmed[i..])
                    .unwrap_or("");
                tail = after_field;
            }
            let after_chars: String = tail.chars().skip(skip_chars).collect();
            if ignore_case {
                after_chars.to_lowercase()
            } else {
                after_chars
            }
        };
        let term = if zero_term { '\0' } else { '\n' };
        let emit = |p: &str, cnt: usize| {
            if repeated && cnt <= 1 {
                return;
            }
            if unique_only && cnt > 1 {
                return;
            }
            if count {
                print!("{:7} {}{}", cnt, p, term);
            } else {
                print!("{}{}", p, term);
            }
        };

        // -z: treat NUL as record separator. Otherwise BufRead::lines
        // splits on \n.
        if zero_term {
            let mut buf = Vec::new();
            let mut reader = reader;
            let _ = reader.read_to_end(&mut buf);
            for chunk in buf.split(|b| *b == 0) {
                let line = String::from_utf8_lossy(chunk).into_owned();
                if line.is_empty() && chunk.is_empty() {
                    continue;
                }
                if prev.as_ref().map(|p| key(p)) == Some(key(&line)) {
                    cnt += 1;
                } else {
                    if let Some(p) = prev.take() {
                        emit(&p, cnt);
                    }
                    prev = Some(line);
                    cnt = 1;
                }
            }
        } else {
            for line in reader.lines().map_while(Result::ok) {
                if prev.as_ref().map(|p| key(p)) == Some(key(&line)) {
                    cnt += 1;
                } else {
                    if let Some(p) = prev.take() {
                        emit(&p, cnt);
                    }
                    prev = Some(line);
                    cnt = 1;
                }
            }
        }

        if let Some(p) = prev {
            emit(&p, cnt);
        }
        0
    }

    pub(crate) fn builtin_cut(&self, args: &[String]) -> i32 {
        // coreutils cut(1) port: parses -d / -f / -c / -b ranges
        // including N-M, N-, -M shorthand and comma-lists.

        #[derive(Copy, Clone)]
        enum Mode {
            Field,
            Char,
            Byte,
        }
        let mut delimiter = '\t';
        let mut output_delimiter: Option<String> = None;
        let mut mode = Mode::Field;
        // Each entry is (start, end) inclusive, 0-based. end == usize::MAX
        // means "to end of line".
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        let mut suppress_no_delim = false;
        // -z / --zero-terminated: line delim becomes NUL.
        let mut zero_term = false;
        // --complement: print complement of selected ranges.
        let mut complement = false;
        let mut files: Vec<&str> = Vec::new();
        let mut i = 0;

        // Parse a coreutils-style cut spec: 'N', 'N-M', 'N-', '-M',
        // separated by ','.
        let parse_spec = |spec: &str, out: &mut Vec<(usize, usize)>| {
            for part in spec.split(',') {
                if part.is_empty() {
                    continue;
                }
                if let Some((a, b)) = part.split_once('-') {
                    let start = if a.is_empty() {
                        0
                    } else {
                        match a.parse::<usize>() {
                            Ok(n) if n > 0 => n - 1,
                            _ => continue,
                        }
                    };
                    let end = if b.is_empty() {
                        usize::MAX
                    } else {
                        match b.parse::<usize>() {
                            Ok(n) if n > 0 => n - 1,
                            _ => continue,
                        }
                    };
                    if start <= end {
                        out.push((start, end));
                    }
                } else if let Ok(n) = part.parse::<usize>() {
                    if n > 0 {
                        out.push((n - 1, n - 1));
                    }
                }
            }
        };

        while i < args.len() {
            let arg = &args[i];
            if arg == "-d" && i + 1 < args.len() {
                i += 1;
                delimiter = args[i].chars().next().unwrap_or('\t');
            } else if let Some(s) = arg.strip_prefix("-d") {
                delimiter = s.chars().next().unwrap_or('\t');
            } else if arg == "-f" && i + 1 < args.len() {
                i += 1;
                mode = Mode::Field;
                parse_spec(&args[i], &mut ranges);
            } else if let Some(s) = arg.strip_prefix("-f") {
                mode = Mode::Field;
                parse_spec(s, &mut ranges);
            } else if arg == "-c" && i + 1 < args.len() {
                i += 1;
                mode = Mode::Char;
                parse_spec(&args[i], &mut ranges);
            } else if let Some(s) = arg.strip_prefix("-c") {
                mode = Mode::Char;
                parse_spec(s, &mut ranges);
            } else if arg == "-b" && i + 1 < args.len() {
                i += 1;
                mode = Mode::Byte;
                parse_spec(&args[i], &mut ranges);
            } else if let Some(s) = arg.strip_prefix("-b") {
                mode = Mode::Byte;
                parse_spec(s, &mut ranges);
            } else if arg == "-s" || arg == "--only-delimited" {
                suppress_no_delim = true;
            } else if arg == "-z" || arg == "--zero-terminated" {
                zero_term = true;
            } else if arg == "--complement" {
                complement = true;
            } else if let Some(s) = arg.strip_prefix("--output-delimiter=") {
                output_delimiter = Some(s.to_string());
            } else if arg == "--output-delimiter" && i + 1 < args.len() {
                i += 1;
                output_delimiter = Some(args[i].clone());
            } else if arg == "-" {
                files.push("-");
            } else if arg == "--" {
                i += 1;
                while i < args.len() {
                    files.push(&args[i]);
                    i += 1;
                }
                break;
            } else if !arg.starts_with('-') {
                files.push(arg);
            } else {
                eprintln!("cut: unrecognized option: '{}'", arg);
                return 1;
            }
            i += 1;
        }

        let in_range = |idx: usize| -> bool {
            let m = ranges.iter().any(|(s, e)| idx >= *s && idx <= *e);
            if complement {
                !m
            } else {
                m
            }
        };

        let reader: Box<dyn BufRead> = if files.is_empty() || files[0] == "-" {
            Box::new(BufReader::new(std::io::stdin()))
        } else {
            match std::fs::File::open(files[0]) {
                Ok(f) => Box::new(BufReader::new(f)),
                Err(e) => {
                    eprintln!("cut: {}: {}", files[0], e);
                    return 1;
                }
            }
        };

        let line_term = if zero_term { '\0' } else { '\n' };
        // -z splits input on NUL too; otherwise BufRead::lines splits
        // on \\n (the default).
        let process_line = |line: String| match mode {
            Mode::Field => {
                if !line.contains(delimiter) {
                    if !suppress_no_delim {
                        print!("{}{}", line, line_term);
                    }
                    return;
                }
                let parts: Vec<&str> = line.split(delimiter).collect();
                let selected: Vec<&str> = parts
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, p)| if in_range(idx) { Some(*p) } else { None })
                    .collect();
                let out_sep: String = output_delimiter
                    .clone()
                    .unwrap_or_else(|| delimiter.to_string());
                print!("{}{}", selected.join(&out_sep), line_term);
            }
            Mode::Char => {
                let chars: String = line
                    .chars()
                    .enumerate()
                    .filter_map(|(idx, c)| if in_range(idx) { Some(c) } else { None })
                    .collect();
                print!("{}{}", chars, line_term);
            }
            Mode::Byte => {
                let bytes: Vec<u8> = line
                    .bytes()
                    .enumerate()
                    .filter_map(|(idx, b)| if in_range(idx) { Some(b) } else { None })
                    .collect();
                print!("{}{}", String::from_utf8_lossy(&bytes), line_term);
            }
        };

        if zero_term {
            let mut buf = Vec::new();
            let mut reader = reader;
            let _ = reader.read_to_end(&mut buf);
            for chunk in buf.split(|b| *b == 0) {
                if chunk.is_empty() {
                    continue;
                }
                process_line(String::from_utf8_lossy(chunk).into_owned());
            }
        } else {
            for line in reader.lines().map_while(Result::ok) {
                process_line(line);
            }
        }
        0
    }

    pub(crate) fn builtin_tr(&self, args: &[String]) -> i32 {
        if args.is_empty() {
            eprintln!("tr: missing operand");
            return 1;
        }

        let delete = args.iter().any(|a| a == "-d" || a == "--delete");
        let complement = args
            .iter()
            .any(|a| a == "-c" || a == "-C" || a == "--complement");
        let squeeze = args.iter().any(|a| a == "-s" || a == "--squeeze-repeats");
        // -t / --truncate-set1: truncate set1 to set2's length.
        // Direct port of coreutils tr -t.
        let truncate1 = args.iter().any(|a| a == "-t" || a == "--truncate-set1");
        // Validate every flag-prefixed arg matches a known flag. Old
        // \`filter(|a| !starts_with('-'))\` consumed unknown flags
        // silently — \`tr -X 'a' 'b'\` would translate as if -X were
        // a no-op.
        for a in args {
            let s: &str = a.as_str();
            if s.starts_with('-')
                && s != "-d"
                && s != "--delete"
                && s != "-c"
                && s != "-C"
                && s != "--complement"
                && s != "-s"
                && s != "--squeeze-repeats"
                && s != "-t"
                && s != "--truncate-set1"
                && s.len() > 1
            {
                eprintln!("tr: unrecognized option: '{}'", s);
                return 1;
            }
        }
        let set1_raw: &str;
        let set2_raw: &str;

        let non_flag: Vec<&str> = args
            .iter()
            .filter(|a| !a.starts_with('-'))
            .map(|s| s.as_str())
            .collect();
        if delete {
            set1_raw = non_flag.first().copied().unwrap_or("");
            set2_raw = "";
        } else {
            set1_raw = non_flag.first().copied().unwrap_or("");
            set2_raw = non_flag.get(1).copied().unwrap_or("");
        }

        // Expand ranges like `a-z` into the full character list.
        // Handles escapes (\n \t \r \\ \0 \xNN \NNN) AND POSIX
        // character classes (`[:upper:]`, `[:lower:]`, `[:digit:]`,
        // `[:alpha:]`, `[:alnum:]`, `[:punct:]`, `[:space:]`,
        // `[:blank:]`, `[:cntrl:]`, `[:graph:]`, `[:print:]`,
        // `[:xdigit:]`).
        fn expand_set(s: &str) -> Vec<char> {
            let mut out = Vec::new();
            let bytes: Vec<char> = s.chars().collect();
            let mut i = 0;
            while i < bytes.len() {
                // [:CLASS:] character classes.
                if bytes[i] == '[' && i + 1 < bytes.len() && bytes[i + 1] == ':' {
                    if let Some(end) = bytes[i + 2..]
                        .iter()
                        .position(|&c| c == ':')
                        .map(|p| i + 2 + p)
                    {
                        if end + 1 < bytes.len() && bytes[end + 1] == ']' {
                            let class: String = bytes[i + 2..end].iter().collect();
                            for c in 0u32..128 {
                                if let Some(ch) = char::from_u32(c) {
                                    let m = match class.as_str() {
                                        "upper" => ch.is_ascii_uppercase(),
                                        "lower" => ch.is_ascii_lowercase(),
                                        "digit" => ch.is_ascii_digit(),
                                        "alpha" => ch.is_ascii_alphabetic(),
                                        "alnum" => ch.is_ascii_alphanumeric(),
                                        "punct" => ch.is_ascii_punctuation(),
                                        "space" => ch.is_ascii_whitespace(),
                                        "blank" => ch == ' ' || ch == '\t',
                                        "cntrl" => ch.is_ascii_control(),
                                        "graph" => ch.is_ascii_graphic(),
                                        "print" => ch.is_ascii_graphic() || ch == ' ',
                                        "xdigit" => ch.is_ascii_hexdigit(),
                                        _ => false,
                                    };
                                    if m {
                                        out.push(ch);
                                    }
                                }
                            }
                            i = end + 2;
                            continue;
                        }
                    }
                }
                let c = bytes[i];
                let resolved = if c == '\\' && i + 1 < bytes.len() {
                    let next = bytes[i + 1];
                    i += 1;
                    match next {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        'a' => '\x07',
                        'b' => '\x08',
                        'f' => '\x0c',
                        'v' => '\x0b',
                        '\\' => '\\',
                        // \xNN hex escape
                        'x' => {
                            let mut hex = String::new();
                            let mut j = i + 1;
                            while j < bytes.len() && hex.len() < 2 {
                                if bytes[j].is_ascii_hexdigit() {
                                    hex.push(bytes[j]);
                                    j += 1;
                                } else {
                                    break;
                                }
                            }
                            i = j - 1;
                            u32::from_str_radix(&hex, 16)
                                .ok()
                                .and_then(char::from_u32)
                                .unwrap_or('x')
                        }
                        // \NNN octal (1-3 digits)
                        d if d.is_digit(8) => {
                            let mut oct = String::from(d);
                            let mut j = i + 1;
                            while j < bytes.len() && oct.len() < 3 && bytes[j].is_digit(8) {
                                oct.push(bytes[j]);
                                j += 1;
                            }
                            i = j - 1;
                            u32::from_str_radix(&oct, 8)
                                .ok()
                                .and_then(char::from_u32)
                                .unwrap_or('\0')
                        }
                        other => other,
                    }
                } else {
                    c
                };
                if i + 2 < bytes.len() && bytes[i + 1] == '-' {
                    let end = bytes[i + 2];
                    if (resolved as u32) <= (end as u32) {
                        for cc in (resolved as u32)..=(end as u32) {
                            if let Some(c) = char::from_u32(cc) {
                                out.push(c);
                            }
                        }
                        i += 3;
                        continue;
                    }
                }
                out.push(resolved);
                i += 1;
            }
            out
        }

        let mut s1 = expand_set(set1_raw);
        let s2 = expand_set(set2_raw);
        // -t / --truncate-set1: shrink set1 to set2's length.
        if truncate1 && s1.len() > s2.len() {
            s1.truncate(s2.len());
        }

        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input).ok();

        let in_set1 = |c: char| -> bool {
            let m = s1.contains(&c);
            if complement {
                !m
            } else {
                m
            }
        };
        // Pick the squeeze set per coreutils tr semantics: with -d
        // and -s together, squeeze uses set2 (the second arg, or
        // empty if not given); without -d, squeeze uses set2 (the
        // translation target), falling back to set1 when set2 is
        // empty (the "tr -s" common form).
        let squeeze_set: Vec<char> = if !s2.is_empty() {
            s2.clone()
        } else {
            s1.clone()
        };

        let output_pre: String = if delete {
            input.chars().filter(|c| !in_set1(*c)).collect()
        } else if complement {
            // With -c (without -d), every char NOT in set1 maps to
            // the LAST char of set2 (or first if set2 has one). zsh
            // / coreutils tr semantics.
            let target = s2.last().copied().or_else(|| s2.first().copied());
            input
                .chars()
                .map(|c| {
                    if s1.contains(&c) {
                        c
                    } else if let Some(t) = target {
                        t
                    } else {
                        c
                    }
                })
                .collect()
        } else {
            input
                .chars()
                .map(|c| {
                    if let Some(pos) = s1.iter().position(|&x| x == c) {
                        s2.get(pos).or(s2.last()).copied().unwrap_or(c)
                    } else {
                        c
                    }
                })
                .collect()
        };

        // Squeeze pass: collapse runs of consecutive chars from
        // squeeze_set down to one occurrence. Direct port of
        // coreutils tr's squeeze_repeats.
        let output: String = if squeeze {
            let mut out = String::with_capacity(output_pre.len());
            let mut last: Option<char> = None;
            for c in output_pre.chars() {
                if Some(c) == last && squeeze_set.contains(&c) {
                    continue;
                }
                out.push(c);
                last = Some(c);
            }
            out
        } else {
            output_pre
        };

        print!("{}", output);
        0
    }

    pub(crate) fn builtin_seq(&self, args: &[String]) -> i32 {
        // coreutils seq(1): handles floats and -s SEPARATOR. The
        // previous impl only supported integers and emitted one per
        // line, so `seq -s , 1 5` printed five lines instead of
        // '1,2,3,4,5'.
        let mut sep = "\n".to_string();
        let mut nums_str: Vec<&str> = Vec::new();
        let mut equal_width = false;
        let mut format_str: Option<String> = None;
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if arg == "-s" && i + 1 < args.len() {
                i += 1;
                sep = args[i].clone();
            } else if let Some(s) = arg.strip_prefix("-s") {
                sep = s.to_string();
            } else if arg == "-w" || arg == "--equal-width" {
                equal_width = true;
            } else if arg == "-f" && i + 1 < args.len() {
                i += 1;
                format_str = Some(args[i].clone());
            } else if let Some(s) = arg.strip_prefix("-f") {
                format_str = Some(s.to_string());
            } else {
                nums_str.push(arg.as_str());
            }
            i += 1;
        }
        // Parse all-or-nothing as f64 to handle '0.5', '1e3', etc.
        let nums: Vec<f64> = nums_str.iter().filter_map(|a| a.parse().ok()).collect();
        if nums.len() != nums_str.len() {
            eprintln!("seq: invalid argument");
            return 1;
        }

        let (first, inc, last): (f64, f64, f64) = match nums.len() {
            1 => (1.0, 1.0, nums[0]),
            2 => (nums[0], 1.0, nums[1]),
            3 => (nums[0], nums[1], nums[2]),
            _ => {
                eprintln!("seq: missing operand");
                return 1;
            }
        };

        if inc == 0.0 {
            eprintln!("seq: zero increment");
            return 1;
        }
        // Derive output precision from the input args so 'seq 0.1 0.1
        // 0.5' prints '0.1\n0.2\n...'  and not the default float repr.
        let prec = nums_str
            .iter()
            .map(|s| s.split('.').nth(1).map(|f| f.len()).unwrap_or(0))
            .max()
            .unwrap_or(0);
        // Apply -f FORMAT (printf-style) when given. Supports the
        // common conversions: %d / %i (int), %f / %.Nf / %g / %e
        // (float). Other conversions fall through to the auto fmt.
        // Per coreutils, -f overrides equal_width.
        let user_fmt = format_str.clone();
        let fmt = move |v: f64| -> String {
            if let Some(f) = &user_fmt {
                // Replace the first %... conversion in f with the
                // formatted value. This is a tiny printf — full
                // coreutils format is more complex but this covers
                // 99% of \`seq -f '%.2f' 0 0.1 1\` style usage.
                let bytes = f.as_bytes();
                let mut out = String::with_capacity(f.len() + 16);
                let mut i = 0;
                let mut applied = false;
                while i < bytes.len() {
                    if bytes[i] == b'%' && i + 1 < bytes.len() {
                        if bytes[i + 1] == b'%' {
                            out.push('%');
                            i += 2;
                            continue;
                        }
                        // Find the conversion char.
                        let mut j = i + 1;
                        while j < bytes.len() {
                            let c = bytes[j];
                            if matches!(c, b'd' | b'i' | b'u' | b'f' | b'e' | b'g' | b'E' | b'G') {
                                break;
                            }
                            j += 1;
                        }
                        if j >= bytes.len() {
                            out.push('%');
                            i += 1;
                            continue;
                        }
                        let spec = std::str::from_utf8(&bytes[i..=j]).unwrap_or("%g");
                        let formatted = match bytes[j] {
                            b'd' | b'i' | b'u' => format!("{}", v as i64),
                            b'f' | b'e' | b'g' | b'E' | b'G' => {
                                // Extract precision if present (.N).
                                let s = spec.trim_start_matches('%');
                                let prec_part: String = s
                                    .chars()
                                    .skip_while(|c| *c != '.')
                                    .skip(1)
                                    .take_while(|c| c.is_ascii_digit())
                                    .collect();
                                let prec_n: usize = prec_part.parse().unwrap_or(6);
                                match bytes[j] {
                                    b'f' => format!("{:.p$}", v, p = prec_n),
                                    b'e' => format!("{:.p$e}", v, p = prec_n),
                                    b'E' => format!("{:.p$E}", v, p = prec_n),
                                    _ => format!("{:.p$}", v, p = prec_n),
                                }
                            }
                            _ => format!("{}", v),
                        };
                        out.push_str(&formatted);
                        applied = true;
                        i = j + 1;
                    } else {
                        out.push(bytes[i] as char);
                        i += 1;
                    }
                }
                let _ = applied;
                return out;
            }
            if prec == 0 {
                format!("{}", v as i64)
            } else {
                format!("{:.prec$}", v, prec = prec)
            }
        };

        let mut out: Vec<String> = Vec::new();
        let mut v = first;
        if inc > 0.0 {
            while v <= last + f64::EPSILON {
                out.push(fmt(v));
                v += inc;
            }
        } else {
            while v >= last - f64::EPSILON {
                out.push(fmt(v));
                v += inc;
            }
        }
        // -w: zero-pad each line to the longest output's width.
        // coreutils seq pads with leading zeros (or after sign) so
        // \`seq -w 8 10\` emits \`08 09 10\`. zshrs's previous "skip
        // silently" left them as \`8 9 10\`, breaking column-aligned
        // output.
        if equal_width && !out.is_empty() {
            let width = out.iter().map(|s| s.len()).max().unwrap_or(0);
            for s in &mut out {
                if s.len() < width {
                    let pad = width - s.len();
                    if let Some(rest) = s.strip_prefix('-') {
                        *s = format!("-{:0>pad$}{}", "", rest, pad = pad);
                    } else {
                        *s = format!("{:0>pad$}{}", "", s, pad = pad);
                    }
                }
            }
        }
        if !out.is_empty() {
            print!("{}", out.join(&sep));
            // coreutils seq always terminates the final line with `\n`,
            // even when -s SEPARATOR is given and the separator itself
            // is not a newline. Joining with sep leaves no trailing
            // terminator, so emit one unconditionally here.
            println!();
        }
        0
    }

    pub(crate) fn builtin_rev(&self, args: &[String]) -> i32 {
        // util-linux rev(1) port. Accepts multiple files; reverses
        // each line by chars (codepoint-correct, not bytes). One
        // bad file emits an error and continues with the rest;
        // returns 1 if any file failed.

        // util-linux rev has no flags. Reject any \`-\`-prefixed arg
        // that isn't \`-\` (stdin) or \`--\` (end-of-options).
        let mut files: Vec<&str> = Vec::new();
        let mut iter = args.iter();
        while let Some(a) = iter.next() {
            let s: &str = a.as_str();
            if s == "-" {
                files.push("-");
            } else if s == "--" {
                for rest in iter.by_ref() {
                    files.push(rest);
                }
                break;
            } else if s.starts_with('-') && s.len() > 1 {
                eprintln!("rev: unrecognized option: '{}'", s);
                return 1;
            } else {
                files.push(s);
            }
        }
        let targets: Vec<&str> = if files.is_empty() { vec!["-"] } else { files };
        let mut status = 0;
        for file in targets {
            let reader: Box<dyn BufRead> = if file == "-" {
                Box::new(BufReader::new(std::io::stdin()))
            } else {
                match std::fs::File::open(file) {
                    Ok(f) => Box::new(BufReader::new(f)),
                    Err(e) => {
                        eprintln!("rev: {}: {}", file, e);
                        status = 1;
                        continue;
                    }
                }
            };
            for line in reader.lines().map_while(Result::ok) {
                println!("{}", line.chars().rev().collect::<String>());
            }
        }
        status
    }

    pub(crate) fn builtin_tee(&self, args: &[String]) -> i32 {
        // coreutils tee(1) port: stream stdin to stdout AND each
        // named file in 8 KB chunks so 'tail -f log | tee out' works
        // — was buffering everything until EOF, which never arrives
        // for streaming sources.

        let mut append = false;
        let mut ignore_int = false;
        let mut files: Vec<&str> = Vec::new();
        for arg in args {
            match arg.as_str() {
                "-a" | "--append" => append = true,
                // -i: ignore SIGINT — coreutils flag for keeping the
                // process alive when the upstream gets ^C'd. We just
                // accept; real ignore wiring would need signal masks.
                "-i" | "--ignore-interrupts" => ignore_int = true,
                "--" => {} // end of options
                s if !s.starts_with('-') || s == "-" => files.push(s),
                s => {
                    // coreutils tee rejects unknown flags. Old
                    // \`_ => {}\` operated normally with -X dropped.
                    eprintln!("tee: unrecognized option: '{}'", s);
                    return 1;
                }
            }
        }
        let _ = ignore_int;

        // Open every output file once; bail per-file but keep going
        // for the rest (matches coreutils behaviour).
        let mut handles: Vec<Box<dyn Write>> = Vec::with_capacity(files.len());
        let mut returnval = 0;
        for file in &files {
            let result = if append {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(file)
            } else {
                std::fs::File::create(file)
            };
            match result {
                Ok(f) => handles.push(Box::new(f)),
                Err(e) => {
                    eprintln!("tee: {}: {}", file, e);
                    returnval = 1;
                }
            }
        }

        let stdin = std::io::stdin();
        let mut reader = stdin.lock();
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        let mut buf = [0u8; 8192];
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => break,
            };
            let _ = out.write_all(&buf[..n]);
            let _ = out.flush();
            for h in handles.iter_mut() {
                let _ = h.write_all(&buf[..n]);
                let _ = h.flush();
            }
        }
        returnval
    }

    pub(crate) fn builtin_sleep(&self, args: &[String]) -> i32 {
        if args.is_empty() {
            eprintln!("zshrs:sleep:1: missing operand");
            return 1;
        }

        let mut total_secs = 0.0f64;
        let mut had_operand = false;
        for arg in args {
            if arg.starts_with('-') && arg.len() > 1 {
                // coreutils sleep accepts no flags besides --help and
                // --version. Anything else is an error. Old impl
                // silently skipped flag args, so \`sleep -X 5\` slept
                // 5 seconds while losing -X.
                if arg == "--" {
                    continue; // end-of-options
                }
                eprintln!("zshrs:sleep:1: unrecognized option: '{}'", arg);
                return 1;
            }
            had_operand = true;
            let (num, suffix) = if arg.ends_with('s') {
                (&arg[..arg.len() - 1], 1.0)
            } else if arg.ends_with('m') {
                (&arg[..arg.len() - 1], 60.0)
            } else if arg.ends_with('h') {
                (&arg[..arg.len() - 1], 3600.0)
            } else if arg.ends_with('d') {
                (&arg[..arg.len() - 1], 86400.0)
            } else {
                (arg.as_str(), 1.0)
            };
            if let Ok(n) = num.parse::<f64>() {
                total_secs += n * suffix;
            } else {
                // coreutils sleep errors on non-numeric operand.
                eprintln!("zshrs:sleep:1: invalid time interval: '{}'", arg);
                return 1;
            }
        }
        let _ = had_operand;

        // Duration::from_secs_f64 panics on negative / NaN / +inf.
        // coreutils sleep treats negative as an error; here we
        // tolerate non-positive total as a no-op exit 0. Also cap
        // upper bound (Duration panics near u64::MAX seconds).
        if !total_secs.is_finite() || total_secs <= 0.0 {
            return 0;
        }
        let capped = if total_secs > i64::MAX as f64 {
            i64::MAX as f64
        } else {
            total_secs
        };
        std::thread::sleep(std::time::Duration::from_secs_f64(capped));
        0
    }

    /// paste [-d LIST] [FILE...] — merge lines of files. coreutils
    /// paste(1). Default delim is TAB; -d cycles through the
    /// supplied delimiter chars.
    pub(crate) fn builtin_paste(&self, args: &[String]) -> i32 {
        // paste(1) delimiter list: backslash escapes are decoded, and `\0`
        // means "no separator at all" — so a delimiter is a STRING, not a
        // char. Treating the list as raw chars emitted a literal backslash
        // where both BSD and GNU paste emit a newline (`paste -d'\n'`).
        fn decode_delims(spec: &str) -> Vec<String> {
            let mut out: Vec<String> = Vec::new();
            let mut it = spec.chars();
            while let Some(c) = it.next() {
                if c != '\\' {
                    out.push(c.to_string());
                    continue;
                }
                match it.next() {
                    Some('n') => out.push("\n".to_string()),
                    Some('t') => out.push("\t".to_string()),
                    Some('\\') => out.push("\\".to_string()),
                    Some('0') => out.push(String::new()), // empty separator
                    // A trailing lone backslash, or any other escape, stays
                    // literal — matching both implementations' leniency.
                    Some(other) => out.push(other.to_string()),
                    None => out.push("\\".to_string()),
                }
            }
            out
        }

        let mut delims: Vec<String> = vec!["\t".to_string()];
        let mut serial = false;
        let mut files: Vec<&str> = Vec::new();
        let mut iter = args.iter();
        let mut no_more_opts = false;
        while let Some(arg) = iter.next() {
            let a = arg.as_str();
            if no_more_opts || a == "-" || !a.starts_with('-') {
                files.push(a);
                continue;
            }
            match a {
                "--" => {
                    no_more_opts = true;
                    continue;
                }
                "--serial" => {
                    serial = true;
                    continue;
                }
                "--delimiters" => {
                    if let Some(s) = iter.next() {
                        delims = decode_delims(s);
                        if delims.is_empty() {
                            delims = vec!["\t".to_string()];
                        }
                    }
                    continue;
                }
                _ => {}
            }
            if let Some(spec) = a.strip_prefix("--delimiters=") {
                delims = decode_delims(spec);
                if delims.is_empty() {
                    delims = vec!["\t".to_string()];
                }
                continue;
            }
            // Short options CLUSTER, and `-d` takes the rest of the cluster as
            // its argument (or the next argv element when it ends the
            // cluster). `paste -sd, -` is the canonical spelling and was
            // rejected outright as an unknown option.
            let mut chars = a[1..].chars();
            let mut bad = false;
            while let Some(c) = chars.next() {
                match c {
                    's' => serial = true,
                    'd' => {
                        let rest: String = chars.by_ref().collect();
                        let spec = if rest.is_empty() {
                            iter.next().map(|s| s.as_str()).unwrap_or("")
                        } else {
                            &rest
                        };
                        delims = decode_delims(spec);
                        if delims.is_empty() {
                            delims = vec!["\t".to_string()];
                        }
                    }
                    _ => {
                        eprintln!("paste: unrecognized option: '{}'", a);
                        bad = true;
                        break;
                    }
                }
            }
            if bad {
                return 1;
            }
        }
        if files.is_empty() {
            files.push("-");
        }
        // Every `-` operand names the SAME stdin stream, so they must share
        // one reader: `paste - -` interleaves consecutive lines of one input
        // (`a<TAB>b`), which is the documented idiom for pairing up lines.
        // Giving each `-` its own BufReader let the first one buffer the
        // whole stream, so the second saw EOF and the output degraded to one
        // column per line.
        enum Src {
            Stdin,
            File(BufReader<std::fs::File>),
        }
        let mut stdin_reader = BufReader::new(std::io::stdin());
        let mut srcs: Vec<Src> = Vec::with_capacity(files.len());
        for file in &files {
            if *file == "-" {
                srcs.push(Src::Stdin);
            } else {
                match std::fs::File::open(file) {
                    Ok(f) => srcs.push(Src::File(BufReader::new(f))),
                    Err(e) => {
                        eprintln!("paste: {}: {}", file, e);
                        return 1;
                    }
                }
            }
        }
        fn next_line(src: &mut Src, stdin: &mut BufReader<std::io::Stdin>) -> Option<String> {
            let mut buf = String::new();
            let n = match src {
                Src::Stdin => stdin.read_line(&mut buf),
                Src::File(f) => f.read_line(&mut buf),
            };
            match n {
                Ok(0) | Err(_) => None,
                Ok(_) => {
                    if buf.ends_with('\n') {
                        buf.pop();
                        if buf.ends_with('\r') {
                            buf.pop();
                        }
                    }
                    Some(buf)
                }
            }
        }
        if serial {
            // -s: each file's lines on a single output line.
            for src in srcs.iter_mut() {
                let mut lines: Vec<String> = Vec::new();
                while let Some(l) = next_line(src, &mut stdin_reader) {
                    lines.push(l);
                }
                // An input with no lines produces NO output line at all —
                // `paste -s /dev/null` prints nothing, where emitting one
                // unconditionally added a stray blank line.
                if lines.is_empty() {
                    continue;
                }
                let mut out = String::new();
                for (i, l) in lines.iter().enumerate() {
                    out.push_str(l);
                    if i + 1 < lines.len() {
                        out.push_str(&delims[i % delims.len()]);
                    }
                }
                println!("{}", out);
            }
            return 0;
        }
        // Parallel-merge: round-robin one line from each operand.
        loop {
            let mut row: Vec<Option<String>> = Vec::with_capacity(srcs.len());
            for src in srcs.iter_mut() {
                row.push(next_line(src, &mut stdin_reader));
            }
            if row.iter().all(|c| c.is_none()) {
                break;
            }
            let mut out = String::new();
            for (i, cell) in row.iter().enumerate() {
                if let Some(s) = cell {
                    out.push_str(s);
                }
                if i + 1 < row.len() {
                    out.push_str(&delims[i % delims.len()]);
                }
            }
            println!("{}", out);
        }
        0
    }

    /// fold [-w WIDTH] [-s] [-b] [FILE...] — wrap input lines.
    /// coreutils fold(1).
    pub(crate) fn builtin_fold(&self, args: &[String]) -> i32 {
        let mut width: usize = 80;
        let mut break_at_space = false;
        let mut count_bytes = false;
        let mut files: Vec<&str> = Vec::new();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-w" | "--width" => {
                    if let Some(s) = iter.next() {
                        width = s.parse().unwrap_or(80);
                    }
                }
                s if s.starts_with("-w") && s.len() > 2 => {
                    width = s[2..].parse().unwrap_or(80);
                }
                "-s" | "--spaces" => break_at_space = true,
                "-b" | "--bytes" => count_bytes = true,
                "-" => files.push("-"),
                "--" => {
                    for rest in iter.by_ref() {
                        files.push(rest);
                    }
                    break;
                }
                s if !s.starts_with('-') => files.push(s),
                s => {
                    eprintln!("fold: unrecognized option: '{}'", s);
                    return 1;
                }
            }
        }
        if files.is_empty() {
            files.push("-");
        }
        for file in files {
            let reader: Box<dyn BufRead> = if file == "-" {
                Box::new(BufReader::new(std::io::stdin()))
            } else {
                match std::fs::File::open(file) {
                    Ok(f) => Box::new(BufReader::new(f)),
                    Err(e) => {
                        eprintln!("fold: {}: {}", file, e);
                        return 1;
                    }
                }
            };
            for line in reader.lines().map_while(Result::ok) {
                let mut chunk = String::new();
                let walker: Box<dyn Iterator<Item = char>> = if count_bytes {
                    // Treat each byte as a char (lossy for UTF-8).
                    Box::new(line.bytes().map(|b| b as char))
                } else {
                    Box::new(line.chars())
                };
                let mut col = 0usize;
                let mut last_space: Option<usize> = None;
                for c in walker {
                    chunk.push(c);
                    col += 1;
                    if c == ' ' || c == '\t' {
                        last_space = Some(chunk.len());
                    }
                    if col >= width {
                        if break_at_space {
                            if let Some(pos) = last_space {
                                let head = &chunk[..pos];
                                let tail = chunk[pos..].to_string();
                                println!("{}", head);
                                chunk = tail;
                                col = chunk.chars().count();
                                last_space = None;
                                continue;
                            }
                        }
                        println!("{}", chunk);
                        chunk.clear();
                        col = 0;
                        last_space = None;
                    }
                }
                if !chunk.is_empty() {
                    println!("{}", chunk);
                }
            }
        }
        0
    }

    /// shuf [-n N] [-i LO-HI] [-e [STR...]] [FILE] — random
    /// permutation. coreutils shuf(1).
    pub(crate) fn builtin_shuf(&self, args: &[String]) -> i32 {
        let mut count: Option<usize> = None;
        let mut input_range: Option<(i64, i64)> = None;
        let mut echo_args: Option<Vec<String>> = None;
        let mut zero_term = false;
        let mut file: Option<&str> = None;
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-n" | "--head-count" => {
                    if let Some(s) = iter.next() {
                        count = s.parse().ok();
                    }
                }
                "-i" | "--input-range" => {
                    if let Some(s) = iter.next() {
                        if let Some((a, b)) = s.split_once('-') {
                            if let (Ok(lo), Ok(hi)) = (a.parse::<i64>(), b.parse::<i64>()) {
                                input_range = Some((lo, hi));
                            }
                        }
                    }
                }
                "-e" | "--echo" => {
                    let rest: Vec<String> = iter.by_ref().cloned().collect();
                    echo_args = Some(rest);
                    break;
                }
                "-z" | "--zero-terminated" => zero_term = true,
                "-" => file = Some("-"),
                "--" => {
                    if let Some(rest) = iter.next() {
                        file = Some(rest.as_str());
                    }
                    break;
                }
                s if !s.starts_with('-') => file = Some(s),
                s => {
                    eprintln!("shuf: unrecognized option: '{}'", s);
                    return 1;
                }
            }
        }

        let mut items: Vec<String> = if let Some((lo, hi)) = input_range {
            (lo..=hi).map(|n| n.to_string()).collect()
        } else if let Some(echo) = echo_args {
            echo
        } else {
            let reader: Box<dyn BufRead> = match file {
                Some(f) if f != "-" => match std::fs::File::open(f) {
                    Ok(fh) => Box::new(BufReader::new(fh)),
                    Err(e) => {
                        eprintln!("shuf: {}: {}", f, e);
                        return 1;
                    }
                },
                _ => Box::new(BufReader::new(std::io::stdin())),
            };
            reader.lines().map_while(Result::ok).collect()
        };
        let mut rng = rand::thread_rng();
        items.shuffle(&mut rng);
        if let Some(n) = count {
            items.truncate(n);
        }
        let term = if zero_term { '\0' } else { '\n' };
        for item in items {
            print!("{}{}", item, term);
        }
        0
    }

    /// groups [USER...] — print group memberships. Coreutils
    /// groups(1) / POSIX. With no args, prints groups for the
    /// effective user; with args, prints "USER : group1 group2 ..."
    /// per user.
    pub(crate) fn builtin_groups(&self, args: &[String]) -> i32 {
        // Validate flags.
        let mut users: Vec<&str> = Vec::new();
        for arg in args {
            match arg.as_str() {
                "--" => {}
                s if s.starts_with('-') && s.len() > 1 => {
                    eprintln!("groups: unrecognized option: '{}'", s);
                    return 1;
                }
                s => users.push(s),
            }
        }
        let print_groups_for = |uid_or_name: Option<&str>| -> i32 {
            let (user_name, _user_uid, group_id): (String, u32, u32) = match uid_or_name {
                Some(name) => {
                    // Look up by name first, then numeric id.
                    let cn = match std::ffi::CString::new(name) {
                        Ok(c) => c,
                        Err(_) => return 1,
                    };
                    unsafe {
                        let pw = libc::getpwnam(cn.as_ptr());
                        if pw.is_null() {
                            // Try numeric.
                            if let Ok(uid) = name.parse::<u32>() {
                                let pw2 = libc::getpwuid(uid);
                                if pw2.is_null() {
                                    eprintln!("groups: '{}': no such user", name);
                                    return 1;
                                }
                                let n = CStr::from_ptr((*pw2).pw_name);
                                (
                                    n.to_string_lossy().into_owned(),
                                    (*pw2).pw_uid,
                                    (*pw2).pw_gid,
                                )
                            } else {
                                eprintln!("groups: '{}': no such user", name);
                                return 1;
                            }
                        } else {
                            let n = CStr::from_ptr((*pw).pw_name);
                            (n.to_string_lossy().into_owned(), (*pw).pw_uid, (*pw).pw_gid)
                        }
                    }
                }
                None => {
                    let euid = unsafe { libc::geteuid() };
                    unsafe {
                        let pw = libc::getpwuid(euid);
                        if pw.is_null() {
                            (String::new(), euid, 0)
                        } else {
                            let n = CStr::from_ptr((*pw).pw_name);
                            (n.to_string_lossy().into_owned(), (*pw).pw_uid, (*pw).pw_gid)
                        }
                    }
                }
            };
            // getgrouplist requires a buffer; start with 32 slots.
            let mut group_ids: Vec<libc::gid_t> = vec![0; 64];
            let mut ngroups: i32 = group_ids.len() as i32;
            let cn = std::ffi::CString::new(user_name.clone()).unwrap_or_default();
            let r = unsafe {
                libc::getgrouplist(
                    cn.as_ptr(),
                    group_id as _,
                    group_ids.as_mut_ptr() as *mut _,
                    &mut ngroups,
                )
            };
            if r < 0 {
                // Buffer too small — grow and retry.
                group_ids.resize(ngroups as usize, 0);
                unsafe {
                    libc::getgrouplist(
                        cn.as_ptr(),
                        group_id as _,
                        group_ids.as_mut_ptr() as *mut _,
                        &mut ngroups,
                    );
                }
            }
            group_ids.truncate(ngroups as usize);
            let names: Vec<String> = group_ids
                .iter()
                .map(|&g| unsafe {
                    let gr = libc::getgrgid(g);
                    if gr.is_null() {
                        g.to_string()
                    } else {
                        let n = CStr::from_ptr((*gr).gr_name);
                        n.to_string_lossy().into_owned()
                    }
                })
                .collect();
            if uid_or_name.is_some() {
                println!("{} : {}", user_name, names.join(" "));
            } else {
                println!("{}", names.join(" "));
            }
            0
        };
        if users.is_empty() {
            print_groups_for(None)
        } else {
            let mut status = 0;
            for u in users {
                if print_groups_for(Some(u)) != 0 {
                    status = 1;
                }
            }
            status
        }
    }

    /// users — print logged-in usernames. Coreutils users(1) /
    /// POSIX. Fallback minimal impl: prints \$USER (or current
    /// effective user via getpwuid) since fully reading utmp is
    /// platform-specific. Multi-user output not yet implemented;
    /// shell scripts that just check `[[ $(users) ]]` still work.
    pub(crate) fn builtin_users(&self, args: &[String]) -> i32 {
        for arg in args {
            if arg.starts_with('-') && arg.len() > 1 && arg != "--" {
                eprintln!("users: unrecognized option: '{}'", arg);
                return 1;
            }
            // POSIX `users [file]` accepts one positional arg — an
            // alternate utmp file. Honored via `utmpxname(3)` on
            // platforms that have it; silently no-op'd on those
            // that don't (macOS doesn't ship utmpxname).
            if !arg.starts_with('-') {
                #[cfg(target_os = "linux")]
                {
                    let cpath = std::ffi::CString::new(arg.as_bytes()).ok();
                    if let Some(c) = cpath {
                        unsafe {
                            // utmpxname(path): set the file getutxent reads from.
                            extern "C" {
                                fn utmpxname(file: *const libc::c_char) -> libc::c_int;
                            }
                            utmpxname(c.as_ptr());
                        }
                    }
                }
            }
        }
        // Walk utmp via getutxent(3) — POSIX-portable on Linux/BSD/
        // macOS. Filter `ut_type == USER_PROCESS` (zsh-level
        // `who(1)` does the same). On systems where utmpx isn't
        // populated (containers, ephemeral hosts), fall back to
        // single-user output via `$USER` / `geteuid()`.
        let mut users: Vec<String> = Vec::new();
        unsafe {
            libc::setutxent();
            loop {
                let ent = libc::getutxent();
                if ent.is_null() {
                    break;
                }
                if (*ent).ut_type == libc::USER_PROCESS {
                    // ut_user is a fixed-size i8 array; convert
                    // until first NUL.
                    let raw = &(*ent).ut_user;
                    let bytes: Vec<u8> = raw
                        .iter()
                        .take_while(|&&c| c != 0)
                        .map(|&c| c as u8)
                        .collect();
                    if !bytes.is_empty() {
                        if let Ok(s) = std::str::from_utf8(&bytes) {
                            users.push(s.to_string());
                        }
                    }
                }
            }
            libc::endutxent();
        }
        users.sort();
        if users.is_empty() {
            // Fallback: utmp empty (containers / no logged-in
            // sessions). Print current user — matches `who am i`
            // when only the calling shell is "logged in".
            let name = match std::env::var("USER") {
                Ok(u) if !u.is_empty() => u,
                _ => {
                    let euid = unsafe { libc::geteuid() };
                    let pw = unsafe { libc::getpwuid(euid) };
                    if !pw.is_null() {
                        let n = unsafe { CStr::from_ptr((*pw).pw_name) };
                        n.to_string_lossy().into_owned()
                    } else {
                        return 0;
                    }
                }
            };
            println!("{}", name);
        } else {
            println!("{}", users.join(" "));
        }
        0
    }

    /// tput — terminfo capability query (minimal subset).
    /// Common subset of ncurses tput(1):
    ///   tput cols / lines      → terminal width / height
    ///   tput colors            → terminal color count
    ///   tput clear / cl        → clear screen
    ///   tput cup R C           → cursor to (row, col) (0-based)
    ///   tput sgr0 / op         → reset attributes / colors
    ///   tput bold / smso / rmso / smul / rmul / rev / blink
    ///                          → text attributes
    ///   tput setaf N / setab N → fg/bg color (8/16 colors)
    /// Many other terminfo capabilities aren't yet wired; unknown
    /// capabilities fall through to echotc's two-letter mapping or
    /// silently exit 1 (tput's standard error code).
    pub(crate) fn builtin_tput(&self, args: &[String]) -> i32 {
        if args.is_empty() {
            eprintln!("tput: missing capname");
            return 2;
        }
        let mut iter = args.iter().peekable();
        let mut stdin_mode = false;
        while let Some(arg) = iter.peek() {
            match arg.as_str() {
                "-T" => {
                    iter.next();
                    // The TERM override is consumed; the handlers
                    // below read TERM via `$TERM` env var anyway,
                    // so applying this would require temporarily
                    // setenv-ing TERM for the cap evaluation. Honest
                    // gap noted; most real scripts don't pass -T.
                    iter.next();
                }
                s if s.starts_with("-T") && s.len() > 2 => {
                    iter.next();
                }
                "-S" => {
                    iter.next();
                    stdin_mode = true;
                }
                "-V" | "--version" => {
                    println!("tput (zshrs) {}", env!("CARGO_PKG_VERSION"));
                    return 0;
                }
                "-h" | "--help" => {
                    println!("Usage: tput [-T TERM] [-S] CAPNAME [PARAMS...]");
                    return 0;
                }
                _ => break,
            }
        }

        // -S stdin mode: each line is `capname [params...]`. Process
        // every line through the same cap handler used below. Blank
        // lines are skipped; final exit status is non-zero if ANY
        // line failed (matches ncurses tput).
        if stdin_mode {
            use std::io::BufRead;
            let stdin = std::io::stdin();
            let mut status = 0;
            for line_res in stdin.lock().lines() {
                let line = match line_res {
                    Ok(l) => l,
                    Err(_) => break,
                };
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let mut parts = trimmed.split_whitespace();
                let cap = match parts.next() {
                    Some(c) => c,
                    None => continue,
                };
                let rest: Vec<&str> = parts.collect();
                let s = tput_emit_cap(cap, &rest);
                if s != 0 {
                    status = s;
                }
            }
            return status;
        }

        let cap = match iter.next() {
            Some(c) => c.as_str(),
            None => {
                eprintln!("tput: missing capname");
                return 2;
            }
        };
        let rest: Vec<&str> = iter.map(|s| s.as_str()).collect();
        tput_emit_cap(cap, &rest)
    }
}

/// Emit the terminal-control sequence for one capability name. Used
/// by both the direct `tput CAP` path and the `-S` stdin loop. Mirrors
/// the cap set zsh's prompt-theme + zinit + p10k routines invoke; not
/// the full terminfo database (that would require linking ncurses).
/// Returns coreutils-tput exit status: 0=ok, 1=unknown bool-cap,
/// 2=unknown string-cap. We collapse both unknowns to 1.
fn tput_emit_cap(cap: &str, rest: &[&str]) -> i32 {
    match cap {
        "cols" | "co" => {
            let cols: i32 = std::env::var("COLUMNS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(80);
            println!("{}", cols);
            0
        }
        "lines" | "li" => {
            let lines: i32 = std::env::var("LINES")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(24);
            println!("{}", lines);
            0
        }
        "colors" | "Co" => {
            // Most modern terminals are 256 or truecolor; default
            // to 256 since that's what TERM=xterm-256color reports.
            let term = std::env::var("TERM").unwrap_or_default();
            let n = if term.contains("256") || term.contains("direct") || term.contains("truecolor")
            {
                256
            } else {
                8
            };
            println!("{}", n);
            0
        }
        "clear" | "cl" => {
            print!("\x1b[H\x1b[2J");
            0
        }
        "cup" => {
            if rest.len() < 2 {
                return 2;
            }
            if let (Ok(r), Ok(c)) = (rest[0].parse::<u32>(), rest[1].parse::<u32>()) {
                print!("\x1b[{};{}H", r + 1, c + 1);
            }
            0
        }
        "sgr0" | "me" | "op" => {
            print!("\x1b[0m");
            0
        }
        "bold" | "md" => {
            print!("\x1b[1m");
            0
        }
        "smso" | "so" | "rev" | "mr" => {
            print!("\x1b[7m");
            0
        }
        "rmso" | "se" => {
            print!("\x1b[27m");
            0
        }
        "smul" | "us" => {
            print!("\x1b[4m");
            0
        }
        "rmul" | "ue" => {
            print!("\x1b[24m");
            0
        }
        "blink" | "mb" => {
            print!("\x1b[5m");
            0
        }
        "setaf" | "AF" => {
            if let Some(n) = rest.first().and_then(|s| s.parse::<i32>().ok()) {
                print!("\x1b[{}m", 30 + n);
            }
            0
        }
        "setab" | "AB" => {
            if let Some(n) = rest.first().and_then(|s| s.parse::<i32>().ok()) {
                print!("\x1b[{}m", 40 + n);
            }
            0
        }
        "civis" | "vi" => {
            print!("\x1b[?25l");
            0
        }
        "cnorm" | "ve" => {
            print!("\x1b[?25h");
            0
        }
        _ => {
            // Unknown capability — exit 1 silently per tput
            // convention. Don't emit error for boolean-cap probes.
            1
        }
    }
}

impl ShellExecutor {
    /// zbuild --in PATHS... --out OUT — bake one or more shell
    /// scripts into a copy of the running zshrs binary in input
    /// order, producing a self-contained AOT executable.
    ///
    ///   zbuild --in *.zsh --out app           # glob expansion
    ///   zbuild --in lib1.zsh lib2.zsh main.zsh --out app
    ///   ./app                                  # runs all three
    ///                                          # sequentially under
    ///                                          # one ShellExecutor
    ///
    /// zsh has no project concept, so there's no manifest, no entry
    /// point convention, no library directory walker. Order is
    /// exactly the order of paths given to `--in` (which honors
    /// shell glob expansion — \`*.zsh\` expands sorted by default).
    ///
    /// `--in` accepts ONE OR MORE paths until the next flag-style
    /// token (anything starting with `-`). This makes glob-driven
    /// invocations like `--in *.zsh` work naturally — every path
    /// the glob expanded to lands as another input file.
    ///
    /// Flags:
    ///   --in PATHS...  / -i PATHS...   script sources (1+, required)
    ///   --out PATH     / -o PATH       output binary (required)
    ///   --help / -h                    print usage
    pub(crate) fn builtin_zbuild(&self, args: &[String]) -> i32 {
        let mut inputs: Vec<std::path::PathBuf> = Vec::new();
        let mut output: Option<String> = None;
        let mut native = false;
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            match arg.as_str() {
                "--native" | "-n" => {
                    native = true;
                }
                "--in" | "-i" | "--input" => {
                    // Consume every non-flag token following --in
                    // until we hit the next `-`-prefixed token (or
                    // end of args). This makes `--in *.zsh` pick up
                    // all paths the glob produced.
                    i += 1;
                    let start = i;
                    while i < args.len() && !args[i].starts_with('-') {
                        inputs.push(std::path::PathBuf::from(&args[i]));
                        i += 1;
                    }
                    if i == start {
                        eprintln!("zshrs:zbuild:1: --in requires at least one path");
                        return 1;
                    }
                    continue;
                }
                s if s.starts_with("--in=") => {
                    inputs.push(std::path::PathBuf::from(&s[5..]));
                }
                s if s.starts_with("--input=") => {
                    inputs.push(std::path::PathBuf::from(&s[8..]));
                }
                "--out" | "-o" | "--output" => {
                    i += 1;
                    if i >= args.len() {
                        eprintln!("zshrs:zbuild:1: --out requires a path");
                        return 1;
                    }
                    output = Some(args[i].clone());
                }
                s if s.starts_with("--out=") => output = Some(s[6..].to_string()),
                s if s.starts_with("--output=") => output = Some(s[9..].to_string()),
                "--help" | "-h" => {
                    println!("Usage: zbuild --in PATHS... --out OUT");
                    println!();
                    println!("Bake one or more shell scripts into an AOT-compiled");
                    println!("standalone executable. Files run sequentially in input");
                    println!("order under a single ShellExecutor (globals/functions");
                    println!("from earlier files visible to later ones).");
                    println!();
                    println!("Examples:");
                    println!("  zbuild --in *.zsh --out app");
                    println!("  zbuild --in lib.zsh main.zsh --out app");
                    println!();
                    println!("Options:");
                    println!("  --in / -i PATHS...  script sources (1+, required)");
                    println!("  --out / -o PATH     output binary (required)");
                    println!("  --native / -n       AOT-compile to native machine code");
                    println!("                      (Cranelift object linked standalone)");
                    return 0;
                }
                _ => {
                    eprintln!("zshrs:zbuild:1: unrecognized argument: {}", arg);
                    return 1;
                }
            }
            i += 1;
        }
        if inputs.is_empty() {
            eprintln!("zshrs:zbuild:1: at least one --in PATH required");
            return 1;
        }
        let out_path = match output {
            Some(p) => p,
            None => {
                eprintln!("zshrs:zbuild:1: --out PATH required");
                return 1;
            }
        };
        if native {
            return match crate::aot::build_native(&inputs, std::path::Path::new(&out_path)) {
                Ok(p) => {
                    eprintln!("zbuild: wrote native binary {}", p.display());
                    0
                }
                Err(e) => {
                    eprintln!("zshrs:zbuild:1: {}", e);
                    1
                }
            };
        }
        match crate::aot::build(&inputs, std::path::Path::new(&out_path)) {
            Ok(p) => {
                eprintln!(
                    "zbuild: wrote {} ({} file{} embedded)",
                    p.display(),
                    inputs.len(),
                    if inputs.len() == 1 { "" } else { "s" }
                );
                0
            }
            Err(e) => {
                eprintln!("zshrs:zbuild:1: {}", e);
                1
            }
        }
    }

    /// logname — print login name. Coreutils logname(1) / POSIX.
    /// Calls getlogin(3) which reads from utmp. Falls back to
    /// \$LOGNAME if getlogin fails.
    pub(crate) fn builtin_logname(&self, args: &[String]) -> i32 {
        for arg in args {
            if arg.starts_with('-') && arg.len() > 1 && arg != "--" {
                eprintln!("logname: unrecognized option: '{}'", arg);
                return 1;
            }
            if !arg.starts_with('-') {
                eprintln!("logname: extra operand '{}'", arg);
                return 1;
            }
        }
        let p = unsafe { libc::getlogin() };
        if !p.is_null() {
            let name = unsafe { std::ffi::CStr::from_ptr(p) };
            println!("{}", name.to_string_lossy());
            return 0;
        }
        // Fallback for environments without utmp (e.g. CI containers).
        match std::env::var("LOGNAME") {
            Ok(v) if !v.is_empty() => {
                println!("{}", v);
                0
            }
            _ => {
                eprintln!("logname: no login name");
                1
            }
        }
    }

    /// nice [-n N] [-N N] [COMMAND...] — adjust niceness of the
    /// shell process (when no COMMAND given) or report current
    /// niceness. Coreutils nice(1) / POSIX nice(1). Without args,
    /// print the current niceness (like \`nice\` with no args). With
    /// just -n N, set the niceness for the SHELL ITSELF — not for
    /// a subsequently-exec'd command. Setting niceness for a child
    /// command requires fork-exec which the in-process model
    /// can't do without breaking subsequent commands; for that case
    /// callers should use /usr/bin/nice via PATH (zshrs's command
    /// dispatch will fall through to the external).
    pub(crate) fn builtin_nice(&self, args: &[String]) -> i32 {
        let mut adjust: Option<i32> = None;
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-n" | "--adjustment" => {
                    if let Some(s) = iter.next() {
                        match s.parse::<i32>() {
                            Ok(n) => adjust = Some(n),
                            Err(_) => {
                                eprintln!("nice: '{}': invalid adjustment", s);
                                return 1;
                            }
                        }
                    } else {
                        eprintln!("nice: option requires an argument: -n");
                        return 1;
                    }
                }
                s if s.starts_with("-n") && s.len() > 2 => match s[2..].parse::<i32>() {
                    Ok(n) => adjust = Some(n),
                    Err(_) => {
                        eprintln!("nice: '{}': invalid adjustment", &s[2..]);
                        return 1;
                    }
                },
                "--" => break,
                s if !s.starts_with('-') => {
                    // Bare COMMAND given. We can't fork-exec from
                    // the in-process model. Tell the user to use
                    // the external /usr/bin/nice for that case.
                    eprintln!(
                        "nice: command-launching mode unavailable in-process; use /usr/bin/nice"
                    );
                    return 1;
                }
                s => {
                    eprintln!("nice: unrecognized option: '{}'", s);
                    return 1;
                }
            }
        }
        let _ = iter;
        if let Some(adj) = adjust {
            // Apply to the shell process itself.
            let cur = unsafe { libc::getpriority(libc::PRIO_PROCESS, 0) };
            let target = (cur + adj).clamp(-20, 19);
            if unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, target) } != 0 {
                let err = std::io::Error::last_os_error();
                eprintln!("nice: cannot set niceness: {}", err);
                return 1;
            }
            return 0;
        }
        // No -n: print current niceness.
        let cur = unsafe { libc::getpriority(libc::PRIO_PROCESS, 0) };
        println!("{}", cur);
        0
    }

    /// arch — print machine architecture name. Coreutils arch(1)
    /// (a synonym for `uname -m` on most systems). Useful in shell
    /// scripts that need a quick `[[ $(arch) == arm64 ]]` check.
    pub(crate) fn builtin_arch(&self, args: &[String]) -> i32 {
        for arg in args {
            if arg.starts_with('-') && arg.len() > 1 {
                eprintln!("arch: unrecognized option: '{}'", arg);
                return 1;
            }
        }
        let mut uts: libc::utsname = unsafe { std::mem::zeroed() };
        if unsafe { libc::uname(&mut uts) } != 0 {
            eprintln!("arch: uname() failed");
            return 1;
        }
        // machine[] is a fixed-size i8 (or u8) array; convert to str.
        let machine: Vec<u8> = uts
            .machine
            .iter()
            .take_while(|&&c| c != 0)
            .map(|&c| c as u8)
            .collect();
        let machine_str = String::from_utf8_lossy(&machine);
        println!("{}", machine_str);
        0
    }

    /// dircolors [-bcp] [FILE] — emit shell commands to set
    /// LS_COLORS. Coreutils dircolors(1). Without args, emits the
    /// default ls color database. -b for Bourne (export VAR=val),
    /// -c for csh (setenv VAR val), -p prints the database.
    /// We hard-code coreutils' compiled-in default since shipping
    /// the full /etc/DIR_COLORS database file isn't feasible here.
    pub(crate) fn builtin_dircolors(&self, args: &[String]) -> i32 {
        let mut bourne = true;
        let mut csh = false;
        let mut print_database = false;
        let mut file: Option<&str> = None;
        for arg in args {
            match arg.as_str() {
                "-b" | "--sh" | "--bourne-shell" => {
                    bourne = true;
                    csh = false;
                }
                "-c" | "--csh" | "--c-shell" => {
                    bourne = false;
                    csh = true;
                }
                "-p" | "--print-database" => print_database = true,
                "--" => {}
                s if !s.starts_with('-') => file = Some(s),
                s => {
                    eprintln!("dircolors: unrecognized option: '{}'", s);
                    return 1;
                }
            }
        }
        if let Some(f) = file {
            // Reading a custom database file isn't implemented; emit
            // the default but tell the user we ignored the file.
            eprintln!(
                "dircolors: using built-in defaults (custom file ignored: '{}')",
                f
            );
        }
        // Coreutils' default LS_COLORS, lightly trimmed. Captured
        // from `dircolors --print-database` of GNU coreutils 9.x.
        // Hardcoding here keeps the builtin self-contained.
        let default_ls_colors = concat!(
            "rs=0:di=01;34:ln=01;36:mh=00:pi=40;33:so=01;35:do=01;35:bd=40;33;01:",
            "cd=40;33;01:or=40;31;01:mi=00:su=37;41:sg=30;43:ca=00:tw=30;42:",
            "ow=34;42:st=37;44:ex=01;32:*.tar=01;31:*.tgz=01;31:*.zip=01;31:",
            "*.gz=01;31:*.bz2=01;31:*.xz=01;31:*.7z=01;31:*.rar=01;31:",
            "*.jpg=01;35:*.jpeg=01;35:*.png=01;35:*.gif=01;35:*.bmp=01;35:",
            "*.tiff=01;35:*.svg=01;35:*.mp3=00;36:*.wav=00;36:*.flac=00;36:",
            "*.mp4=01;35:*.mkv=01;35:*.avi=01;35:*.mov=01;35:"
        );
        if print_database {
            // Emit one entry per line (coreutils format).
            for entry in default_ls_colors.split(':') {
                if entry.is_empty() {
                    continue;
                }
                if let Some((k, v)) = entry.split_once('=') {
                    println!("{} {}", k, v);
                }
            }
            return 0;
        }
        if csh {
            println!("setenv LS_COLORS '{}';", default_ls_colors);
        } else {
            let _ = bourne;
            println!("LS_COLORS='{}';", default_ls_colors);
            println!("export LS_COLORS");
        }
        0
    }

    /// link FILE1 FILE2 — call link(2) directly to create a hard
    /// link from FILE1 to FILE2. POSIX link(1) / coreutils link(1).
    /// Unlike `ln`, link takes EXACTLY two args and rejects flags
    /// (POSIX requirement).
    pub(crate) fn builtin_link(&self, args: &[String]) -> i32 {
        if args.len() != 2 {
            eprintln!("link: missing operand");
            return 1;
        }
        for a in args {
            if a.starts_with('-') && a.len() > 1 {
                // POSIX link(1) accepts no options.
                eprintln!("link: unrecognized option: '{}'", a);
                return 1;
            }
        }
        if let Err(e) = std::fs::hard_link(&args[0], &args[1]) {
            eprintln!("link: cannot link '{}' to '{}': {}", args[0], args[1], e);
            return 1;
        }
        0
    }

    /// unlink FILE — call unlink(2) directly to remove a single
    /// file. POSIX unlink(1) / coreutils unlink(1). Strict: takes
    /// exactly ONE arg, no flags, no recursion. Cannot remove
    /// directories (errors if FILE is a directory).
    pub(crate) fn builtin_unlink(&self, args: &[String]) -> i32 {
        if args.len() != 1 {
            eprintln!("unlink: missing operand");
            return 1;
        }
        if args[0].starts_with('-') && args[0].len() > 1 {
            eprintln!("unlink: unrecognized option: '{}'", args[0]);
            return 1;
        }
        if let Err(e) = std::fs::remove_file(&args[0]) {
            eprintln!("unlink: cannot unlink '{}': {}", args[0], e);
            return 1;
        }
        0
    }

    /// mkfifo [-m MODE] FILE... — create named pipes (FIFOs).
    /// Coreutils mkfifo(1) / POSIX. -m sets the mode (default 0666
    /// minus umask). Each FIFO is created independently; failures
    /// are reported per-file and the others continue.
    pub(crate) fn builtin_mkfifo(&self, args: &[String]) -> i32 {
        let mut mode: libc::mode_t = 0o666;
        let mut files: Vec<&str> = Vec::new();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-m" | "--mode" => {
                    if let Some(m) = iter.next() {
                        match libc::mode_t::from_str_radix(m, 8) {
                            Ok(v) => mode = v,
                            Err(_) => {
                                eprintln!("mkfifo: invalid mode: '{}'", m);
                                return 1;
                            }
                        }
                    } else {
                        eprintln!("mkfifo: option requires an argument -- '-m'");
                        return 1;
                    }
                }
                s if s.starts_with("--mode=") => match libc::mode_t::from_str_radix(&s[7..], 8) {
                    Ok(v) => mode = v,
                    Err(_) => {
                        eprintln!("mkfifo: invalid mode: '{}'", &s[7..]);
                        return 1;
                    }
                },
                s if !s.starts_with('-') => files.push(s),
                "-" => files.push("-"),
                _ => {
                    eprintln!("mkfifo: unrecognized option: '{}'", arg);
                    return 1;
                }
            }
        }
        if files.is_empty() {
            eprintln!("mkfifo: missing operand");
            return 1;
        }
        let mut status = 0;
        for f in files {
            let cpath = match CString::new(f) {
                Ok(c) => c,
                Err(_) => {
                    eprintln!("mkfifo: cannot create fifo '{}': invalid path", f);
                    status = 1;
                    continue;
                }
            };
            if unsafe { libc::mkfifo(cpath.as_ptr(), mode) } != 0 {
                let err = std::io::Error::last_os_error();
                eprintln!("mkfifo: cannot create fifo '{}': {}", f, err);
                status = 1;
            }
        }
        status
    }

    /// tsort [FILE] — topological sort. Coreutils tsort(1) / POSIX.
    /// Input is whitespace-separated pairs `A B` meaning "A precedes
    /// B"; tsort prints a partial order (Kahn's algorithm). Cycles
    /// are reported on stderr (one cycle node per line) and the
    /// program continues with that node treated as a leaf. Reads
    /// stdin when no file is given or `-`.
    pub(crate) fn builtin_tsort(&self, args: &[String]) -> i32 {
        let file: Option<&str> = args
            .iter()
            .find(|a| !a.starts_with('-') || a.as_str() == "-")
            .map(|s| s.as_str());
        let reader: Box<dyn BufRead> = match file {
            Some(f) if f != "-" => match std::fs::File::open(f) {
                Ok(fh) => Box::new(BufReader::new(fh)),
                Err(e) => {
                    eprintln!("tsort: {}: {}", f, e);
                    return 1;
                }
            },
            _ => Box::new(BufReader::new(std::io::stdin())),
        };
        let mut tokens: Vec<String> = Vec::new();
        for line in reader.lines().map_while(Result::ok) {
            for tok in line.split_whitespace() {
                tokens.push(tok.to_string());
            }
        }
        if !tokens.len().is_multiple_of(2) {
            // POSIX tsort: odd token count is an error per coreutils.
            eprintln!("tsort: input contains an odd number of tokens");
            return 1;
        }
        // BTreeMap for deterministic listing order — coreutils
        // visits in input-encounter order, which BTreeMap+iteration
        // approximates with sorted order. Tests on typical Makefile
        // dep-order input produce identical-shape output.
        let mut succ: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut indeg: BTreeMap<String, usize> = BTreeMap::new();
        let mut nodes: BTreeMap<String, ()> = BTreeMap::new();
        let mut k = 0;
        while k + 1 < tokens.len() {
            let a = tokens[k].clone();
            let b = tokens[k + 1].clone();
            nodes.insert(a.clone(), ());
            nodes.insert(b.clone(), ());
            indeg.entry(a.clone()).or_insert(0);
            if a != b {
                succ.entry(a.clone()).or_default().push(b.clone());
                *indeg.entry(b).or_insert(0) += 1;
            } else {
                indeg.entry(b).or_insert(0);
            }
            k += 2;
        }
        let mut ready: Vec<String> = nodes
            .keys()
            .filter(|n| indeg.get(*n).copied().unwrap_or(0) == 0)
            .cloned()
            .collect();
        let mut emitted: Vec<String> = Vec::new();
        while !ready.is_empty() {
            ready.sort();
            let n = ready.remove(0);
            println!("{}", n);
            emitted.push(n.clone());
            if let Some(succs) = succ.remove(&n) {
                for s in succs {
                    if let Some(d) = indeg.get_mut(&s) {
                        if *d > 0 {
                            *d -= 1;
                        }
                        if *d == 0 {
                            ready.push(s);
                        }
                    }
                }
            }
        }
        if emitted.len() != nodes.len() {
            // Cycle detected — print the remaining unprocessed nodes
            // to stderr per coreutils.
            eprintln!("tsort: input contains a loop:");
            for (n, d) in &indeg {
                if *d > 0 {
                    eprintln!("tsort: {}", n);
                }
            }
            return 1;
        }
        0
    }

    /// sum [-rs] [FILE...] — BSD or SysV checksum.
    /// `-r`: BSD 16-bit rotating checksum (default per POSIX).
    /// `-s`: SysV checksum (sum-of-bytes mod 65535, then folded).
    /// Output: `<sum> <512-byte-blocks> [name]` (BSD) or
    ///         `<sum> <kbytes> [name]` (SysV).
    pub(crate) fn builtin_sum(&self, args: &[String]) -> i32 {
        let mut sysv = false;
        let mut files: Vec<&str> = Vec::new();
        for arg in args {
            match arg.as_str() {
                "-r" => sysv = false,
                "-s" | "--sysv" => sysv = true,
                "--bsd" => sysv = false,
                s if !s.starts_with('-') || s == "-" => files.push(s),
                _ => {
                    eprintln!("zshrs:sum:1: unknown option: {}", arg);
                    return 1;
                }
            }
        }
        let targets: Vec<&str> = if files.is_empty() { vec!["-"] } else { files };
        let bsd_sum = |bytes: &[u8]| -> u32 {
            let mut s: u32 = 0;
            for &b in bytes {
                // BSD: rotate right one bit then add — 16-bit value.
                s = (s >> 1) | ((s & 1) << 15);
                s = (s + b as u32) & 0xffff;
            }
            s
        };
        let sysv_sum = |bytes: &[u8]| -> u32 {
            let mut s: u32 = 0;
            for &b in bytes {
                s = s.wrapping_add(b as u32);
            }
            // Two-stage fold: r = (s & 0xffff) + (s >> 16);
            // then r = (r & 0xffff) + (r >> 16). Per coreutils.
            let r = (s & 0xffff) + (s >> 16);
            (r & 0xffff) + (r >> 16)
        };
        let mut status = 0;
        for path in targets {
            let mut buf = Vec::new();
            let read_res = if path == "-" {
                std::io::stdin().read_to_end(&mut buf)
            } else {
                match std::fs::File::open(path) {
                    Ok(mut f) => f.read_to_end(&mut buf),
                    Err(e) => {
                        eprintln!("sum: {}: {}", path, e);
                        status = 1;
                        continue;
                    }
                }
            };
            if let Err(e) = read_res {
                eprintln!("sum: {}: {}", path, e);
                status = 1;
                continue;
            }
            if sysv {
                let s = sysv_sum(&buf);
                let kbytes = buf.len().div_ceil(1024);
                if path == "-" {
                    println!("{} {}", s, kbytes);
                } else {
                    println!("{} {} {}", s, kbytes, path);
                }
            } else {
                let s = bsd_sum(&buf);
                let blocks = buf.len().div_ceil(512);
                if path == "-" {
                    println!("{:05} {:5}", s, blocks);
                } else {
                    println!("{:05} {:5} {}", s, blocks, path);
                }
            }
        }
        status
    }

    /// cksum [FILE...] — POSIX CRC-32 + byte-count + filename.
    /// Output: `<crc> <bytes> <name>`. With no files or `-` reads
    /// stdin (filename column omitted in that case, per coreutils).
    /// Polynomial: 0x04C11DB7, init 0, length appended (POSIX).
    pub(crate) fn builtin_cksum(&self, args: &[String]) -> i32 {
        // POSIX cksum table, generated for polynomial 0x04C11DB7 with
        // bits processed MSB-first. Built once at runtime per call;
        // a const table would be ~1KB but the runtime cost of building
        // is microseconds and avoids the const-array boilerplate.
        let mut table = [0u32; 256];
        for (i, slot) in table.iter_mut().enumerate() {
            let mut c = (i as u32) << 24;
            for _ in 0..8 {
                c = if c & 0x8000_0000 != 0 {
                    (c << 1) ^ 0x04C11DB7
                } else {
                    c << 1
                };
            }
            *slot = c;
        }
        let crc_bytes = |bytes: &[u8]| -> (u32, u64) {
            let mut crc: u32 = 0;
            let mut len: u64 = 0;
            for &b in bytes {
                crc = (crc << 8) ^ table[((crc >> 24) ^ b as u32) as usize & 0xff];
                len += 1;
            }
            // POSIX cksum appends the length as little-endian-by-byte
            // until length consumed.
            let mut n = len;
            while n != 0 {
                crc = (crc << 8) ^ table[((crc >> 24) ^ (n as u32 & 0xff)) as usize & 0xff];
                n >>= 8;
            }
            (!crc, len)
        };
        let files: Vec<&str> = args
            .iter()
            .filter(|a| !a.starts_with('-') || *a == "-")
            .map(|s| s.as_str())
            .collect();
        let targets: Vec<&str> = if files.is_empty() { vec!["-"] } else { files };
        let mut status = 0;
        for path in targets {
            let mut buf = Vec::new();
            let read_res = if path == "-" {
                std::io::stdin().read_to_end(&mut buf)
            } else {
                match std::fs::File::open(path) {
                    Ok(mut f) => f.read_to_end(&mut buf),
                    Err(e) => {
                        eprintln!("cksum: {}: {}", path, e);
                        status = 1;
                        continue;
                    }
                }
            };
            if let Err(e) = read_res {
                eprintln!("cksum: {}: {}", path, e);
                status = 1;
                continue;
            }
            let (crc, len) = crc_bytes(&buf);
            if path == "-" {
                println!("{} {}", crc, len);
            } else {
                println!("{} {} {}", crc, len, path);
            }
        }
        status
    }

    /// factor N... — print prime factorization of each integer arg.
    /// Coreutils factor(1). Format: `N: p1 p2 p3 ...`. Reads stdin
    /// if no args. Negative numbers and zero are rejected.
    pub(crate) fn builtin_factor(&self, args: &[String]) -> i32 {
        let factor_line = |line: &str| {
            for tok in line.split_whitespace() {
                let n: u64 = match tok.parse() {
                    Ok(v) => v,
                    Err(_) => {
                        eprintln!("factor: '{}' is not a valid positive integer", tok);
                        continue;
                    }
                };
                let mut x = n;
                let mut factors: Vec<u64> = Vec::new();
                if x < 2 {
                    // 0 and 1 have no prime factorization. coreutils
                    // emits `N:` with empty list. Match that.
                    println!("{}:", n);
                    continue;
                }
                while x.is_multiple_of(2) {
                    factors.push(2);
                    x /= 2;
                }
                let mut p: u64 = 3;
                while p.saturating_mul(p) <= x {
                    while x.is_multiple_of(p) {
                        factors.push(p);
                        x /= p;
                    }
                    p += 2;
                }
                if x > 1 {
                    factors.push(x);
                }
                let parts: Vec<String> = factors.iter().map(|p| p.to_string()).collect();
                println!("{}: {}", n, parts.join(" "));
            }
        };
        let nums: Vec<&str> = args
            .iter()
            .filter(|a| !a.starts_with('-'))
            .map(|s| s.as_str())
            .collect();
        if nums.is_empty() {
            let stdin = std::io::stdin();
            for line in stdin.lock().lines().map_while(Result::ok) {
                factor_line(&line);
            }
        } else {
            for tok in nums {
                factor_line(tok);
            }
        }
        0
    }

    /// comm [-123] FILE1 FILE2 — line-by-line comparison of two
    /// sorted files. Coreutils comm(1) / POSIX. Three columns:
    /// (1) unique to FILE1, (2) unique to FILE2, (3) common.
    /// Flags `-1`/`-2`/`-3` suppress the respective column. Either
    /// file may be `-` for stdin. Files MUST be sorted in the same
    /// collation; comm performs a streaming merge-compare and is
    /// undefined-behavior on unsorted input (matches coreutils).
    pub(crate) fn builtin_comm(&self, args: &[String]) -> i32 {
        let mut suppress1 = false;
        let mut suppress2 = false;
        let mut suppress3 = false;
        let mut files: Vec<&str> = Vec::new();
        for arg in args {
            match arg.as_str() {
                "-1" => suppress1 = true,
                "-2" => suppress2 = true,
                "-3" => suppress3 = true,
                "-12" | "-21" => {
                    suppress1 = true;
                    suppress2 = true;
                }
                "-13" | "-31" => {
                    suppress1 = true;
                    suppress3 = true;
                }
                "-23" | "-32" => {
                    suppress2 = true;
                    suppress3 = true;
                }
                "-123" | "-132" | "-213" | "-231" | "-312" | "-321" => {
                    suppress1 = true;
                    suppress2 = true;
                    suppress3 = true;
                }
                "--help" => {
                    println!("Usage: comm [-123] FILE1 FILE2");
                    return 0;
                }
                s if !s.starts_with('-') || s == "-" => files.push(s),
                _ => {
                    eprintln!("zshrs:comm:1: unknown option: {}", arg);
                    return 1;
                }
            }
        }
        if files.len() != 2 {
            eprintln!("zshrs:comm:1: expected exactly 2 file arguments");
            return 1;
        }
        let read_lines = |path: &str| -> std::io::Result<Vec<String>> {
            let reader: Box<dyn BufRead> = if path == "-" {
                Box::new(BufReader::new(std::io::stdin()))
            } else {
                Box::new(BufReader::new(std::fs::File::open(path)?))
            };
            reader.lines().collect()
        };
        let lines1 = match read_lines(files[0]) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("zshrs:comm:1: {}: {}", files[0], e);
                return 1;
            }
        };
        let lines2 = match read_lines(files[1]) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("zshrs:comm:1: {}: {}", files[1], e);
                return 1;
            }
        };
        let mut i = 0usize;
        let mut j = 0usize;
        let emit_col1 = |s: &str| {
            if !suppress1 {
                println!("{}", s);
            }
        };
        let emit_col2 = |s: &str| {
            if !suppress2 {
                let prefix = if suppress1 { "" } else { "\t" };
                println!("{}{}", prefix, s);
            }
        };
        let emit_col3 = |s: &str| {
            if !suppress3 {
                let prefix = match (suppress1, suppress2) {
                    (true, true) => "",
                    (false, true) | (true, false) => "\t",
                    (false, false) => "\t\t",
                };
                println!("{}{}", prefix, s);
            }
        };
        while i < lines1.len() && j < lines2.len() {
            match lines1[i].cmp(&lines2[j]) {
                std::cmp::Ordering::Less => {
                    emit_col1(&lines1[i]);
                    i += 1;
                }
                std::cmp::Ordering::Greater => {
                    emit_col2(&lines2[j]);
                    j += 1;
                }
                std::cmp::Ordering::Equal => {
                    emit_col3(&lines1[i]);
                    i += 1;
                    j += 1;
                }
            }
        }
        while i < lines1.len() {
            emit_col1(&lines1[i]);
            i += 1;
        }
        while j < lines2.len() {
            emit_col2(&lines2[j]);
            j += 1;
        }
        0
    }

    /// tac [FILE...] — concatenate files, reverse line order.
    /// coreutils tac(1).
    pub(crate) fn builtin_tac(&self, args: &[String]) -> i32 {
        // tac in coreutils accepts -b (before) / -r (regex separator)
        // / -s (separator). Most usage is positional-only. Validate
        // unknown flags rather than silent-drop.
        let mut files: Vec<&str> = Vec::new();
        let mut iter = args.iter();
        while let Some(a) = iter.next() {
            let s: &str = a.as_str();
            match s {
                "-" => files.push("-"),
                "-b" | "--before" | "-r" | "--regex" => {} // accepted, no-op
                "-s" | "--separator" => {
                    iter.next(); // consume the separator arg
                }
                "--" => {
                    for rest in iter.by_ref() {
                        files.push(rest);
                    }
                    break;
                }
                x if !x.starts_with('-') => files.push(x),
                _ => {
                    eprintln!("tac: unrecognized option: '{}'", s);
                    return 1;
                }
            }
        }
        let targets: Vec<&str> = if files.is_empty() { vec!["-"] } else { files };
        let mut all: Vec<String> = Vec::new();
        for file in targets {
            let reader: Box<dyn BufRead> = if file == "-" {
                Box::new(BufReader::new(std::io::stdin()))
            } else {
                match std::fs::File::open(file) {
                    Ok(f) => Box::new(BufReader::new(f)),
                    Err(e) => {
                        eprintln!("tac: {}: {}", file, e);
                        return 1;
                    }
                }
            };
            for line in reader.lines().map_while(Result::ok) {
                all.push(line);
            }
        }
        for line in all.iter().rev() {
            println!("{}", line);
        }
        0
    }

    /// expand [-t TAB] [FILE...] — convert tabs to spaces.
    /// coreutils expand(1).
    pub(crate) fn builtin_expand(&self, args: &[String]) -> i32 {
        // Default tab stop 8.
        let mut tabs: Vec<usize> = vec![8];
        let mut files: Vec<&str> = Vec::new();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-t" | "--tabs" => {
                    if let Some(s) = iter.next() {
                        tabs = s.split([',', ' ']).filter_map(|x| x.parse().ok()).collect();
                        if tabs.is_empty() {
                            tabs = vec![8];
                        }
                    }
                }
                s if s.starts_with("-t") && s.len() > 2 => {
                    tabs = s[2..]
                        .split([',', ' '])
                        .filter_map(|x| x.parse().ok())
                        .collect();
                    if tabs.is_empty() {
                        tabs = vec![8];
                    }
                }
                "-i" | "--initial" => {} // accepted: only-leading-tabs
                "-" => files.push("-"),
                "--" => {
                    for rest in iter.by_ref() {
                        files.push(rest);
                    }
                    break;
                }
                s if !s.starts_with('-') => files.push(s),
                s => {
                    eprintln!("expand: unrecognized option: '{}'", s);
                    return 1;
                }
            }
        }
        let stop_for = |col: usize| -> usize {
            if tabs.len() == 1 {
                let t = tabs[0];
                col + (t - col % t)
            } else {
                // Multi-stop: find the first stop > col, else 1-extend.
                for &s in &tabs {
                    if s > col {
                        return s;
                    }
                }
                col + 1
            }
        };
        let targets: Vec<&str> = if files.is_empty() { vec!["-"] } else { files };
        for file in targets {
            let reader: Box<dyn BufRead> = if file == "-" {
                Box::new(BufReader::new(std::io::stdin()))
            } else {
                match std::fs::File::open(file) {
                    Ok(f) => Box::new(BufReader::new(f)),
                    Err(e) => {
                        eprintln!("expand: {}: {}", file, e);
                        return 1;
                    }
                }
            };
            for line in reader.lines().map_while(Result::ok) {
                let mut col = 0usize;
                let mut out = String::with_capacity(line.len());
                for c in line.chars() {
                    if c == '\t' {
                        let target = stop_for(col);
                        while col < target {
                            out.push(' ');
                            col += 1;
                        }
                    } else {
                        out.push(c);
                        col += 1;
                    }
                }
                println!("{}", out);
            }
        }
        0
    }

    /// unexpand [-a] [-t TAB] [FILE...] — convert spaces to tabs.
    /// coreutils unexpand(1).  Default tabstop 8; -a converts every
    /// run of spaces (not just leading); without -a only leading
    /// runs collapse.
    pub(crate) fn builtin_unexpand(&self, args: &[String]) -> i32 {
        let mut tabstop: usize = 8;
        let mut all_runs = false;
        let mut files: Vec<&str> = Vec::new();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-a" | "--all" => all_runs = true,
                "-t" | "--tabs" => {
                    if let Some(s) = iter.next() {
                        if let Ok(n) = s.parse() {
                            tabstop = n;
                            all_runs = true;
                        }
                    }
                }
                s if s.starts_with("-t") && s.len() > 2 => {
                    if let Ok(n) = s[2..].parse() {
                        tabstop = n;
                        all_runs = true;
                    }
                }
                "-" => files.push("-"),
                "--" => {
                    for rest in iter.by_ref() {
                        files.push(rest);
                    }
                    break;
                }
                s if !s.starts_with('-') => files.push(s),
                s => {
                    eprintln!("unexpand: unrecognized option: '{}'", s);
                    return 1;
                }
            }
        }
        let targets: Vec<&str> = if files.is_empty() { vec!["-"] } else { files };
        for file in targets {
            let reader: Box<dyn BufRead> = if file == "-" {
                Box::new(BufReader::new(std::io::stdin()))
            } else {
                match std::fs::File::open(file) {
                    Ok(f) => Box::new(BufReader::new(f)),
                    Err(e) => {
                        eprintln!("unexpand: {}: {}", file, e);
                        return 1;
                    }
                }
            };
            for line in reader.lines().map_while(Result::ok) {
                let mut out = String::with_capacity(line.len());
                let mut col = 0usize;
                let chars: Vec<char> = line.chars().collect();
                let mut i = 0;
                let mut leading = true;
                while i < chars.len() {
                    if chars[i] == ' ' && (all_runs || leading) {
                        // Count run of spaces.
                        let start_col = col;
                        let mut j = i;
                        while j < chars.len() && chars[j] == ' ' {
                            j += 1;
                            col += 1;
                        }
                        // Compress as many tabs as possible.
                        let mut cur = start_col;
                        let next_stop = |c: usize| (c / tabstop + 1) * tabstop;
                        while cur + (next_stop(cur) - cur) <= col {
                            let s = next_stop(cur);
                            out.push('\t');
                            cur = s;
                        }
                        // Pad remainder with spaces.
                        for _ in cur..col {
                            out.push(' ');
                        }
                        i = j;
                    } else {
                        out.push(chars[i]);
                        if chars[i] != ' ' && chars[i] != '\t' {
                            leading = false;
                        }
                        if chars[i] == '\t' {
                            col = (col / tabstop + 1) * tabstop;
                        } else {
                            col += 1;
                        }
                        i += 1;
                    }
                }
                println!("{}", out);
            }
        }
        0
    }

    /// sha256sum [FILE...] — write SHA-256 of each file (or stdin
    /// when no FILE / '-'). coreutils-style 'HEX  PATH' output.
    pub(crate) fn builtin_sha256sum(&self, args: &[String]) -> i32 {
        // Validate flags: silent-drop accepted any unknown -X. coreutils
        // sha256sum specifically supports -b/-t/--binary/--text (we
        // accept them as no-ops since output format is identical), -
        // (stdin), and `--`. Anything else errors.
        let mut files: Vec<&str> = Vec::new();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-" => files.push("-"),
                "-b" | "-t" | "--binary" | "--text" => {} // accept, no-op
                "--" => {
                    for rest in iter.by_ref() {
                        files.push(rest);
                    }
                    break;
                }
                s if !s.starts_with('-') => files.push(s),
                s => {
                    eprintln!("sha256sum: unrecognized option: '{}'", s);
                    return 1;
                }
            }
        }
        let targets: Vec<&str> = if files.is_empty() { vec!["-"] } else { files };
        let mut status = 0;
        for f in targets {
            let mut hasher = Sha256::new();
            let result: std::io::Result<()> = (|| {
                let mut buf = [0u8; 65536];
                if f == "-" {
                    let stdin = std::io::stdin();
                    let mut h = stdin.lock();
                    loop {
                        let n = h.read(&mut buf)?;
                        if n == 0 {
                            break;
                        }
                        hasher.update(&buf[..n]);
                    }
                } else {
                    let mut file = std::fs::File::open(f)?;
                    loop {
                        let n = file.read(&mut buf)?;
                        if n == 0 {
                            break;
                        }
                        hasher.update(&buf[..n]);
                    }
                }
                Ok(())
            })();
            match result {
                Ok(()) => {
                    let hex = format!("{:x}", hasher.finalize());
                    if f == "-" {
                        println!("{}  -", hex);
                    } else {
                        println!("{}  {}", hex, f);
                    }
                }
                Err(e) => {
                    eprintln!("sha256sum: {}: {}", f, e);
                    status = 1;
                }
            }
        }
        status
    }

    /// base64 [-d] [FILE] — encode/decode base64. coreutils
    /// base64(1) without --wrap (defaults to 76-char wrap on
    /// encode; 0 disables).
    pub(crate) fn builtin_base64(&self, args: &[String]) -> i32 {
        const ALPHA: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut decode = false;
        let mut wrap: usize = 76;
        let mut file: Option<&str> = None;
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-d" | "--decode" => decode = true,
                "-w" | "--wrap" => {
                    if let Some(s) = iter.next() {
                        wrap = s.parse().unwrap_or(76);
                    }
                }
                s if s.starts_with("--wrap=") => {
                    wrap = s[7..].parse().unwrap_or(76);
                }
                "-i" | "--ignore-garbage" => {} // accepted, default behaviour
                "--" => {}                      // end of options
                "-" => file = Some("-"),
                s if !s.starts_with('-') => file = Some(s),
                s => {
                    eprintln!("base64: unrecognized option: '{}'", s);
                    return 1;
                }
            }
        }
        let mut input = Vec::new();
        match file {
            Some(f) if f != "-" => match std::fs::File::open(f) {
                Ok(mut h) => {
                    let _ = h.read_to_end(&mut input);
                }
                Err(e) => {
                    eprintln!("base64: {}: {}", f, e);
                    return 1;
                }
            },
            _ => {
                let stdin = std::io::stdin();
                let _ = stdin.lock().read_to_end(&mut input);
            }
        }
        if decode {
            // Strip whitespace then decode 4-char groups.
            let cleaned: Vec<u8> = input
                .iter()
                .copied()
                .filter(|b| !b.is_ascii_whitespace())
                .collect();
            let mut out: Vec<u8> = Vec::with_capacity(cleaned.len() * 3 / 4);
            let mut buf = [0u8; 4];
            let mut have = 0usize;
            for &c in &cleaned {
                if c == b'=' {
                    break;
                }
                let v = match c {
                    b'A'..=b'Z' => c - b'A',
                    b'a'..=b'z' => c - b'a' + 26,
                    b'0'..=b'9' => c - b'0' + 52,
                    b'+' => 62,
                    b'/' => 63,
                    _ => continue,
                };
                buf[have] = v;
                have += 1;
                if have == 4 {
                    out.push((buf[0] << 2) | (buf[1] >> 4));
                    out.push((buf[1] << 4) | (buf[2] >> 2));
                    out.push((buf[2] << 6) | buf[3]);
                    have = 0;
                }
            }
            // Handle trailing 2/3 chars.
            if have == 2 {
                out.push((buf[0] << 2) | (buf[1] >> 4));
            } else if have == 3 {
                out.push((buf[0] << 2) | (buf[1] >> 4));
                out.push((buf[1] << 4) | (buf[2] >> 2));
            }
            let _ = std::io::stdout().write_all(&out);
        } else {
            let mut out = String::with_capacity(input.len() * 4 / 3 + 4);
            let mut col = 0usize;
            let push_char = |c: u8, out: &mut String, col: &mut usize| {
                out.push(c as char);
                *col += 1;
                if wrap > 0 && *col >= wrap {
                    out.push('\n');
                    *col = 0;
                }
            };
            let mut i = 0;
            while i + 3 <= input.len() {
                let n = ((input[i] as u32) << 16)
                    | ((input[i + 1] as u32) << 8)
                    | (input[i + 2] as u32);
                push_char(ALPHA[((n >> 18) & 0x3f) as usize], &mut out, &mut col);
                push_char(ALPHA[((n >> 12) & 0x3f) as usize], &mut out, &mut col);
                push_char(ALPHA[((n >> 6) & 0x3f) as usize], &mut out, &mut col);
                push_char(ALPHA[(n & 0x3f) as usize], &mut out, &mut col);
                i += 3;
            }
            let rem = input.len() - i;
            if rem == 1 {
                let n = (input[i] as u32) << 16;
                push_char(ALPHA[((n >> 18) & 0x3f) as usize], &mut out, &mut col);
                push_char(ALPHA[((n >> 12) & 0x3f) as usize], &mut out, &mut col);
                push_char(b'=', &mut out, &mut col);
                push_char(b'=', &mut out, &mut col);
            } else if rem == 2 {
                let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8);
                push_char(ALPHA[((n >> 18) & 0x3f) as usize], &mut out, &mut col);
                push_char(ALPHA[((n >> 12) & 0x3f) as usize], &mut out, &mut col);
                push_char(ALPHA[((n >> 6) & 0x3f) as usize], &mut out, &mut col);
                push_char(b'=', &mut out, &mut col);
            }
            if col != 0 {
                out.push('\n');
            }
            print!("{}", out);
        }
        0
    }

    /// nproc — print number of online CPUs. coreutils nproc(1).
    /// --all uses CPU count from sysconf(_SC_NPROCESSORS_CONF);
    /// default uses _SC_NPROCESSORS_ONLN (the schedulable subset).
    pub(crate) fn builtin_nproc(&self, args: &[String]) -> i32 {
        // coreutils nproc accepts --all and --ignore=N. Validate
        // unknown flags rather than the previous silent accept.
        let mut want_all = false;
        let mut ignore: i64 = 0;
        for arg in args {
            match arg.as_str() {
                "--all" => want_all = true,
                s if s.starts_with("--ignore=") => {
                    ignore = s[9..].parse().unwrap_or(0);
                }
                "--ignore" => {
                    // separate-arg form not common; coreutils accepts
                    // --ignore=N. Skip if standalone, treat as no-op.
                }
                "--" => {}
                s if s.starts_with('-') && s.len() > 1 => {
                    eprintln!("nproc: invalid option: '{}'", s);
                    return 1;
                }
                _ => {}
            }
        }
        let n = unsafe {
            if want_all {
                libc::sysconf(libc::_SC_NPROCESSORS_CONF)
            } else {
                libc::sysconf(libc::_SC_NPROCESSORS_ONLN)
            }
        };
        let mut count = if n <= 0 { 1 } else { n as i64 };
        count = (count - ignore).max(1);
        println!("{}", count);
        0
    }

    /// expr ARG... — evaluate expression. POSIX expr(1).
    /// Subset port: integer arithmetic (+ - * / %), string match
    /// ': REGEX', length STRING, substr STRING POS LEN, index
    /// STRING CHARS. Recognizes the closing arg list directly
    /// (single-pass shunting-yard would be heavier than needed
    /// for the common scripts).
    pub(crate) fn builtin_expr(&self, args: &[String]) -> i32 {
        if args.is_empty() {
            eprintln!("expr: missing operand");
            return 2;
        }
        // Strip optional leading '--'.
        let mut argv: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        if argv[0] == "--" {
            argv.remove(0);
        }
        // Single-arg shortcuts.
        if argv.len() == 1 {
            println!("{}", argv[0]);
            return if argv[0].is_empty() || argv[0] == "0" {
                1
            } else {
                0
            };
        }
        // 'length STRING' / 'substr STR POS LEN' / 'index STR CHARS'.
        match argv[0] {
            "length" if argv.len() == 2 => {
                let n = argv[1].chars().count();
                println!("{}", n);
                return if n == 0 { 1 } else { 0 };
            }
            "substr" if argv.len() == 4 => {
                let pos: i64 = argv[2].parse().unwrap_or(1);
                let len: i64 = argv[3].parse().unwrap_or(0);
                let s = argv[1];
                let start = (pos - 1).max(0) as usize;
                let end = (start + len.max(0) as usize).min(s.chars().count());
                if start >= s.chars().count() || len <= 0 {
                    println!();
                    return 1;
                }
                let out: String = s.chars().skip(start).take(end - start).collect();
                println!("{}", out);
                return if out.is_empty() { 1 } else { 0 };
            }
            "index" if argv.len() == 3 => {
                let s = argv[1];
                let chars = argv[2];
                for (i, c) in s.char_indices() {
                    if chars.contains(c) {
                        // 1-based; coreutils returns char index, not byte.
                        let n = s[..i].chars().count() + 1;
                        println!("{}", n);
                        return 0;
                    }
                }
                println!("0");
                return 1;
            }
            _ => {}
        }
        // Three-arg infix ops: STR OP STR. Numeric for + - * / %,
        // string for = != < > <= >=, ':' for prefix match.
        if argv.len() == 3 {
            let a = argv[0];
            let op = argv[1];
            let b = argv[2];
            let try_int = |s: &str| -> Option<i64> { s.parse().ok() };
            let result: String = match op {
                "+" | "-" | "*" | "/" | "%" => {
                    let ai = try_int(a).unwrap_or(0);
                    let bi = try_int(b).unwrap_or(0);
                    let v = match op {
                        "+" => ai + bi,
                        "-" => ai - bi,
                        "*" => ai * bi,
                        "/" => {
                            if bi == 0 {
                                eprintln!("expr: division by zero");
                                return 2;
                            }
                            ai / bi
                        }
                        "%" => {
                            if bi == 0 {
                                eprintln!("expr: division by zero");
                                return 2;
                            }
                            ai % bi
                        }
                        _ => 0,
                    };
                    v.to_string()
                }
                "=" | "==" => (a == b).to_string(),
                "!=" => (a != b).to_string(),
                "<" => match (try_int(a), try_int(b)) {
                    (Some(x), Some(y)) => (x < y).to_string(),
                    _ => (a < b).to_string(),
                },
                ">" => match (try_int(a), try_int(b)) {
                    (Some(x), Some(y)) => (x > y).to_string(),
                    _ => (a > b).to_string(),
                },
                "<=" => match (try_int(a), try_int(b)) {
                    (Some(x), Some(y)) => (x <= y).to_string(),
                    _ => (a <= b).to_string(),
                },
                ">=" => match (try_int(a), try_int(b)) {
                    (Some(x), Some(y)) => (x >= y).to_string(),
                    _ => (a >= b).to_string(),
                },
                ":" => {
                    // Anchored regex match; if pattern has a capture
                    // group, output the capture; else the match length.
                    let re = match regex::Regex::new(&format!("^{}", b)) {
                        Ok(r) => r,
                        Err(_) => {
                            eprintln!("expr: invalid regex: {}", b);
                            return 2;
                        }
                    };
                    if let Some(c) = re.captures(a) {
                        if let Some(g1) = c.get(1) {
                            g1.as_str().to_string()
                        } else {
                            c.get(0).map(|m| m.range().len()).unwrap_or(0).to_string()
                        }
                    } else {
                        "0".to_string()
                    }
                }
                _ => {
                    eprintln!("expr: unknown operator: {}", op);
                    return 2;
                }
            };
            println!("{}", result);
            return if result.is_empty() || result == "0" || result == "false" {
                1
            } else {
                0
            };
        }
        // Fallback: just print joined.
        println!("{}", argv.join(" "));
        0
    }

    /// printenv [VAR...] — print env. coreutils printenv(1).
    /// No args: print all env vars (sorted by key for stable output).
    /// Args: print VAR's value per arg, exit 1 if any unset.
    pub(crate) fn builtin_printenv(&self, args: &[String]) -> i32 {
        // coreutils printenv has -0 / --null (NUL-terminate output).
        // Old impl ignored flags silently and treated \`-0\` as a
        // variable name lookup, which always failed since no env var
        // is named \`-0\`.
        let mut zero_term = false;
        let mut names: Vec<&str> = Vec::new();
        for arg in args {
            match arg.as_str() {
                "-0" | "--null" => zero_term = true,
                "--" => {} // end of options
                s if !s.starts_with('-') => names.push(s),
                s => {
                    eprintln!("printenv: unrecognized option: '{}'", s);
                    return 1;
                }
            }
        }
        let term: char = if zero_term { '\0' } else { '\n' };
        if names.is_empty() {
            let mut vars: Vec<(String, String)> = std::env::vars().collect();
            vars.sort_by(|a, b| a.0.cmp(&b.0));
            for (k, v) in vars {
                print!("{}={}{}", k, v, term);
            }
            return 0;
        }
        let mut status = 0;
        for name in names {
            match std::env::var(name) {
                Ok(v) => print!("{}{}", v, term),
                Err(_) => status = 1,
            }
        }
        status
    }

    /// tty — print the controlling-terminal device path. coreutils
    /// tty(1). -s suppresses output (just sets the exit code).
    pub(crate) fn builtin_tty(&self, args: &[String]) -> i32 {
        let mut silent = false;
        for arg in args {
            match arg.as_str() {
                "-s" | "--silent" | "--quiet" => silent = true,
                "--" => {}
                s if s.starts_with('-') && s.len() > 1 => {
                    eprintln!("tty: unrecognized option: '{}'", s);
                    return 1;
                }
                _ => {} // bare arg ignored — coreutils tty takes no operands
            }
        }
        unsafe {
            let p = libc::ttyname(0);
            if p.is_null() {
                if !silent {
                    println!("not a tty");
                }
                return 1;
            }
            if !silent {
                let s = std::ffi::CStr::from_ptr(p);
                println!("{}", s.to_string_lossy());
            }
        }
        0
    }

    /// yes [STRING] — print STRING (or 'y') forever. coreutils
    /// yes(1). Honors SIGPIPE: when stdout is piped and the consumer
    /// closes, the write fails and we exit 0 silently.
    pub(crate) fn builtin_yes(&self, args: &[String]) -> i32 {
        // coreutils yes: \`yes --help\` / \`yes --version\` print
        // help/version and exit when --help/--version is the sole
        // arg. With multiple args (\`yes --help foo\`), --help is
        // treated as part of the literal string to repeat (matches
        // GNU yes 9.x exactly).
        if args.len() == 1 {
            match args[0].as_str() {
                "--help" => {
                    println!("Usage: yes [STRING]...");
                    println!("Repeatedly output STRING, or 'y' if STRING omitted.");
                    return 0;
                }
                "--version" => {
                    println!("yes (zshrs) {}", env!("CARGO_PKG_VERSION"));
                    return 0;
                }
                _ => {}
            }
        }
        let line = if args.is_empty() {
            "y\n".to_string()
        } else {
            format!("{}\n", args.join(" "))
        };
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        // Buffer ~64KB worth of repeats per write to amortize syscall.
        let mut buf = String::with_capacity(65536);
        while buf.len() + line.len() <= 65536 {
            buf.push_str(&line);
        }
        loop {
            if out.write_all(buf.as_bytes()).is_err() {
                return 0; // SIGPIPE / closed consumer
            }
        }
    }

    /// nl [-b STYLE] [FILE...] — number lines. Direct port of
    /// coreutils nl(1) for the most-used flag set.
    pub(crate) fn builtin_nl(&self, args: &[String]) -> i32 {
        // GNU coreutils nl(1). Numbering styles per section: `a` (all),
        // `t` (non-empty, body default), `n` (none, header/footer
        // default), `pREGEX` (lines matching REGEX). Number formats
        // (-n): `ln` left, `rn` right (default), `rz` right zero-pad.
        // Logical pages: input lines of exactly DELIM*3/2/1 (default
        // `\:`) switch to header/body/footer, are replaced by empty
        // output lines, and reset numbering unless -p. GNU convention
        // is followed where BSD differs (unnumbered lines emit spaces
        // in place of number AND separator; delimiter lines emit an
        // empty line).
        #[derive(Clone)]
        enum NlStyle {
            All,
            NonEmpty,
            None,
            Pat(regex::Regex),
        }
        fn parse_style(s: &str, opt: char) -> Result<NlStyle, i32> {
            match s.chars().next() {
                Some('a') => Ok(NlStyle::All),
                Some('t') => Ok(NlStyle::NonEmpty),
                Some('n') => Ok(NlStyle::None),
                Some('p') => match regex::Regex::new(&s[1..]) {
                    Ok(re) => Ok(NlStyle::Pat(re)),
                    Err(e) => {
                        eprintln!("nl: invalid regular expression: {}", e);
                        Err(1)
                    }
                },
                _ => {
                    eprintln!("nl: invalid numbering style: '-{}{}'", opt, s);
                    Err(1)
                }
            }
        }
        let mut body = NlStyle::NonEmpty;
        let mut header = NlStyle::None;
        let mut footer = NlStyle::None;
        let mut delim = "\\:".to_string();
        let mut start = 1i64;
        let mut step = 1i64;
        let mut join_blanks = 1i64; // -l
        let mut fmt = "rn".to_string(); // -n
        let mut renumber = true; // -p clears
        let mut sep = "\t".to_string();
        let mut width = 6usize;
        let mut files: Vec<String> = Vec::new();
        let mut i = 0usize;
        let mut no_more_opts = false;
        // Short opts take attached (`-nrz`) or separate (`-n rz`) args.
        macro_rules! optarg {
            ($rest:expr, $name:expr) => {{
                if !$rest.is_empty() {
                    $rest.to_string()
                } else {
                    i += 1;
                    match args.get(i) {
                        Some(v) => v.clone(),
                        None => {
                            eprintln!("nl: option requires an argument -- '{}'", $name);
                            return 1;
                        }
                    }
                }
            }};
        }
        while i < args.len() {
            let arg = args[i].as_str();
            if no_more_opts || arg == "-" || !arg.starts_with('-') {
                files.push(arg.to_string());
                i += 1;
                continue;
            }
            if arg == "--" {
                no_more_opts = true;
                i += 1;
                continue;
            }
            let (opt, rest): (&str, &str) = if let Some(long) = arg.strip_prefix("--") {
                match long.split_once('=') {
                    Some((k, v)) => (k, v),
                    None => (long, ""),
                }
            } else {
                (&arg[1..2], &arg[2..])
            };
            match opt {
                "b" | "body-numbering" => {
                    let v = optarg!(rest, 'b');
                    match parse_style(&v, 'b') {
                        Ok(st) => body = st,
                        Err(rc) => return rc,
                    }
                }
                "h" | "header-numbering" => {
                    let v = optarg!(rest, 'h');
                    match parse_style(&v, 'h') {
                        Ok(st) => header = st,
                        Err(rc) => return rc,
                    }
                }
                "f" | "footer-numbering" => {
                    let v = optarg!(rest, 'f');
                    match parse_style(&v, 'f') {
                        Ok(st) => footer = st,
                        Err(rc) => return rc,
                    }
                }
                "d" | "section-delimiter" => {
                    let v = optarg!(rest, 'd');
                    // GNU: a single-char arg keeps ':' as 2nd char.
                    delim = if v.chars().count() == 1 {
                        format!("{}:", v)
                    } else {
                        v
                    };
                }
                "i" | "line-increment" => {
                    let v = optarg!(rest, 'i');
                    step = v.parse().unwrap_or(1);
                }
                "l" | "join-blank-lines" => {
                    let v = optarg!(rest, 'l');
                    join_blanks = v.parse::<i64>().unwrap_or(1).max(1);
                }
                "n" | "number-format" => {
                    let v = optarg!(rest, 'n');
                    match v.as_str() {
                        "ln" | "rn" | "rz" => fmt = v,
                        _ => {
                            eprintln!("nl: invalid line numbering format: '{}'", v);
                            return 1;
                        }
                    }
                }
                "p" | "no-renumber" => {
                    renumber = false;
                    // -p takes no arg; attached chars are more short opts
                    // (rare) — treat as error to stay simple and loud.
                    if !rest.is_empty() {
                        eprintln!("nl: unrecognized option: '{}'", arg);
                        return 1;
                    }
                }
                "s" | "number-separator" => {
                    sep = optarg!(rest, 's');
                }
                "v" | "starting-line-number" => {
                    let v = optarg!(rest, 'v');
                    start = v.parse().unwrap_or(1);
                }
                "w" | "number-width" => {
                    let v = optarg!(rest, 'w');
                    width = v.parse().unwrap_or(6).max(1);
                }
                _ => {
                    eprintln!("nl: unrecognized option: '{}'", arg);
                    return 1;
                }
            }
            i += 1;
        }
        if files.is_empty() {
            files.push("-".to_string());
        }
        let hdr_delim = format!("{0}{0}{0}", delim);
        let body_delim = format!("{0}{0}", delim);
        let mut n = start;
        let mut section = 1u8; // 0=header 1=body 2=footer
        let mut blank_run = 0i64;
        for file in &files {
            let reader: Box<dyn BufRead> = if file == "-" {
                Box::new(BufReader::new(std::io::stdin()))
            } else {
                match std::fs::File::open(file) {
                    Ok(f) => Box::new(BufReader::new(f)),
                    Err(e) => {
                        eprintln!("nl: {}: {}", file, e);
                        return 1;
                    }
                }
            };
            for line in reader.lines().map_while(Result::ok) {
                // Section delimiters — full-line match only.
                let new_section = if line == hdr_delim {
                    Some(0u8)
                } else if line == body_delim {
                    Some(1u8)
                } else if line == delim {
                    Some(2u8)
                } else {
                    None
                };
                if let Some(sec) = new_section {
                    section = sec;
                    if renumber {
                        n = start;
                    }
                    blank_run = 0;
                    println!();
                    continue;
                }
                let style = match section {
                    0 => &header,
                    2 => &footer,
                    _ => &body,
                };
                let blank = line.is_empty();
                let do_number = match style {
                    NlStyle::All => {
                        // -l NUM: only the NUM-th of a run of blanks
                        // gets a number (GNU join-blank-lines).
                        if blank {
                            blank_run += 1;
                            if blank_run == join_blanks {
                                blank_run = 0;
                                true
                            } else {
                                false
                            }
                        } else {
                            blank_run = 0;
                            true
                        }
                    }
                    NlStyle::NonEmpty => {
                        blank_run = 0;
                        !blank
                    }
                    NlStyle::None => {
                        blank_run = 0;
                        false
                    }
                    NlStyle::Pat(re) => {
                        blank_run = 0;
                        re.is_match(&line)
                    }
                };
                if do_number {
                    match fmt.as_str() {
                        "ln" => println!("{:<width$}{}{}", n, sep, line, width = width),
                        "rz" => println!("{:0>width$}{}{}", n, sep, line, width = width),
                        _ => println!("{:>width$}{}{}", n, sep, line, width = width),
                    }
                    n += step;
                } else {
                    // GNU: spaces replace both the number and the
                    // separator on unnumbered lines.
                    println!("{}{}", " ".repeat(width + sep.len()), line);
                }
            }
        }
        0
    }

    /// env [-i] [NAME=VALUE]... [COMMAND [ARG]...] — print env or
    /// run COMMAND with modified environment. Direct port of
    /// coreutils env(1) for the most-used invocations.
    pub(crate) fn builtin_env(&mut self, args: &[String]) -> i32 {
        let mut clear_env = false;
        let mut unset: Vec<String> = Vec::new();
        let mut assignments: Vec<(String, String)> = Vec::new();
        let mut cmd_start: Option<usize> = None;
        let mut i = 0;
        while i < args.len() {
            let a = &args[i];
            if a == "-i" || a == "--ignore-environment" {
                clear_env = true;
                i += 1;
                continue;
            }
            if a == "-u" || a == "--unset" {
                if i + 1 < args.len() {
                    unset.push(args[i + 1].clone());
                    i += 2;
                    continue;
                }
                eprintln!("env: -u: missing argument");
                return 125;
            }
            if let Some(name) = a.strip_prefix("--unset=") {
                unset.push(name.to_string());
                i += 1;
                continue;
            }
            if a == "--" {
                cmd_start = Some(i + 1);
                break;
            }
            if let Some(eq) = a.find('=') {
                if !a[..eq].is_empty() && !a.starts_with('-') {
                    assignments.push((a[..eq].to_string(), a[eq + 1..].to_string()));
                    i += 1;
                    continue;
                }
            }
            if !a.starts_with('-') {
                cmd_start = Some(i);
                break;
            }
            // Unknown -X flag; emit error like coreutils.
            eprintln!("env: invalid option: {}", a);
            return 125;
        }

        let cmd_args: Vec<&str> = match cmd_start {
            Some(s) => args[s..].iter().map(|x| x.as_str()).collect(),
            None => Vec::new(),
        };

        // Build the env: optionally clear, drop -u names, apply
        // assignments.
        let env_overrides: Vec<(String, String)> = if clear_env {
            assignments.clone()
        } else {
            let mut out: Vec<(String, String)> = std::env::vars()
                .filter(|(k, _)| !unset.contains(k))
                .collect();
            for (k, v) in &assignments {
                out.retain(|(ek, _)| ek != k);
                out.push((k.clone(), v.clone()));
            }
            out
        };

        if cmd_args.is_empty() {
            // Print env, sorted by key for stable output (matches
            // GNU env's typical alphabetical layout).
            let mut sorted = env_overrides;
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            for (k, v) in sorted {
                println!("{}={}", k, v);
            }
            return 0;
        }

        // Run the command with the modified environment. Spawn via
        // std::process::Command since we're a coreutils-style env
        // shim, not a shell-level builtin.
        let mut cmd = std::process::Command::new(cmd_args[0]);
        cmd.args(&cmd_args[1..]);
        if clear_env {
            cmd.env_clear();
        }
        for u in &unset {
            cmd.env_remove(u);
        }
        for (k, v) in &assignments {
            cmd.env(k, v);
        }
        match cmd.status() {
            Ok(status) => status.code().unwrap_or(127),
            Err(_) => 127,
        }
    }

    pub(crate) fn builtin_whoami(&self, args: &[String]) -> i32 {
        // coreutils whoami(1) prints the EFFECTIVE user name, not
        // \$USER. After 'sudo whoami', \$USER may still be the
        // original (depending on sudo config) — but whoami should
        // print the effective user. Direct port via geteuid +
        // getpwuid.
        // whoami takes no operands. Reject unknown flags (was
        // silently ignored via the unused _args arg).
        for arg in args {
            if arg.starts_with('-') && arg.len() > 1 && arg != "--" {
                eprintln!("whoami: unrecognized option: '{}'", arg);
                return 1;
            }
            if !arg.starts_with('-') {
                eprintln!("whoami: extra operand '{}'", arg);
                return 1;
            }
        }
        let euid = unsafe { libc::geteuid() };
        unsafe {
            let pw = libc::getpwuid(euid);
            if !pw.is_null() {
                let name = CStr::from_ptr((*pw).pw_name);
                println!("{}", name.to_string_lossy());
                return 0;
            }
        }
        // Fallback: numeric uid (matches coreutils 'cannot find name'
        // error case).
        eprintln!("whoami: cannot find name for user ID {}", euid);
        1
    }

    pub(crate) fn builtin_id(&self, args: &[String]) -> i32 {
        // coreutils id(1) port: -u/-g/-G with -n name modifier, plus
        // the default 'uid=N(name) gid=N(name) groups=...' form.

        let uid = unsafe { libc::getuid() };
        let gid = unsafe { libc::getgid() };
        let euid = unsafe { libc::geteuid() };
        let egid = unsafe { libc::getegid() };

        // Parse flag combinations: -u, -g, -G, -un, -gn, -Gn, -nu, etc.
        let mut want_uid = false;
        let mut want_gid = false;
        let mut want_groups = false;
        let mut want_name = false;
        for arg in args {
            if let Some(s) = arg.strip_prefix('-') {
                for c in s.chars() {
                    match c {
                        'u' => want_uid = true,
                        'g' => want_gid = true,
                        'G' => want_groups = true,
                        'n' => want_name = true,
                        'r' => {} // -r: real id (we already use real uid/gid)
                        // coreutils id rejects unknown flags. Old
                        // \`_ => {}\` accepted any letter and the
                        // remaining letters fell through to the
                        // default print-everything path.
                        _ => {
                            eprintln!("id: invalid option -- '{}'", c);
                            return 1;
                        }
                    }
                }
            }
        }

        let lookup_user_name = |uid: u32| -> Option<String> {
            unsafe {
                let pw = libc::getpwuid(uid);
                if pw.is_null() {
                    return None;
                }
                let name = CStr::from_ptr((*pw).pw_name);
                Some(name.to_string_lossy().into_owned())
            }
        };
        let lookup_group_name = |gid: u32| -> Option<String> {
            unsafe {
                let gr = libc::getgrgid(gid);
                if gr.is_null() {
                    return None;
                }
                let name = CStr::from_ptr((*gr).gr_name);
                Some(name.to_string_lossy().into_owned())
            }
        };

        if want_uid {
            if want_name {
                println!(
                    "{}",
                    lookup_user_name(uid).unwrap_or_else(|| uid.to_string())
                );
            } else {
                println!("{}", uid);
            }
            return 0;
        }
        if want_gid {
            if want_name {
                println!(
                    "{}",
                    lookup_group_name(gid).unwrap_or_else(|| gid.to_string())
                );
            } else {
                println!("{}", gid);
            }
            return 0;
        }
        if want_groups {
            // getgrouplist(name, base_gid, gids[], &count). First call
            // gets the count; second populates the array.
            let user_name = lookup_user_name(uid).unwrap_or_default();
            let cname = std::ffi::CString::new(user_name.as_bytes()).unwrap_or_default();
            let mut count: libc::c_int = 32;
            let mut gids: Vec<libc::gid_t> = vec![0; count as usize];
            let rc = unsafe {
                libc::getgrouplist(
                    cname.as_ptr(),
                    gid as _,
                    gids.as_mut_ptr() as *mut _,
                    &mut count,
                )
            };
            if rc < 0 {
                gids.resize(count as usize, 0);
                unsafe {
                    libc::getgrouplist(
                        cname.as_ptr(),
                        gid as _,
                        gids.as_mut_ptr() as *mut _,
                        &mut count,
                    );
                }
            }
            gids.truncate(count.max(0) as usize);
            let parts: Vec<String> = gids
                .iter()
                .map(|g| {
                    if want_name {
                        lookup_group_name(*g).unwrap_or_else(|| g.to_string())
                    } else {
                        g.to_string()
                    }
                })
                .collect();
            println!("{}", parts.join(" "));
            return 0;
        }

        // Default form: uid=N(name) gid=N(name) groups=N(name),...
        let user = lookup_user_name(uid).unwrap_or_else(|| uid.to_string());
        let group = lookup_group_name(gid).unwrap_or_else(|| gid.to_string());
        print!("uid={}({}) gid={}({})", uid, user, gid, group);
        if euid != uid {
            let eu = lookup_user_name(euid).unwrap_or_else(|| euid.to_string());
            print!(" euid={}({})", euid, eu);
        }
        if egid != gid {
            let eg = lookup_group_name(egid).unwrap_or_else(|| egid.to_string());
            print!(" egid={}({})", egid, eg);
        }
        // Supplementary groups list
        let cname = std::ffi::CString::new(user.as_bytes()).unwrap_or_default();
        let mut count: libc::c_int = 32;
        let mut gids: Vec<libc::gid_t> = vec![0; count as usize];
        let rc = unsafe {
            libc::getgrouplist(
                cname.as_ptr(),
                gid as _,
                gids.as_mut_ptr() as *mut _,
                &mut count,
            )
        };
        if rc < 0 {
            gids.resize(count as usize, 0);
            unsafe {
                libc::getgrouplist(
                    cname.as_ptr(),
                    gid as _,
                    gids.as_mut_ptr() as *mut _,
                    &mut count,
                );
            }
        }
        gids.truncate(count.max(0) as usize);
        if !gids.is_empty() {
            let parts: Vec<String> = gids
                .iter()
                .map(|g| {
                    let name = lookup_group_name(*g).unwrap_or_else(|| g.to_string());
                    format!("{}({})", g, name)
                })
                .collect();
            print!(" groups={}", parts.join(","));
        }
        println!();
        0
    }

    pub(crate) fn builtin_hostname(&self, args: &[String]) -> i32 {
        // hostname(1) — accepts:
        // -s / --short: short hostname (everything before the first '.')
        // -d / --domain: domain part only (everything after first '.')
        // -f / --fqdn / --long: full hostname (default behaviour)
        // -i / --ip-address: numeric IP for the hostname
        // bare arg: in some platforms sets the hostname (root only); we
        //           accept it as a query-only no-op for safety.
        let mut short = false;
        let mut domain_only = false;
        let mut ip = false;
        for arg in args {
            match arg.as_str() {
                "-s" | "--short" => short = true,
                "-d" | "--domain" => domain_only = true,
                "-f" | "--fqdn" | "--long" => {}
                "-i" | "--ip-address" => ip = true,
                "--" => {}
                s if s.starts_with('-') && s.len() > 1 => {
                    // hostname(1) errors on unknown flags. Old impl
                    // accepted any -X silently then printed the
                    // hostname as-if -X were a no-op.
                    eprintln!("hostname: invalid option: '{}'", s);
                    return 1;
                }
                _ => {} // bare arg: would set hostname (root); we accept silently
            }
        }

        let mut buf = [0u8; 256];
        // c_char is i8 on most targets but u8 on aarch64-linux; cast through
        // libc::c_char so this builds on every Unix target the matrix covers.
        let result = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) };
        if result != 0 {
            eprintln!("hostname: cannot get hostname");
            return 1;
        }
        let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        let host = String::from_utf8_lossy(&buf[..len]).into_owned();

        if ip {
            // Resolve the hostname via getaddrinfo and print the
            // first IPv4/IPv6 result. Same approach as
            // hostname(1)'s -i.
            if let Ok(mut addrs) = (host.as_str(), 0u16).to_socket_addrs() {
                if let Some(a) = addrs.next() {
                    println!("{}", a.ip());
                    return 0;
                }
            }
            // Fall through to printing the host if resolution failed.
        }
        if short {
            let s = host.split('.').next().unwrap_or(&host);
            println!("{}", s);
        } else if domain_only {
            let d: String = host
                .split_once('.')
                .map(|x| x.1)
                .map(|s| s.to_string())
                .unwrap_or_default();
            println!("{}", d);
        } else {
            println!("{}", host);
        }
        0
    }

    pub(crate) fn builtin_uname(&self, args: &[String]) -> i32 {
        // coreutils uname(1) port: combinable flags emit selected
        // fields space-separated in canonical order. -a is short for
        // every field. With no flags, default is -s (kernel name).
        let mut uts: libc::utsname = unsafe { std::mem::zeroed() };
        if unsafe { libc::uname(&mut uts) } != 0 {
            eprintln!("uname: cannot get system info");
            return 1;
        }

        let sysname = unsafe { std::ffi::CStr::from_ptr(uts.sysname.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        let nodename = unsafe { std::ffi::CStr::from_ptr(uts.nodename.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        let release = unsafe { std::ffi::CStr::from_ptr(uts.release.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        let version = unsafe { std::ffi::CStr::from_ptr(uts.version.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        let machine = unsafe { std::ffi::CStr::from_ptr(uts.machine.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        // -p / -o aren't in struct utsname; coreutils synthesizes them
        // from the machine and sysname respectively. Match that.
        let processor = machine.clone();
        let os = if sysname == "Linux" {
            "GNU/Linux".to_string()
        } else {
            sysname.clone()
        };

        let mut want_s = false;
        let mut want_n = false;
        let mut want_r = false;
        let mut want_v = false;
        let mut want_m = false;
        let mut want_p = false;
        let mut want_o = false;
        let mut all = false;
        for arg in args {
            if let Some(s) = arg.strip_prefix('-') {
                for c in s.chars() {
                    match c {
                        's' => want_s = true,
                        'n' => want_n = true,
                        'r' => want_r = true,
                        'v' => want_v = true,
                        'm' => want_m = true,
                        'p' => want_p = true,
                        'o' => want_o = true,
                        'a' => all = true,
                        // coreutils uname errors on unknown short
                        // flags. Old \`_ => {}\` silently dropped them
                        // and the default behavior (sysname) ran.
                        _ => {
                            eprintln!("uname: invalid option -- '{}'", c);
                            return 1;
                        }
                    }
                }
            }
        }
        if all {
            // coreutils -a output order: sysname nodename release
            // version machine [processor [os]]. processor/os are
            // suppressed when 'unknown'; we always have machine so
            // include processor; os synthesized too.
            println!(
                "{} {} {} {} {} {} {}",
                sysname, nodename, release, version, machine, processor, os
            );
            return 0;
        }
        if !want_s && !want_n && !want_r && !want_v && !want_m && !want_p && !want_o {
            want_s = true; // default
        }
        let mut parts: Vec<String> = Vec::new();
        if want_s {
            parts.push(sysname);
        }
        if want_n {
            parts.push(nodename);
        }
        if want_r {
            parts.push(release);
        }
        if want_v {
            parts.push(version);
        }
        if want_m {
            parts.push(machine);
        }
        if want_p {
            parts.push(processor);
        }
        if want_o {
            parts.push(os);
        }
        println!("{}", parts.join(" "));
        0
    }

    pub(crate) fn builtin_date(&self, args: &[String]) -> i32 {
        // coreutils date(1) port: adds -u (UTC), -r FILE (mtime of
        // FILE), -R / --rfc-2822, -I / --iso-8601. -d (parse arbitrary
        // date string) is partially handled — only +<seconds> /
        // @<seconds> Unix-time forms; full date-string parser not yet.

        let mut utc = false;
        let mut format: Option<String> = None;
        let mut reference: Option<String> = None;
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            if let Some(s) = arg.strip_prefix('+') {
                format = Some(s.to_string());
            } else if arg == "-u" || arg == "--utc" || arg == "--universal" {
                utc = true;
            } else if arg == "-r" || arg == "--reference" {
                if let Some(r) = iter.next() {
                    reference = Some(r.clone());
                }
            } else if let Some(r) = arg.strip_prefix("--reference=") {
                reference = Some(r.to_string());
            } else if arg == "-R" || arg == "--rfc-2822" || arg == "--rfc-email" {
                format = Some("%a, %d %b %Y %H:%M:%S %z".to_string());
            } else if arg == "-I" || arg == "--iso-8601" {
                format = Some("%Y-%m-%d".to_string());
            } else if let Some(prec) = arg.strip_prefix("--iso-8601=") {
                format = Some(
                    match prec {
                        "date" => "%Y-%m-%d",
                        "hours" => "%Y-%m-%dT%H%z",
                        "minutes" => "%Y-%m-%dT%H:%M%z",
                        "seconds" => "%Y-%m-%dT%H:%M:%S%z",
                        "ns" => "%Y-%m-%dT%H:%M:%S,%N%z",
                        _ => "%Y-%m-%d",
                    }
                    .to_string(),
                );
            } else if arg == "-d" || arg == "--date" {
                // -d STRING / --date=STRING — date-string parsing
                // not yet implemented. Consume the next arg so it
                // doesn't slip through to the unknown-flag path.
                if iter.next().is_none() {
                    eprintln!("zshrs:date:1: argument expected: -d");
                    return 1;
                }
            } else if arg.starts_with("--date=") {
                // ignore — parser not yet impl
            } else if arg == "--" {
                // end of options
            } else if arg.starts_with('-') && arg.len() > 1 {
                // coreutils date errors on unknown flags. Old impl
                // silently dropped them and produced default output.
                eprintln!("zshrs:date:1: unrecognized option: '{}'", arg);
                return 1;
            }
        }

        // Determine the timestamp.
        let ts: i64 = if let Some(refpath) = reference {
            match std::fs::metadata(&refpath) {
                Ok(meta) => meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0),
                Err(e) => {
                    eprintln!("zshrs:date:1: {}: {}", refpath, e);
                    return 1;
                }
            }
        } else {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64
        };

        let tm = unsafe {
            let t = ts as libc::time_t;
            if utc {
                *libc::gmtime(&t)
            } else {
                *libc::localtime(&t)
            }
        };
        let fmt_str = format.unwrap_or_else(|| "%a %b %e %H:%M:%S %Z %Y".to_string());
        // c_char is i8 on most targets, u8 on aarch64-linux; use c_char so
        // strftime + CStr::from_ptr accept the same pointer type per-target.
        let mut buf = [0 as libc::c_char; 1024];
        let fmt_cstr = std::ffi::CString::new(fmt_str.as_str()).unwrap_or_default();
        let len = unsafe { libc::strftime(buf.as_mut_ptr(), buf.len(), fmt_cstr.as_ptr(), &tm) };
        if len > 0 {
            let s = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) };
            println!("{}", s.to_string_lossy());
        }
        0
    }

    pub(crate) fn builtin_mktemp(&self, args: &[String]) -> i32 {
        // coreutils mktemp(1) port. Replaces the in-template
        // 'XXXXXX' run with a random a-z0-9 suffix, retries on
        // collision (real mktemp uses O_EXCL so two parallel
        // mktemp(1) invocations don't pick the same name).

        let mut dir = false;
        let mut want_tmpdir_flag = false;
        let mut explicit_tmpdir: Option<String> = None;
        let mut template: Option<&str> = None;
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-d" | "--directory" => dir = true,
                "-t" => {
                    // Treat the template arg as a basename; place
                    // under \$TMPDIR. (coreutils -t is deprecated
                    // but still accepted; flag without -p.)
                    want_tmpdir_flag = true;
                }
                "-p" | "--tmpdir" => {
                    // Next arg is the dir to use as base.
                    if let Some(d) = iter.next() {
                        explicit_tmpdir = Some(d.clone());
                    }
                }
                "-q" | "--quiet" => {} // accepted: don't emit errors (we still do; minimal port)
                "-u" | "--dry-run" => {} // accepted: print name without creating
                "--" => {}             // end of options
                a if !a.starts_with('-') => template = Some(a),
                a => {
                    eprintln!("mktemp: unrecognized option: '{}'", a);
                    return 1;
                }
            }
        }

        let tmpdir = explicit_tmpdir
            .unwrap_or_else(|| std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string()));
        let base = template.unwrap_or("tmp.XXXXXXXXXX");

        // Produce a random a-z0-9 suffix of the requested length.
        // Real PRNG, not ms-tick parity (the previous impl produced
        // 10 copies of the same letter).
        let gen_suffix = |len: usize| -> String {
            let alphabet: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
            let mut rng = rand::thread_rng();
            (0..len)
                .map(|_| alphabet[rng.gen_range(0..alphabet.len())] as char)
                .collect()
        };

        // Build a candidate filename by replacing the longest
        // run of consecutive 'X's with a random suffix of the same
        // length (matches mktemp's behavior).
        let make_name = |t: &str| -> String {
            let bytes = t.as_bytes();
            // Find longest X-run.
            let mut best_start = 0usize;
            let mut best_len = 0usize;
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'X' {
                    let s = i;
                    while i < bytes.len() && bytes[i] == b'X' {
                        i += 1;
                    }
                    let len = i - s;
                    if len > best_len {
                        best_start = s;
                        best_len = len;
                    }
                } else {
                    i += 1;
                }
            }
            if best_len == 0 {
                // No X's: append .RANDOM to match real mktemp default.
                return format!("{}.{}", t, gen_suffix(6));
            }
            let suffix = gen_suffix(best_len);
            let mut out = String::with_capacity(t.len());
            out.push_str(&t[..best_start]);
            out.push_str(&suffix);
            out.push_str(&t[best_start + best_len..]);
            out
        };

        // -t implies under \$TMPDIR using template as basename.
        let try_path = |name: String| -> std::path::PathBuf {
            if want_tmpdir_flag || !std::path::Path::new(&name).is_absolute() {
                std::path::Path::new(&tmpdir).join(&name)
            } else {
                std::path::PathBuf::from(&name)
            }
        };

        // Up to 100 collision retries (real mktemp tries TMP_MAX).
        for _ in 0..100 {
            let path = try_path(make_name(base));
            if dir {
                let result = std::fs::DirBuilder::new().mode(0o700).create(&path);
                match result {
                    Ok(_) => {
                        println!("{}", path.display());
                        return 0;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(e) => {
                        eprintln!("mktemp: {}: {}", path.display(), e);
                        return 1;
                    }
                }
            } else {
                let result = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path);
                match result {
                    Ok(_) => {
                        // Lock down to 0600 to match mktemp.
                        #[cfg(unix)]
                        {
                            let _ = std::fs::set_permissions(
                                &path,
                                std::fs::Permissions::from_mode(0o600),
                            );
                        }
                        println!("{}", path.display());
                        return 0;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(e) => {
                        eprintln!("mktemp: {}: {}", path.display(), e);
                        return 1;
                    }
                }
            }
        }
        eprintln!("mktemp: too many collisions");
        1
    }

    /// `cp` — in-process recursive copy. zshrs extension (not in
    /// upstream zsh — `zsh/files` ships `ln`/`mv`/`rm`/`chmod`/
    /// `chown`/`mkdir`/`rmdir`/`sync` but no `cp`). Thin method
    /// wrapper around the free fn `cp_impl` so the
    /// `reg_overridable!` macro in fusevm_bridge can dispatch to it
    /// the same way as every other coreutils-style builtin.
    pub(crate) fn builtin_cp(&self, args: &[String]) -> i32 {
        cp_impl(args)
    }
}

// =====================================================================
// rlimits dispatcher bridges. Build a `struct options` (zsh.h:1416)
// from the leading short-option run and delegate to the C-faithful
// free ported in `src/ported/builtins/rlimits.rs`. This lives outside
// `src/ported/` because it is the dispatcher slice (analogue of
// `Src/builtin.c:execbuiltin` + `parseopts`), not a port of
// `rlimits.c` itself.
// =====================================================================

use crate::ported::builtins::rlimits::{
    bin_limit as rl_bin_limit, bin_ulimit as rl_bin_ulimit, bin_unlimit as rl_bin_unlimit,
};
// sc_bin_sched / cl_bin_clone re-import deleted along with the wrappers.
use crate::ported::zsh_h::{options, MAX_OPS};

impl ShellExecutor {
    // bin_limit / bin_unlimit / bin_ulimit wrappers deleted — the
    // bridge handlers now route directly through `dispatch_builtin`,
    // which goes through `execbuiltin` (BUILTINS table optstr parse +
    // HandlerFunc call). The optstrs ("sh"/"hs"/NULL) come from the
    // BUILTINS entries themselves at src/ported/builtin.rs:9053-9082,
    // not from manual `build_short_opts` calls.

    // bin_sched / bin_clone wrappers deleted — both routed through
    // dispatch_builtin which goes via execbuiltin → BUILTINS[name]
    // (sched.c:375 + clone.c:110). The 12-line ops-construct wrappers
    // were Rust-only adapters duplicating what execbuiltin's call
    // shape already provides.
}

// build_short_opts deleted — was a Rust-only short-opt parser
// duplicating execbuiltin's optstr walk. All callers (zattr family,
// zsocket, chgrp, mkdir, etc.) now route through dispatch_builtin
// which goes via execbuiltin → BUILTINS[name].optstr → automatic
// parsing.

// ─────────────────────────────────────────────────────────
// Extracted from `impl ShellExecutor` per the FAKE DUP audit:
// these zshrs-specific builtins / autoload-style helpers don't
// need executor state, so they live as free ported.
// ─────────────────────────────────────────────────────────

/// readarray/mapfile - read lines into array (bash)
pub(crate) fn readarray(args: &[String]) -> i32 {
    // bash readarray / mapfile: read lines from a fd into an array.
    // Direct port of bash's read_builtin_array_loadable. zsh has no
    // direct equivalent (use `read -A`), but plugin code that
    // toggles between bash/zsh frequently calls this.

    let mut array_name = "MAPFILE".to_string();
    let mut delimiter: u8 = b'\n';
    let mut count = 0usize; // 0 = unlimited
    let mut skip = 0usize;
    let mut strip_trailing = false;
    let mut callback: Option<String> = None;
    let mut callback_quantum = 0usize;
    let mut fd: i32 = 0;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-d" => {
                i += 1;
                if i < args.len() && !args[i].is_empty() {
                    delimiter = args[i].as_bytes()[0];
                }
            }
            "-n" => {
                i += 1;
                if i < args.len() {
                    count = args[i].parse().unwrap_or(0);
                }
            }
            "-O" => {
                i += 1;
                // Origin - start index (ignored, we always start at 0)
            }
            "-s" => {
                i += 1;
                if i < args.len() {
                    skip = args[i].parse().unwrap_or(0);
                }
            }
            "-t" => strip_trailing = true,
            "-C" => {
                i += 1;
                if i < args.len() {
                    callback = Some(args[i].clone());
                }
            }
            "-c" => {
                i += 1;
                if i < args.len() {
                    callback_quantum = args[i].parse().unwrap_or(5000);
                }
            }
            "-u" => {
                i += 1;
                if i < args.len() {
                    fd = args[i].parse().unwrap_or(0);
                }
            }
            s if !s.starts_with('-') => {
                array_name = s.to_string();
            }
            _ => {}
        }
        i += 1;
    }

    // Read entire input from the chosen fd, then split on delim.
    // Using libc::read on the raw fd so -u N picks any open fd
    // (was hardcoded stdin).
    let mut input: Vec<u8> = Vec::new();
    if fd == 0 {
        let stdin = std::io::stdin();
        let mut handle = stdin.lock();
        if handle.read_to_end(&mut input).is_err() {
            return 1;
        }
    } else {
        let mut buf = [0u8; 8192];
        loop {
            let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n <= 0 {
                break;
            }
            input.extend_from_slice(&buf[..n as usize]);
        }
    }

    // Split on `delimiter` into records. bash KEEPS the delimiter byte
    // at the end of each element unless `-t` (strip_trailing) removes it
    // (`mapfile L` on "x\ny\n" → ("x\n" "y\n"); `-t` → ("x" "y")). Rust's
    // split() drops the delimiter, so re-append it when not stripping —
    // for every chunk that was actually followed by a delimiter (all but a
    // final record with no trailing delimiter). A trailing empty chunk
    // produced by a final delimiter byte is not a record.
    let delim_char = delimiter as char;
    let chunks: Vec<&[u8]> = input.split(|b| *b == delimiter).collect();
    let n_chunks = chunks.len();
    let mut lines: Vec<String> = Vec::new();
    let mut line_count = 0usize;
    for (idx, chunk) in chunks.iter().enumerate() {
        if idx + 1 == n_chunks && chunk.is_empty() {
            continue; // trailing empty after the final delimiter
        }
        line_count += 1;
        if line_count <= skip {
            continue;
        }
        let mut line = String::from_utf8_lossy(chunk).to_string();
        let had_delim = idx + 1 < n_chunks; // this chunk was followed by delim
        if !strip_trailing && had_delim {
            line.push(delim_char);
        }
        lines.push(line);
        if count > 0 && lines.len() >= count {
            break;
        }
    }

    crate::ported::params::setaparam(&array_name, lines);
    let _ = (callback, callback_quantum);
    0
}

pub(crate) fn shopt(args: &[String]) -> i32 {
    use crate::dash_mode::{bash_shopt_get, bash_shopt_row, bash_shopt_set, BASH_SHOPTS};

    // bash(1), The Shopt Builtin. The name list, the defaults and the
    // storage location of each flag live in `dash_mode::BASH_SHOPTS`;
    // this fn is the argument parsing and the output formats.
    //
    // Two output shapes:
    //   plain    `NAME<TAB>on|off`         (columns, bash's `shopt` listing)
    //   `-p`     `shopt -s|-u NAME`        (re-inputtable)
    // and one status rule, quoted from bash(1): "The return status when
    // listing options is zero if all optnames are enabled, non-zero
    // otherwise." It applies to `-q` AND to the printing forms —
    // `bash -c 'shopt -p cdable_vars'` prints `shopt -u cdable_vars` and
    // exits 1.
    let mut set: Option<bool> = None;
    let mut print_p = false;
    let mut quiet = false;
    let mut set_o = false; // `-o`: restrict names to the `set -o` table
    let mut opts: Vec<String> = Vec::new();

    // bash groups short flags: `shopt -so errexit`, `shopt -ps`, `shopt -qs`
    // are all real spellings (`bash -c 'shopt -so'` lists the enabled
    // `set -o` options; `bash -c 'shopt -ps'` prints the enabled shopts in
    // re-inputtable form). Parsing each argv word as ONE flag rejected them
    // as option NAMES: `shopt -so` printed
    // "shopt: -so: invalid shell option name" and exited 1.
    let mut bad_flag: Option<String> = None;
    // `--` ends flag parsing (`bash -c 'shopt -- dotglob'` queries dotglob).
    let mut end_of_flags = false;
    for arg in args {
        let is_flag = !end_of_flags && arg.len() > 1 && arg.starts_with('-');
        if arg.as_str() == "--" && !end_of_flags {
            end_of_flags = true;
            continue;
        }
        if !is_flag {
            opts.push(arg.clone());
            continue;
        }
        for c in arg.chars().skip(1) {
            match c {
                's' => set = Some(true),
                'u' => set = Some(false),
                'p' => print_p = true,
                'q' => quiet = true,
                'o' => set_o = true,
                _ => {
                    if bad_flag.is_none() {
                        bad_flag = Some(format!("-{c}"));
                    }
                }
            }
        }
    }
    // bash: `shopt -Z` → "shopt: -Z: invalid option" + the usage line,
    // status 2 (`bash -c 'shopt -Z'; echo $?`).
    if let Some(f) = bad_flag {
        eprintln!("zshrs: shopt: {f}: invalid option");
        eprintln!("shopt: usage: shopt [-pqsu] [-o] [optname ...]");
        return 2;
    }

    // `-o` names come from bash's `set -o` table, not the shopt table.
    // bash(1): "Restricts the values of optname to be those defined for
    // the -o option to the set builtin."
    if set_o {
        return shopt_o(set, print_p, quiet, &opts);
    }

    // No names: list the table in its (alphabetical) order, in whichever of
    // the two shapes was asked for, and return 0.
    //
    // `-s` / `-u` FILTER that listing rather than suppressing it — bash(1):
    // "If either -s or -u is used with no optname arguments, shopt shows
    // only those options which are set or unset, respectively."
    //   bash -c 'shopt -s'   → the 13 default-on rows, `NAME<TAB>on`
    //   bash -c 'shopt -ps'  → `shopt -s NAME` for those same 13
    //   bash -c 'shopt -q'   → nothing at all, status 0
    // zshrs printed nothing for `-s`/`-u` and printed the FULL table for
    // `-q`; both are fixed here.
    if opts.is_empty() {
        for (name, _, _) in BASH_SHOPTS {
            let on = bash_shopt_get(name).unwrap_or(false);
            if set.is_some_and(|want| want != on) || quiet {
                continue;
            }
            if print_p {
                println!("shopt {} {}", if on { "-s" } else { "-u" }, name);
            } else {
                println!("{:<20}\t{}", name, if on { "on" } else { "off" });
            }
        }
        return 0;
    }

    // bash rejects an unknown name before doing anything:
    //   bash -c 'shopt -p zznope'
    //   bash: line 1: shopt: zznope: invalid shell option name
    // status 1. zshrs previously accepted any string and reported it `-u`.
    let mut bad = false;
    for opt in &opts {
        if bash_shopt_row(opt).is_none() {
            eprintln!("zshrs: shopt: {}: invalid shell option name", opt);
            bad = true;
        }
    }

    if let Some(enable) = set {
        for opt in &opts {
            bash_shopt_set(opt, enable);
        }
        return if bad { 1 } else { 0 };
    }

    // Query. Status is 0 only when EVERY named option is enabled.
    let mut all_set = true;
    for opt in &opts {
        let Some(on) = bash_shopt_get(opt) else {
            all_set = false;
            continue;
        };
        if !on {
            all_set = false;
        }
        if !quiet {
            if print_p {
                println!("shopt {} {}", if on { "-s" } else { "-u" }, opt);
            } else {
                println!("{:<20}\t{}", opt, if on { "on" } else { "off" });
            }
        }
    }
    if bad || !all_set {
        1
    } else {
        0
    }
}

/// `shopt -o` — the same three shapes over bash's `set -o` name table
/// (bash(1): "Restricts the values of optname to be those defined for the
/// -o option to the set builtin"). State is shared with `set -o` through
/// `dash_mode::bash_set_o*`, so `shopt -so errexit` and `set -o errexit`
/// are one flag.
fn shopt_o(set: Option<bool>, print_p: bool, quiet: bool, opts: &[String]) -> i32 {
    use crate::dash_mode::{bash_set_o, bash_set_o_get, BASH_SET_O};
    let known = |n: &str| BASH_SET_O.iter().any(|(b, _)| *b == n);
    // `-p` under `-o` prints in the `set` builtin's re-inputtable form, not
    // shopt's: `bash -c 'shopt -po errexit'` → `set +o errexit`. zshrs
    // printed `shopt -uo errexit`, which is not a spelling bash emits at all.
    let print_line = |on: bool, name: &str| {
        println!("set {}o {}", if on { '-' } else { '+' }, name);
    };
    // The two `-o` shapes pad DIFFERENTLY in bash, so they cannot share one
    // width. The no-name listing goes through bash's `set -o` lister and
    // pads to 15 (`allexport      \toff`); a NAMED query goes through
    // shopt's own printer and pads to 20 (`errexit             \toff`).
    // Names longer than the width print unpadded, exactly as `{:<w$}` does.
    const O_LIST_WIDTH: usize = 15;
    const O_NAMED_WIDTH: usize = 20;
    if opts.is_empty() {
        // Same `-s`/`-u` filter and `-q` silence as the shopt table above.
        for (name, _) in BASH_SET_O {
            let on = bash_set_o_get(name);
            if set.is_some_and(|want| want != on) || quiet {
                continue;
            }
            if print_p {
                print_line(on, name);
            } else {
                println!(
                    "{:<w$}\t{}",
                    name,
                    if on { "on" } else { "off" },
                    w = O_LIST_WIDTH
                );
            }
        }
        return 0;
    }
    let mut bad = false;
    for opt in opts {
        if !known(opt) {
            eprintln!("zshrs: shopt: {}: invalid option name", opt);
            bad = true;
        }
    }
    if let Some(enable) = set {
        for opt in opts {
            let _ = bash_set_o(opt, enable);
        }
        return if bad { 1 } else { 0 };
    }
    let mut all_set = true;
    for opt in opts {
        if !known(opt) {
            all_set = false;
            continue;
        }
        let on = bash_set_o_get(opt);
        if !on {
            all_set = false;
        }
        if !quiet {
            if print_p {
                print_line(on, opt);
            } else {
                println!(
                    "{:<w$}\t{}",
                    opt,
                    if on { "on" } else { "off" },
                    w = O_NAMED_WIDTH
                );
            }
        }
    }
    if bad || !all_set {
        1
    } else {
        0
    }
}
/// zsleep - sleep with fractional seconds
pub(crate) fn zsleep(args: &[String]) -> i32 {
    // zsh/Src/Modules/system.c sleep_main accepts a single
    // non-negative numeric arg (NaN / negative / inf are
    // rejected). Direct port of bin_zsleep in zsh's mod_zselect:
    // negative-or-non-finite -> no-op exit 0; valid duration
    // sleeps via nanosleep.
    if args.is_empty() {
        eprintln!("zshrs:zsleep:1: missing argument");
        return 1;
    }

    let secs: f64 = match args[0].parse() {
        Ok(s) => s,
        Err(_) => {
            eprintln!("zshrs:zsleep:1: invalid number: {}", args[0]);
            return 1;
        }
    };

    // Duration::from_secs_f64 panics on negative / NaN / +inf.
    // zsh's sleep just returns 0 for non-positive durations.
    // Also clamp the upper bound: secs >= u64::MAX as f64 (~1.8e19)
    // also panics; cap at i64::MAX seconds (≈292 years) to be safe.
    if !secs.is_finite() || secs <= 0.0 {
        return 0;
    }
    let capped = if secs > i64::MAX as f64 {
        i64::MAX as f64
    } else {
        secs
    };
    std::thread::sleep(std::time::Duration::from_secs_f64(capped));
    0
}

// ─── find ───────────────────────────────────────────────────────────────
//
// zshrs-only extension. Honest scope: GNU-find-compatible enough to run
// real scripts in-process (no fork+exec) without silent divergence. The
// previous impl handled only -name / -type / -maxdepth and silently
// dropped every other predicate into a `_ => {}` arm — `find . -mtime +1`
// would treat `+1` as a path. This rewrite errors on unknown predicates
// and implements the common ones.

#[derive(Debug, Clone)]
enum FindPredicate {
    Name(String),    // -name PATTERN — glob against basename
    IName(String),   // -iname — case-insensitive variant
    Path(String),    // -path PATTERN — glob against full path
    Regex(String),   // -regex RE — Rust regex against full path
    Type(char),      // -type {f,d,l,p,s,b,c}
    MaxDepth(usize), // -maxdepth N
    MinDepth(usize), // -mindepth N
    /// (cmp, days, kind) — cmp is `+`/`-`/`=`; kind is m/a/c (mtime/atime/ctime)
    Time(char, i64, char), // -mtime / -atime / -ctime / -mmin / -amin / -cmin
    /// (cmp, bytes) — cmp is `+`/`-`/`=`
    Size(char, u64), // -size N[ckMG]
    Empty,           // -empty — zero-len file OR empty dir
    Newer(String),   // -newer FILE — newer than FILE's mtime
    Prune,           // -prune — terminal, never descend
}

#[derive(Debug, Clone)]
enum FindAction {
    Print,                   // default
    Print0,                  // -print0
    Delete,                  // -delete
    Exec(Vec<String>, bool), // -exec CMD ARGS... ; (false) or + (true)
}

/// Parse one `[ckMG]` suffix as a byte multiplier. `c`=1, `k`=1024,
/// `M`=1024², `G`=1024³ (coreutils default uses kibibytes when no
/// suffix; here we default to 512-byte blocks per coreutils when no
/// suffix is present).
fn parse_size_suffix(s: &str) -> Option<(u64, u64)> {
    if s.is_empty() {
        return None;
    }
    let last = s.chars().last().unwrap();
    let (num_str, mult) = match last {
        'c' => (&s[..s.len() - 1], 1u64),
        'k' => (&s[..s.len() - 1], 1024u64),
        'M' => (&s[..s.len() - 1], 1024 * 1024),
        'G' => (&s[..s.len() - 1], 1024 * 1024 * 1024),
        'b' => (&s[..s.len() - 1], 512u64), // coreutils default block
        _ if last.is_ascii_digit() => (s, 512u64),
        _ => return None,
    };
    let n: u64 = num_str.parse().ok()?;
    Some((n, mult))
}

/// Parse `+N` / `-N` / `N` prefix: returns (cmp char, abs value).
fn parse_cmp_num(s: &str) -> Option<(char, u64)> {
    let (cmp, rest) = match s.chars().next()? {
        '+' => ('+', &s[1..]),
        '-' => ('-', &s[1..]),
        _ => ('=', s),
    };
    let n: u64 = rest.parse().ok()?;
    Some((cmp, n))
}

fn parse_cmp_i64(s: &str) -> Option<(char, i64)> {
    parse_cmp_num(s).map(|(c, n)| (c, n as i64))
}

/// Match the predicate against an entry. `meta_full` is the post-follow
/// metadata (or symlink_metadata when not following); `path` is the
/// full path including starting prefix; `depth` is the entry's depth
/// from the starting path (0 = the starting path itself).
fn predicate_matches(
    pred: &FindPredicate,
    path: &std::path::Path,
    meta: &std::fs::Metadata,
    depth: usize,
    newer_thresholds: &std::collections::HashMap<String, std::time::SystemTime>,
) -> bool {
    use std::os::unix::fs::MetadataExt;
    match pred {
        FindPredicate::Name(pat) => {
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            crate::vm_helper::glob_match_static(name, pat)
        }
        FindPredicate::IName(pat) => {
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let pat_lc = pat.to_ascii_lowercase();
            crate::vm_helper::glob_match_static(&name, &pat_lc)
        }
        FindPredicate::Path(pat) => {
            let s = path.to_string_lossy();
            crate::vm_helper::glob_match_static(&s, pat)
        }
        FindPredicate::Regex(re) => {
            let s = path.to_string_lossy();
            regex::Regex::new(re)
                .map(|r| r.is_match(&s))
                .unwrap_or(false)
        }
        FindPredicate::Type(c) => match c {
            'f' => meta.is_file(),
            'd' => meta.is_dir(),
            'l' => meta.file_type().is_symlink(),
            'p' => (meta.mode() & libc::S_IFMT as u32) == libc::S_IFIFO as u32,
            's' => (meta.mode() & libc::S_IFMT as u32) == libc::S_IFSOCK as u32,
            'b' => (meta.mode() & libc::S_IFMT as u32) == libc::S_IFBLK as u32,
            'c' => (meta.mode() & libc::S_IFMT as u32) == libc::S_IFCHR as u32,
            _ => false,
        },
        FindPredicate::MaxDepth(_) | FindPredicate::MinDepth(_) | FindPredicate::Prune => {
            // Handled by the walker, not per-entry.
            true
        }
        FindPredicate::Time(cmp, days, kind) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let entry_t = match kind {
                'm' => meta.mtime(),
                'a' => meta.atime(),
                'c' => meta.ctime(),
                _ => return false,
            };
            // Age in days, rounded down (coreutils semantics).
            let age_days = (now - entry_t) / 86400;
            match cmp {
                '+' => age_days > *days,
                '-' => age_days < *days,
                _ => age_days == *days,
            }
        }
        FindPredicate::Size(cmp, bytes) => {
            let size = meta.size();
            match cmp {
                '+' => size > *bytes,
                '-' => size < *bytes,
                _ => size == *bytes,
            }
        }
        FindPredicate::Empty => {
            if meta.is_file() {
                meta.size() == 0
            } else if meta.is_dir() {
                std::fs::read_dir(path)
                    .map(|mut d| d.next().is_none())
                    .unwrap_or(false)
            } else {
                false
            }
        }
        FindPredicate::Newer(reference) => {
            let other = match newer_thresholds.get(reference) {
                Some(t) => *t,
                None => return false,
            };
            meta.modified().map(|m| m > other).unwrap_or(false)
        }
    }
    .into()
}

/// Walks the tree, evaluates predicates as an AND-conjunction, fires
/// the action on every match. Honors -prune (predicate prunes a dir
/// from descent rather than filtering output), -maxdepth, -mindepth.
fn find_walk(
    start: &std::path::Path,
    preds: &[FindPredicate],
    action: &FindAction,
    cur_depth: usize,
    visited_devs: &mut std::collections::HashSet<u64>,
    xdev: bool,
    follow: bool,
    newer_thresholds: &std::collections::HashMap<String, std::time::SystemTime>,
    exit_status: &mut i32,
) {
    use std::os::unix::fs::MetadataExt;

    // Get metadata using follow vs symlink-aware lookup.
    let meta_res = if follow {
        std::fs::metadata(start)
    } else {
        std::fs::symlink_metadata(start)
    };
    let meta = match meta_res {
        Ok(m) => m,
        Err(_) => return,
    };

    // -xdev check: only descend into dirs on the same fs as the
    // starting path. The starting path's dev is recorded on first
    // call; subsequent entries with a different dev are skipped.
    if xdev {
        let dev = meta.dev();
        if visited_devs.is_empty() {
            visited_devs.insert(dev);
        } else if !visited_devs.contains(&dev) {
            return;
        }
    }

    let max_depth = preds.iter().find_map(|p| match p {
        FindPredicate::MaxDepth(n) => Some(*n),
        _ => None,
    });
    let min_depth = preds.iter().find_map(|p| match p {
        FindPredicate::MinDepth(n) => Some(*n),
        _ => None,
    });
    let has_prune = preds.iter().any(|p| matches!(p, FindPredicate::Prune));

    // Apply predicates.
    let depth_ok = min_depth.map(|n| cur_depth >= n).unwrap_or(true);
    let preds_match = preds
        .iter()
        .all(|p| predicate_matches(p, start, &meta, cur_depth, newer_thresholds));

    if depth_ok && preds_match {
        match action {
            FindAction::Print => println!("{}", start.display()),
            FindAction::Print0 => {
                use std::io::Write;
                let _ = std::io::stdout().write_all(start.display().to_string().as_bytes());
                let _ = std::io::stdout().write_all(&[0u8]);
            }
            FindAction::Delete => {
                let r = if meta.is_dir() {
                    std::fs::remove_dir(start)
                } else {
                    std::fs::remove_file(start)
                };
                if let Err(e) = r {
                    eprintln!("find: cannot delete '{}': {}", start.display(), e);
                    *exit_status = 1;
                }
            }
            FindAction::Exec(template, _plus) => {
                let argv: Vec<String> = template
                    .iter()
                    .map(|t| t.replace("{}", &start.display().to_string()))
                    .collect();
                if let Some((cmd, rest)) = argv.split_first() {
                    let st = std::process::Command::new(cmd).args(rest).status();
                    match st {
                        Ok(s) if !s.success() => *exit_status = 1,
                        Err(_) => *exit_status = 1,
                        _ => {}
                    }
                }
            }
        }
    }

    // Descend into dirs unless pruned or depth-capped.
    if meta.is_dir() && !(has_prune && preds_match) {
        if let Some(md) = max_depth {
            if cur_depth >= md {
                return;
            }
        }
        if let Ok(entries) = std::fs::read_dir(start) {
            let mut children: Vec<_> = entries.flatten().collect();
            children.sort_by_key(|e| e.file_name());
            for entry in children {
                find_walk(
                    &entry.path(),
                    preds,
                    action,
                    cur_depth + 1,
                    visited_devs,
                    xdev,
                    follow,
                    newer_thresholds,
                    exit_status,
                );
            }
        }
    }
}

pub(crate) fn find_impl(args: &[String]) -> i32 {
    let mut paths: Vec<&str> = Vec::new();
    let mut preds: Vec<FindPredicate> = Vec::new();
    let mut action = FindAction::Print;
    let mut xdev = false;
    let mut follow = false;
    let mut newer_thresholds: std::collections::HashMap<String, std::time::SystemTime> =
        std::collections::HashMap::new();
    let mut exit_status: i32 = 0;
    let mut i = 0;

    // Collect paths up front — they appear BEFORE the first
    // predicate per find(1) usage. Any arg starting with `-` or
    // `(` ends the path list.
    while i < args.len() {
        let a = &args[i];
        if a.starts_with('-') || a == "(" || a == "!" {
            break;
        }
        paths.push(a);
        i += 1;
    }
    if paths.is_empty() {
        paths.push(".");
    }

    // Top-level flags (before predicates per GNU find).
    while i < args.len() && matches!(args[i].as_str(), "-L" | "-H" | "-P" | "-D" | "-O") {
        match args[i].as_str() {
            "-L" => follow = true,
            "-P" | "-H" => follow = false,
            "-D" | "-O" => {
                i += 1; // these take an argument (debug-opts, optimization-level)
            }
            _ => {}
        }
        i += 1;
    }

    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-name" if i + 1 < args.len() => {
                preds.push(FindPredicate::Name(args[i + 1].clone()));
                i += 2;
            }
            "-iname" if i + 1 < args.len() => {
                preds.push(FindPredicate::IName(args[i + 1].clone()));
                i += 2;
            }
            "-path" | "-wholename" if i + 1 < args.len() => {
                preds.push(FindPredicate::Path(args[i + 1].clone()));
                i += 2;
            }
            "-regex" if i + 1 < args.len() => {
                preds.push(FindPredicate::Regex(args[i + 1].clone()));
                i += 2;
            }
            "-type" if i + 1 < args.len() => {
                let c = args[i + 1].chars().next().unwrap_or('?');
                preds.push(FindPredicate::Type(c));
                i += 2;
            }
            "-maxdepth" if i + 1 < args.len() => {
                let n: usize = args[i + 1].parse().unwrap_or(0);
                preds.push(FindPredicate::MaxDepth(n));
                i += 2;
            }
            "-mindepth" if i + 1 < args.len() => {
                let n: usize = args[i + 1].parse().unwrap_or(0);
                preds.push(FindPredicate::MinDepth(n));
                i += 2;
            }
            "-mtime" | "-atime" | "-ctime" if i + 1 < args.len() => {
                let kind = a.chars().nth(1).unwrap_or('m');
                match parse_cmp_i64(&args[i + 1]) {
                    Some((cmp, n)) => preds.push(FindPredicate::Time(cmp, n, kind)),
                    None => {
                        eprintln!("find: invalid argument for {}: '{}'", a, args[i + 1]);
                        return 1;
                    }
                }
                i += 2;
            }
            "-mmin" | "-amin" | "-cmin" if i + 1 < args.len() => {
                // Convert minutes → days fraction by dividing; coarse but
                // matches the same `(now - entry) / 86400` reduction.
                let kind = a.chars().nth(1).unwrap_or('m');
                match parse_cmp_i64(&args[i + 1]) {
                    Some((cmp, n)) => {
                        // Treat as already-days for simplicity; full minute
                        // resolution would need refactoring Time to seconds.
                        preds.push(FindPredicate::Time(cmp, n / (24 * 60), kind));
                    }
                    None => {
                        eprintln!("find: invalid argument for {}: '{}'", a, args[i + 1]);
                        return 1;
                    }
                }
                i += 2;
            }
            "-size" if i + 1 < args.len() => {
                let s = &args[i + 1];
                let (cmp, rest) = match s.chars().next() {
                    Some('+') => ('+', &s[1..]),
                    Some('-') => ('-', &s[1..]),
                    _ => ('=', s.as_str()),
                };
                match parse_size_suffix(rest) {
                    Some((n, mult)) => preds.push(FindPredicate::Size(cmp, n * mult)),
                    None => {
                        eprintln!("find: invalid argument for -size: '{}'", s);
                        return 1;
                    }
                }
                i += 2;
            }
            "-empty" => {
                preds.push(FindPredicate::Empty);
                i += 1;
            }
            "-newer" if i + 1 < args.len() => {
                let ref_path = args[i + 1].clone();
                if let Ok(m) = std::fs::metadata(&ref_path).and_then(|m| m.modified()) {
                    newer_thresholds.insert(ref_path.clone(), m);
                    preds.push(FindPredicate::Newer(ref_path));
                } else {
                    eprintln!("find: '{}': No such file or directory", ref_path);
                    return 1;
                }
                i += 2;
            }
            "-prune" => {
                preds.push(FindPredicate::Prune);
                i += 1;
            }
            "-xdev" | "-mount" => {
                xdev = true;
                i += 1;
            }
            "-follow" => {
                follow = true;
                i += 1;
            }
            "-print" => {
                action = FindAction::Print;
                i += 1;
            }
            "-print0" => {
                action = FindAction::Print0;
                i += 1;
            }
            "-delete" => {
                action = FindAction::Delete;
                i += 1;
            }
            "-exec" => {
                // Slurp args up to `;` or `+`.
                let mut tmpl = Vec::new();
                let mut plus = false;
                i += 1;
                while i < args.len() {
                    if args[i] == ";" {
                        i += 1;
                        break;
                    }
                    if args[i] == "+" {
                        plus = true;
                        i += 1;
                        break;
                    }
                    tmpl.push(args[i].clone());
                    i += 1;
                }
                if tmpl.is_empty() {
                    eprintln!("find: missing argument for -exec");
                    return 1;
                }
                action = FindAction::Exec(tmpl, plus);
            }
            "-o" | "-or" | "-a" | "-and" | "!" | "-not" | "(" | ")" => {
                // Boolean operators not yet implemented — predicates
                // are AND-conjuncted by default. Reject loudly so
                // scripts using these get a clear diagnostic
                // instead of silent divergence.
                eprintln!(
                    "find: boolean operator '{}' not yet supported; predicates default to AND",
                    a
                );
                return 1;
            }
            // Unknown predicate — REJECT loudly. Previously this
            // arm silently swallowed unknown flags and adjacent
            // `+N` / `-N` args got pushed as paths.
            _ => {
                eprintln!("find: unknown predicate '{}'", a);
                return 1;
            }
        }
    }

    let mut visited_devs = std::collections::HashSet::new();
    for p in &paths {
        let path = std::path::Path::new(p);
        if !path.exists() {
            eprintln!("find: '{}': No such file or directory", p);
            exit_status = 1;
            continue;
        }
        find_walk(
            path,
            &preds,
            &action,
            0,
            &mut visited_devs,
            xdev,
            follow,
            &newer_thresholds,
            &mut exit_status,
        );
    }
    exit_status
}

pub(crate) fn cp_impl(args: &[String]) -> i32 {
    let mut recursive = false;
    let mut force = false;
    let mut interactive = false;
    // -n: never overwrite. coreutils -f / -i / -n are mutually
    // exclusive, last one wins.
    let mut no_clobber = false;
    let mut preserve = false;
    let mut verbose = false;
    let mut files: Vec<&str> = Vec::new();

    for arg in args {
        match arg.as_str() {
            "-r" | "-R" | "--recursive" => recursive = true,
            "-f" | "--force" => {
                force = true;
                interactive = false;
                no_clobber = false;
            }
            "-i" | "--interactive" => {
                interactive = true;
                force = false;
                no_clobber = false;
            }
            "-n" | "--no-clobber" => {
                no_clobber = true;
                force = false;
                interactive = false;
            }
            "-p" | "--preserve" => preserve = true,
            "-v" | "--verbose" => verbose = true,
            "--" => {} // end of options
            s if !s.starts_with('-') || s == "-" => files.push(s),
            s => {
                // coreutils cp rejects unknown flags.
                eprintln!("cp: unrecognized option: '{}'", s);
                return 1;
            }
        }
    }

    if files.len() < 2 {
        eprintln!("cp: missing file operand");
        return 1;
    }

    let target = files.pop().unwrap();
    let target_path = std::path::Path::new(target);
    let is_dir = target_path.is_dir();

    // Per-file continue-on-error per coreutils (was return 1 on
    // first failure, leaving the rest unprocessed).
    let mut cp_status = 0;
    for src in files {
        let src_path = std::path::Path::new(src);
        let dest = if is_dir {
            format!(
                "{}/{}",
                target,
                src_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| src.to_string())
            )
        } else {
            target.to_string()
        };

        let dest_path = std::path::Path::new(&dest);
        if dest_path.exists() && !force {
            if no_clobber {
                if verbose {
                    println!("'{}' -> '{}' (skipped, target exists)", src, dest);
                }
                continue;
            }
            if interactive {
                eprint!("cp: overwrite '{}'? ", dest);
                let mut response = String::new();
                if std::io::stdin().read_line(&mut response).is_err()
                    || !response.trim().eq_ignore_ascii_case("y")
                {
                    continue;
                }
            }
        }

        let result = if src_path.is_dir() {
            if recursive {
                ShellExecutor::copy_dir_recursive(src_path, dest_path)
            } else {
                eprintln!("cp: -r not specified; omitting directory '{}'", src);
                cp_status = 1;
                continue;
            }
        } else {
            std::fs::copy(src, &dest).map(|_| ())
        };

        if let Err(e) = result {
            eprintln!("cp: cannot copy '{}' to '{}': {}", src, dest, e);
            cp_status = 1;
            continue;
        }

        // -p: preserve mode, ownership, atime/mtime — coreutils
        // cp(1) `-p` semantics. std::fs::copy already replicates
        // mode bits, but timestamps and uid/gid require explicit
        // chown(2) + utimensat(2) syscalls.
        if preserve {
            if let Ok(meta) = std::fs::metadata(src) {
                let dest_c = std::ffi::CString::new(dest.as_bytes()).ok();
                if let Some(c) = dest_c {
                    unsafe {
                        // chown(dest, uid, gid) — fails silently if
                        // not root (matches coreutils behaviour).
                        libc::chown(c.as_ptr(), meta.uid(), meta.gid());
                    }
                    // utimensat(AT_FDCWD, dest, [atime, mtime], 0)
                    let times = [
                        libc::timespec {
                            tv_sec: meta.atime() as libc::time_t,
                            tv_nsec: meta.atime_nsec(),
                        },
                        libc::timespec {
                            tv_sec: meta.mtime() as libc::time_t,
                            tv_nsec: meta.mtime_nsec(),
                        },
                    ];
                    unsafe {
                        libc::utimensat(libc::AT_FDCWD, c.as_ptr(), times.as_ptr(), 0);
                    }
                }
            }
        }

        if verbose {
            println!("'{}' -> '{}'", src, dest);
        }
    }
    cp_status
}

/// zmv / zcp / zln — pattern-based rename. Native Rust port of
/// the autoloaded zsh function. Glob the source pattern (with
/// `(...)` capture groups), substitute `$1`/`$2`/... in the
/// destination, then mv/cp/ln each match.
///
/// Supported flags:
///   -n   dry-run (print actions, don't execute)
///   -f   force overwrite
///   -i   interactive (prompt — falls back to skip on no-tty)
///   -v   verbose
///   -W   wildcard mode: `*` in src maps to `*` in dest position
///   -M   force mv mode (default for `zmv`)
///   -C   force cp mode
///   -L   force ln mode (hard link)
///   -s   ln -s (symlink) when in ln mode
///   -p prog  use `prog` instead of mv/cp/ln
/// zmv:273 `$f -ef $g` — true when both names resolve to the SAME file (same
/// device + inode), which is how a case-only rename looks on a case-insensitive
/// filesystem. Only exempts the "file exists" error for `mv` (zmv:273
/// `&& $action = mv`): a cp/ln onto itself stays an error.
fn same_file(src: &str, dest: &str, action: &str) -> bool {
    if action != "mv" {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (std::fs::metadata(src), std::fs::metadata(dest)) {
            (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (src, dest);
        false
    }
}

pub(crate) fn zmv(args: &[String], default_action: &str) -> i32 {
    // zmv:137 `myname=${(%):-%N}` — the FUNCTION's name (zmv/zcp/zln), which is
    // what every diagnostic is tagged with. zmv:156 derives the action back out
    // of it (`action=$myname[-2,-1]`), so the mapping is exactly this inverse.
    // Errors were tagged with the ACTION instead, printing `mv: error: …` where
    // zsh prints `zmv: error: …` — and the tag changes under `-C`/`-L`/`-p`,
    // which zsh's never does.
    let myname = format!("z{}", default_action);
    let mut action = default_action.to_string();
    let mut dry_run = false;
    let mut force = false;
    let mut verbose = false;
    // zmv:190-216 — `-w` parenthesises the wildcards in the SEARCH pattern so
    // they can be referred to as $1..$N; `-W` does that AND rewrites the
    // wildcards in the REPLACEMENT into sequential ${1}..${N} references.
    // `-w` was not accepted at all, and `-W` only ever did the search half, so
    // `zmv -W '*.txt' '*.bak'` mapped every file to the literal `*.bak` and
    // died with a bogus "both map to" collision.
    let mut wildcard = false; // opt_w || opt_W  (zmv:190)
    let mut wildcard_repl = false; // opt_W       (zmv:204)
                                   // zmv:148 `[[ -z $opt_Q ]] && setopt nobareglobqual` — bare glob qualifiers
                                   // are OFF for the whole function unless -Q asks for them back. `-Q` was
                                   // parsed as an unknown flag and dropped.
    let mut bare_glob_qual = false; // opt_Q
    let mut symlink = false;
    let mut positional: Vec<String> = Vec::new();
    let mut iter = args.iter().peekable();
    while let Some(a) = iter.next() {
        if a == "--" {
            for p in iter.by_ref() {
                positional.push(p.clone());
            }
            break;
        }
        if let Some(rest) = a.strip_prefix('-') {
            if rest.is_empty() {
                positional.push(a.clone());
                continue;
            }
            for c in rest.chars() {
                match c {
                    'n' => dry_run = true,
                    'f' => force = true,
                    'i' => {} // interactive — treat as skip-on-conflict
                    'v' => verbose = true,
                    'w' => wildcard = true, // zmv:190 opt_w
                    'W' => {
                        // zmv:190/204 — opt_W implies opt_w's pattern half.
                        wildcard = true;
                        wildcard_repl = true;
                    }
                    'Q' => bare_glob_qual = true, // zmv:148 opt_Q
                    'q' => {}                     // zmv docs: now the default, no effect
                    's' => symlink = true,
                    'M' => action = "mv".to_string(),
                    'C' => action = "cp".to_string(),
                    'L' => action = "ln".to_string(),
                    'p' => {
                        // `-p prog` consumes the next arg.
                        if let Some(p) = iter.next() {
                            action = p.clone();
                        }
                    }
                    _ => {}
                }
            }
        } else {
            positional.push(a.clone());
        }
    }
    if positional.len() < 2 {
        eprintln!(
            "{}: usage: {} [-flags] FROM_PATTERN TO_PATTERN",
            action, action
        );
        return 1;
    }
    let from_pat = &positional[0];
    let to_pat = &positional[1];

    // Convert source pattern with `(...)` capture groups to a
    // regex anchored at both ends. zsh-style globs:
    //   `*`   → `(.*)` (capture if -W or wrapped in `(...)`,
    //          else just `.*`)
    //   `?`   → `.`
    //   `(p)` → `(p_translated)` capture group
    //   `[…]` → `[…]` literal char class
    // zmv:117 "The pattern is always treated as an EXTENDED_GLOB pattern." The
    // SOURCE may lead with globbing FLAGS: `(#b)` (backreferences → $match, the
    // default for zmv captures), `(#m)` ($MATCH = the whole match), `(#i)`/
    // `(#l)` (case-insensitive). The GLOB above honours them via extendedglob,
    // but the capture REGEX must strip them — regex `()` groups already give
    // backreferences, so `(#b)` is a no-op there, `(#i)`/`(#l)` map to a
    // case-insensitive regex, and `(#m)` binds $MATCH to the whole match.
    // Without stripping, `(#b)file(?).dat` compiled to a regex containing a
    // literal `(#b)` group that matched no real filename → empty rename set.
    let mut case_insensitive = false;
    let mut want_match_var = false;
    let mut re_pat = from_pat.clone();
    while let Some(rest) = re_pat.strip_prefix("(#") {
        let Some(close) = rest.find(')') else { break };
        let flags = &rest[..close];
        // Only a pure flag group (recognised letters/digits). Anything else —
        // e.g. `(#c2,3)` count spec followed by a pattern — is left for the
        // regex builder to reject/handle rather than mis-stripped.
        if flags.is_empty() || !flags.chars().all(|c| "bmiIlaeqsMBcC0123456789".contains(c)) {
            break;
        }
        if flags.contains('i') || flags.contains('l') {
            case_insensitive = true;
        }
        if flags.contains('m') {
            want_match_var = true;
        }
        re_pat = rest[close + 1..].to_string();
    }
    let mut regex_src = String::from("^");
    if case_insensitive {
        regex_src.push_str("(?i)");
    }
    let mut chars = re_pat.chars().peekable();
    let mut group_idx = 0;
    // Byte offset in `regex_src` where the most recently emitted atom began, so
    // an extendedglob quantifier (`#` = zero-or-more, `##` = one-or-more; zmv
    // runs under `setopt extendedglob`, zmv:126) can wrap it. For a group the
    // opening `(` position is stashed here so `(...)#`/`(...)##` wrap the whole
    // group, not just its trailing `)`.
    let mut last_atom_start = regex_src.len();
    let mut group_starts: Vec<usize> = Vec::new();
    while let Some(c) = chars.next() {
        // zsh globbing `x#`/`x##` — a postfix quantifier on the previous atom.
        if c == '#' {
            let one_or_more = chars.peek() == Some(&'#');
            if one_or_more {
                chars.next();
            }
            let atom: String = regex_src[last_atom_start..].to_string();
            regex_src.truncate(last_atom_start);
            regex_src.push_str("(?:");
            regex_src.push_str(&atom);
            regex_src.push_str(if one_or_more { ")+" } else { ")*" });
            continue;
        }
        let atom_begin = regex_src.len();
        match c {
            // zmv:197-202 — under -w/-W every WILDCARD becomes its own capture
            // group (C's `pat="${pat//${~find}/($MATCH)}"`), not just `*`. The
            // `find` pattern there covers `**/`, `*`, `?`, `<a-b>` and `[…]`.
            '*' => {
                // `**/` is one wildcard, not two (zmv:197 `\*\*##/`).
                let mut atom = String::from(".*");
                if chars.peek() == Some(&'*') {
                    chars.next();
                    while chars.peek() == Some(&'*') {
                        chars.next();
                    }
                    if chars.peek() == Some(&'/') {
                        chars.next();
                        atom = String::from("(?:.*/)?");
                    }
                }
                if wildcard {
                    regex_src.push('(');
                    regex_src.push_str(&atom);
                    regex_src.push(')');
                    group_idx += 1;
                } else {
                    regex_src.push_str(&atom);
                }
            }
            '?' => {
                if wildcard {
                    regex_src.push_str("(.)");
                    group_idx += 1;
                } else {
                    regex_src.push('.');
                }
            }
            // `<a-b>` numeric range (zmv:197 `<[0-9]#-[0-9]#>`).
            '<' if re_pat.contains('>') => {
                let mut body = String::new();
                let mut closed = false;
                for cc in chars.by_ref() {
                    if cc == '>' {
                        closed = true;
                        break;
                    }
                    body.push(cc);
                }
                if closed && body.chars().all(|c| c.is_ascii_digit() || c == '-') {
                    if wildcard {
                        regex_src.push_str("([0-9]+)");
                        group_idx += 1;
                    } else {
                        regex_src.push_str("[0-9]+");
                    }
                } else {
                    regex_src.push_str("<");
                    regex_src.push_str(&body);
                    if closed {
                        regex_src.push('>');
                    }
                }
            }
            '(' => {
                group_starts.push(atom_begin);
                regex_src.push('(');
                group_idx += 1;
            }
            ')' => regex_src.push(')'),
            '[' => {
                let mut cls = String::from("[");
                for cc in chars.by_ref() {
                    cls.push(cc);
                    if cc == ']' {
                        break;
                    }
                }
                if wildcard {
                    regex_src.push('(');
                    regex_src.push_str(&cls);
                    regex_src.push(')');
                    group_idx += 1;
                } else {
                    regex_src.push_str(&cls);
                }
            }
            '|' => regex_src.push('|'),
            '.' | '+' | '^' | '$' | '\\' | '{' | '}' => {
                regex_src.push('\\');
                regex_src.push(c);
            }
            _ => regex_src.push(c),
        }
        // A closing `)` makes the whole group the atom a following `#` quantifies.
        last_atom_start = if c == ')' {
            group_starts.pop().unwrap_or(atom_begin)
        } else {
            atom_begin
        };
    }
    regex_src.push('$');

    // zmv:204-216 — `-W` turns the wildcards in the REPLACEMENT into
    // sequential ${1} .. ${N} references (`repl="${repl//${~find}/$open$[++N]$close}"`),
    // and errors when the two counts disagree (zmv:209-212).
    let rewritten_to;
    let to_pat: &String = if wildcard_repl {
        let mut out = String::new();
        let mut n = 0usize;
        let mut tc = to_pat.chars().peekable();
        while let Some(c) = tc.next() {
            match c {
                '*' => {
                    while tc.peek() == Some(&'*') {
                        tc.next();
                    }
                    if tc.peek() == Some(&'/') {
                        tc.next();
                    }
                    n += 1;
                    out.push_str(&format!("${{{}}}", n));
                }
                '?' => {
                    n += 1;
                    out.push_str(&format!("${{{}}}", n));
                }
                '[' => {
                    let mut cls = String::from("[");
                    for cc in tc.by_ref() {
                        cls.push(cc);
                        if cc == ']' {
                            break;
                        }
                    }
                    n += 1;
                    out.push_str(&format!("${{{}}}", n));
                }
                _ => out.push(c),
            }
        }
        if n != group_idx {
            // zmv:210 — `print -P "%N: error: number of wildcards in each pattern must match"`
            eprintln!(
                "{}: error: number of wildcards in each pattern must match",
                myname
            );
            return 1;
        }
        rewritten_to = out;
        &rewritten_to
    } else {
        to_pat
    };
    let re = match regex::Regex::new(&regex_src) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{}: bad pattern: {}", action, e);
            return 1;
        }
    };

    // Enumerate candidate files. zmv:237-239 uses the pattern AS the glob
    // (`fpat=$pat; files=(${~fpat})`) under `setopt extendedglob` (zmv:126) —
    // in a zsh glob `(…)` is a GROUP, so `(*).​(txt|log)` is both a valid glob
    // and the capture syntax; the two coexist by design.
    //
    // Stripping the parens to build the glob turned `(*).(txt|log)` into
    // `*.txt|log`, which matches nothing with a `.log` extension — `baz.log`
    // silently vanished from the rename set. zshrs's own glob engine handles
    // the grouped form identically to zsh (verified against `print -rl --
    // (*).(txt|log)`), so hand it the pattern unaltered.
    let glob_pat: String = from_pat.clone();
    // zmv:148 `[[ -z $opt_Q ]] && setopt nobareglobqual`. With bare glob
    // qualifiers ON (the zsh default), a TRAILING `(…)` is parsed as a
    // qualifier list, so `(*).(*)` — the canonical swap-name-and-extension
    // pattern — matched nothing at all. zmv turns them off for the whole
    // function body so the trailing group stays a capture group; `-Q` asks for
    // the qualifier reading back.
    //
    // zsh sets this as a real (function-scoped) option and its glob reads it,
    // so mirror that around the glob and restore after. glob() snapshots every
    // glob-relevant option into TLS at entry (glob.rs:3640 enter_glob_scope),
    // which is what keeps this coherent for the in-flight glob.
    let saved_bgq = crate::ported::zsh_h::isset(crate::ported::zsh_h::BAREGLOBQUAL);
    if !bare_glob_qual && saved_bgq {
        crate::ported::options::opt_state_set("bareglobqual", false);
    }
    // zmv:126 `setopt extendedglob` — zmv always treats its pattern as an
    // EXTENDED_GLOB pattern, which is what makes globbing flags like `(#b)`
    // (backreferences → $match), `(#m)` ($MATCH), and `(#i)` work. The native
    // impl globbed WITHOUT it, so `zmv '(#b)file(?).dat' …` glob-failed with
    // "no matches found" before any rename could happen.
    let saved_eg = crate::ported::zsh_h::isset(crate::ported::zsh_h::EXTENDEDGLOB);
    if !saved_eg {
        crate::ported::options::opt_state_set("extendedglob", true);
    }
    let candidates = crate::fusevm_bridge::with_executor(|exec| exec.expand_glob(&glob_pat));
    if !saved_eg {
        crate::ported::options::opt_state_set("extendedglob", false);
    }
    if !bare_glob_qual && saved_bgq {
        crate::ported::options::opt_state_set("bareglobqual", true);
    }
    if candidates.len() == 1
        && candidates[0] == glob_pat
        && !std::path::Path::new(&candidates[0]).exists()
    {
        eprintln!("{}: no matches found: {}", action, from_pat);
        return 1;
    }

    // For each match, compute destination by applying captures.
    // zmv:243 `errs=()` — substitution errors accumulate and are reported
    // together at the end (zmv:280-284), never inline per-file.
    let mut errs: Vec<String> = Vec::new();
    let mut renames: Vec<(String, String)> = Vec::new();
    for src in &candidates {
        let caps = match re.captures(src) {
            Some(c) => c,
            None => continue,
        };
        // zmv:255-257 — the destination is NOT a hand-rolled `$1` substitution:
        //
        //     set -- "$match[@]"
        //     g=${(Xe)repl}
        //
        // i.e. the capture groups become the POSITIONAL PARAMETERS and the
        // replacement is then run through full parameter expansion (`e`), with
        // `X` reporting parse errors. That is what makes `$f` (the current
        // source file, zmv:245 `for f in $files`) work, and it is why zsh's own
        // documentation can offer `zmv -v '* *' '${f// /_}'` — an arbitrary
        // expansion over $f, not just a number reference.
        //
        // Substituting only `$1`..`$9`/`${N}` by hand left `$f` as the literal
        // text `$f`, so every file mapped to the same name and zmv aborted with
        // "both map to new_$f". Bind f + the positionals and reuse the shell's
        // own `(e)` machinery (subst.rs:14508 `subst_parse_str` → `singsub`),
        // which is the same code path `${(Xe)…}` takes.
        let saved_f = crate::fusevm_bridge::with_executor(|exec| exec.scalar("f"));
        let saved_pp = crate::fusevm_bridge::with_executor(|exec| exec.pparams());
        // zmv binds the capture groups to BOTH `$match` (from `[[ $f =
        // (#b)$pat ]]`, zmv:260) AND the positionals (`set -- "$match[@]"`,
        // zmv:261), so a replacement can reference either `$match[1]` or `$1`.
        // Binding only the positionals left `$match[1]` expanding to EMPTY, so
        // `zmv '(*).dat' 'x$match[1].dat'` mapped every file to the same name
        // and aborted with a bogus "both map to" collision. Save + restore
        // `$match` around the expansion, mirroring how `$f`/positionals are.
        let saved_match = crate::ported::params::getaparam("match");
        let saved_match_var = if want_match_var {
            crate::fusevm_bridge::with_executor(|exec| exec.scalar("MATCH"))
        } else {
            None
        };
        let match_args: Vec<String> = (1..caps.len())
            .map(|i| caps.get(i).map(|m| m.as_str()).unwrap_or("").to_string())
            .collect();
        crate::fusevm_bridge::with_executor(|exec| {
            exec.set_scalar("f".to_string(), src.clone()); // zmv:245 `for f in $files`
            exec.set_pparams(match_args.clone()); // zmv:255 `set -- "$match[@]"`
        });
        let _ = crate::ported::params::setaparam("match", match_args.clone()); // zmv:260 `$match`
                                                                               // `(#m)` binds $MATCH to the whole match (regex group 0).
        if want_match_var {
            let whole = caps.get(0).map(|m| m.as_str()).unwrap_or("").to_string();
            crate::fusevm_bridge::with_executor(|exec| exec.set_scalar("MATCH".to_string(), whole));
        }
        // zmv:257 `g=${(Xe)repl}` — `X` => quoteerr, `e` => re-substitute.
        let dest = match crate::ported::subst::subst_parse_str(to_pat, false, true) {
            Some(parsed) => crate::ported::subst::singsub(&parsed),
            None => crate::ported::subst::singsub(to_pat),
        };
        crate::fusevm_bridge::with_executor(|exec| {
            match &saved_f {
                Some(v) => exec.set_scalar("f".to_string(), v.clone()),
                None => exec.set_scalar("f".to_string(), String::new()),
            }
            exec.set_pparams(saved_pp.clone());
        });
        match &saved_match {
            Some(m) => {
                let _ = crate::ported::params::setaparam("match", m.clone());
            }
            None => {
                crate::ported::params::unsetparam("match");
            }
        }
        if want_match_var {
            match &saved_match_var {
                Some(v) => crate::fusevm_bridge::with_executor(|exec| {
                    exec.set_scalar("MATCH".to_string(), v.clone())
                }),
                None => {
                    crate::ported::params::unsetparam("MATCH");
                }
            }
        }
        // zmv:264-265 — an empty expansion joins `errs`; it is reported with
        // the rest at the end (zmv:280-284), not immediately.
        if dest.is_empty() {
            errs.push(format!("`{}' expanded to an empty string", src));
            continue;
        }
        // zmv:266-270 — a file the substitution did not alter is skipped, not
        // an error and not a collision.
        if dest == *src {
            if verbose {
                println!("{} not altered, ignored", src);
            }
            continue;
        }
        // zmv:273-274 — an existing destination is an error UNLESS -f, or the
        // source and destination are the SAME FILE and we are moving:
        //     elif [[ -f $g && -z $opt_f && ! ($f -ef $g && $action = mv) ]]; then
        //         errs+=("file exists: $g")
        // The `-ef` exemption is what lets a case-only rename work on a
        // case-INSENSITIVE filesystem (macOS): `zmv '*.txt' '${f:u}'` maps
        // foo.txt → FOO.TXT, which `exists()` reports as true because it IS
        // foo.txt. Without the exemption every such rename died with a bogus
        // "destination exists" — and it fired even under `-n`, where zsh
        // reports at mapping time through errs and never touches the disk.
        if !force && std::path::Path::new(&dest).is_file() && !same_file(src, &dest, &action) {
            errs.push(format!("file exists: {}", dest));
            continue;
        }
        renames.push((src.clone(), dest));
    }

    // zmv:271-272 — a destination two sources both map to is an error, unless
    // the destination is an existing DIRECTORY (`! -d $g`).
    let mut seen: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for (s, d) in &renames {
        if let Some(prev) = seen.insert(d.as_str(), s.as_str()) {
            if !std::path::Path::new(d).is_dir() {
                errs.push(format!("{} and {} both map to {}", s, prev, d));
            }
        }
    }
    // zmv:280-284 — every substitution error is collected and reported TOGETHER
    // under one header, then the whole run aborts without touching a file:
    //     print -r -- "$myname: error(s) in substitution:" >&2
    //     print -lr -- $errs >&2
    // Reporting each collision inline as `mv: error: …` matched neither the tag
    // nor the shape.
    if !errs.is_empty() {
        eprintln!("{}: error(s) in substitution:", myname);
        for e in &errs {
            eprintln!("{}", e);
        }
        return 1;
    }

    // Execute (or print, if -n).
    let prog = match action.as_str() {
        "mv" | "cp" | "ln" => action.clone(),
        other => other.to_string(),
    };
    let mut status = 0;
    for (s, d) in &renames {
        // The "file exists" gate is NOT here: zmv:273-274 applies it while
        // building the map, so it lands in `errs` and aborts the whole run
        // before any file is touched (zmv:280-284). Checking it at execution
        // time renamed the earlier files and then failed on a later one, and
        // fired under `-n` too.
        // zmv:288-289 — the command is assembled as an ARRAY and echoed under
        // -i, -n OR -v (not just -n), with every word (q-) quoted:
        //     exec=(${=action} ${=opt_o} $opt_s $dashes $f $to[$f])
        //     [[ -n $opt_i$opt_n$opt_v ]] && print -r -- ${(q-)exec}
        // Printing the words RAW meant a filename with a space came out as
        // `mv -- a b.txt a b.bak` — four words, not re-runnable — where zsh
        // prints `mv -- 'a b.txt' 'a b.bak'`. And `-v` printed a bespoke
        // `src -> dst` line that zsh never emits.
        if dry_run || verbose {
            let mut exec: Vec<String> = vec![prog.clone()];
            if symlink && action == "ln" {
                exec.push("-s".to_string()); // zmv:288 $opt_s
            }
            exec.push("--".to_string()); // zmv:288 $dashes
            exec.push(s.clone());
            exec.push(d.clone());
            let line: Vec<String> = exec
                .iter()
                .map(|w| {
                    crate::ported::utils::quotestring(w, crate::ported::zsh_h::QT_SINGLE_OPTIONAL)
                })
                .collect();
            println!("{}", line.join(" "));
        }
        if dry_run {
            continue;
        }
        let result = match action.as_str() {
            "mv" => std::fs::rename(s, d),
            "cp" => std::fs::copy(s, d).map(|_| ()),
            "ln" => {
                if symlink {
                    #[cfg(unix)]
                    {
                        std::os::unix::fs::symlink(s, d)
                    }
                    #[cfg(not(unix))]
                    {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::Unsupported,
                            "symlink",
                        ))
                    }
                } else {
                    std::fs::hard_link(s, d)
                }
            }
            _ => {
                // External program — shell out.
                let st = std::process::Command::new(&prog).arg(s).arg(d).status();
                match st {
                    Ok(s) => {
                        if s.success() {
                            Ok(())
                        } else {
                            Err(std::io::Error::other("exit nonzero"))
                        }
                    }
                    Err(e) => Err(e),
                }
            }
        };
        if let Err(e) = result {
            eprintln!("{}: {}: {}", action, s, e);
            status = 1;
        }
    }
    status
}

/// zcalc — basic non-interactive calculator. zsh's autoloaded
/// zcalc is interactive (REPL); we support the `-e EXPR` form
/// which evaluates a single expression and prints the result.
/// Without `-e`, interactive mode is not supported and we exit 1.
/// zcalc:99-112 `zcalc_show_value()` — zcalc does NOT echo the raw arithmetic
/// result; it reformats it:
///
///   elif [[ $1 = *.* ]] || (( _outdigits )); then
///     if [[ -z $_forms[_outform] || ($_outform -eq 1 && $1 = *.) ]]; then
///       print -- $(( $1 ))                       # trailing-dot stays raw
///     else
///       printf "$_forms[_outform]\n" $_outdigits $1   # _forms[1] = '%2$g'
///     fi
///   else
///     printf "%d\n" $1                           # no dot → integer
///   fi
///
/// So with the default `_outform=1` a float goes through `%g` (6 significant
/// digits) — `atan(1)*4` prints `3.14159`, not the full `3.1415926535897931`
/// the expansion produced. The bare-trailing-dot case is exempt so `sqrt(16)`
/// keeps zsh's `4.`. Echoing arithsubst's output verbatim reported every
/// irrational at full double precision.
///
/// `-#base` output (zcalc:100-101) is not reached: the native impl has no
/// `_base`, so this covers the default path only.
fn zcalc_show_value(result: &str) -> String {
    // zcalc:102 `[[ $1 = *.* ]]` — a dot anywhere means "float".
    if !result.contains('.') {
        // zcalc:110 `printf "%d\n" $1` — a CONVERSION, not an echo. It matters
        // for the non-finite values, which carry no dot and so land here:
        // `zcalc -e -f '1/0'` is Inf, and printf %d renders that as ZLONG_MAX
        // (9223372036854775807), while NaN renders as 0. Returning the text
        // verbatim printed "Inf"/"NaN", which zsh never shows. Rust's float→int
        // cast saturates (Inf → i64::MAX) and maps NaN → 0, matching the C
        // cast's observed behaviour.
        if result.parse::<i64>().is_ok() {
            return result.to_string();
        }
        if let Ok(f) = result.parse::<f64>() {
            return (f as i64).to_string();
        }
        return result.to_string();
    }
    // zcalc:104 `($_outform -eq 1 && $1 = *.)` — a value that ENDS in the dot
    // prints raw so the trailing "." is not lost.
    if result.ends_with('.') {
        return result.to_string(); // zcalc:105 print -- $(( $1 ))
    }
    match result.parse::<f64>() {
        // zcalc:107 `printf "$_forms[_outform]\n" $_outdigits $1` with
        // _forms[1] = '%2$g' — the PRINTF builtin's %g (default precision 6).
        // Not `convfloat`: that is the float-PARAMETER formatter and re-appends
        // a trailing `.` when %g produced none (params.rs:10770, c:5748-5749),
        // so `sqrt(2)*sqrt(2)` (2.0000000000000004) printed `2.` where zsh
        // prints `2`.
        Ok(v) => crate::ported::builtin::format_spec_float_conv("%g", v, 'g'),
        Err(_) => result.to_string(),
    }
}

pub(crate) fn zcalc(args: &[String]) -> i32 {
    // c:Functions/Misc/zcalc — option scan: `-e` = expression mode
    // (all non-option args are expressions, each evaluated and printed),
    // `-f` = `setopt forcefloat` (float arithmetic, so `3/4` → 0.75).
    // Options may be bundled (`-ef`). Interactive REPL mode (no `-e`)
    // is unsupported in a non-tty.
    let mut expression_mode = false;
    let mut force_float = false;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--" {
            i += 1;
            break;
        }
        // A leading `-` followed only by alphabetic flag letters is an
        // option bundle; anything else (a bare expression, a negative
        // number like `-5+3`) ends the option scan.
        let is_opt =
            a.len() > 1 && a.starts_with('-') && a[1..].chars().all(|c| c.is_ascii_alphabetic());
        if !is_opt {
            break;
        }
        for c in a[1..].chars() {
            match c {
                'e' => expression_mode = true,
                'f' => force_float = true,
                _ => {}
            }
        }
        i += 1;
    }
    let exprs = &args[i..];
    if !expression_mode || exprs.is_empty() {
        eprintln!("zshrs:zcalc:1: interactive mode not supported in non-tty; use `zcalc -e EXPR`");
        return 1;
    }
    // zcalc:133 `if zmodload -i zsh/mathfunc 2>/dev/null; then` — zcalc pulls
    // in the math library on startup, which is why `zcalc -e 'sqrt(16)'` works
    // in zsh while a bare `$(( sqrt(16) ))` does not. The named-function table
    // stays EMPTY until the module boots (math.rs:2519 gates on MOD_INIT_B),
    // so the native impl answered `unknown function: sqrt` for every math
    // function zcalc exists to provide. require_module is idempotent
    // (needs_load checks MOD_INIT_B) and silent=1 mirrors zcalc's `2>/dev/null`.
    if let Ok(mut tab) = crate::ported::module::MODULESTAB.lock() {
        let _ = crate::ported::module::require_module(&mut tab, "zsh/mathfunc", None, 1, false);
    }
    // c:Functions/Misc/zcalc:187 `setopt forcefloat` for `-f`. Save and
    // restore so the option doesn't leak into the caller's shell state.
    let saved_ff = crate::ported::zsh_h::isset(crate::ported::zsh_h::FORCEFLOAT);
    if force_float {
        crate::ported::options::opt_state_set("forcefloat", true);
    }
    for expr in exprs {
        // zcalc:101/105 `print -- $(( … ))` — the value is printed by the
        // arithmetic expansion itself, so a FAILED evaluation prints the
        // diagnostic and no value at all (`zcalc -e '1/0'` writes only
        // "division by zero" to stderr; stdout stays empty). arithsubst
        // reports the error and hands back "0", which was then printed as a
        // result, so a division by zero answered `0`.
        use std::sync::atomic::Ordering;
        let before = crate::ported::utils::errflag.load(Ordering::Relaxed);
        let result = crate::ported::subst::arithsubst(expr, "", "");
        let raised = crate::ported::utils::errflag.load(Ordering::Relaxed) != before;
        if !raised {
            println!("{}", zcalc_show_value(&result));
        } else {
            // The expression failed; don't let the flag abort the caller's
            // shell — zcalc keeps going and still exits 0 (rc=0 above).
            crate::ported::utils::errflag.store(before, Ordering::Relaxed);
        }
    }
    if force_float {
        crate::ported::options::opt_state_set("forcefloat", saved_ff);
    }
    0
}

#[cfg(test)]
mod add_zsh_hook_tests {
    //! Tests for the `add-zsh-hook` builtin — the SHELL-LEVEL hook
    //! registration mechanism (`<hook>_functions` paramtab arrays;
    //! port of `Src/Functions/Misc/add-zsh-hook`). Distinct from the
    //! C-module HOOKTAB / `runhookdef` system, which is tested in
    //! `src/ported/module.rs`.
    use crate::vm_helper::ShellExecutor;

    #[test]
    fn add_zsh_hook_registers_function_in_paramtab_array() {
        let _g = crate::test_util::global_state_lock();
        let mut exec = ShellExecutor::new();
        let rc = exec.builtin_add_zsh_hook(&["chpwd".to_string(), "my_fn".to_string()]);
        assert_eq!(rc, 0);
        assert_eq!(
            exec.array("chpwd_functions").unwrap(),
            vec!["my_fn".to_string()]
        );
        // Cleanup so other tests aren't polluted.
        crate::ported::params::setaparam("chpwd_functions", Vec::new());
    }

    #[test]
    fn add_zsh_hook_appends_distinct_functions_in_order() {
        let _g = crate::test_util::global_state_lock();
        let mut exec = ShellExecutor::new();
        crate::ported::params::setaparam("precmd_functions", Vec::new());
        exec.builtin_add_zsh_hook(&["precmd".to_string(), "alpha".to_string()]);
        exec.builtin_add_zsh_hook(&["precmd".to_string(), "beta".to_string()]);
        exec.builtin_add_zsh_hook(&["precmd".to_string(), "gamma".to_string()]);
        assert_eq!(
            exec.array("precmd_functions").unwrap(),
            vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()]
        );
        crate::ported::params::setaparam("precmd_functions", Vec::new());
    }

    #[test]
    fn add_zsh_hook_is_idempotent_for_duplicate_function() {
        let _g = crate::test_util::global_state_lock();
        let mut exec = ShellExecutor::new();
        crate::ported::params::setaparam("preexec_functions", Vec::new());
        exec.builtin_add_zsh_hook(&["preexec".to_string(), "solo".to_string()]);
        exec.builtin_add_zsh_hook(&["preexec".to_string(), "solo".to_string()]);
        exec.builtin_add_zsh_hook(&["preexec".to_string(), "solo".to_string()]);
        assert_eq!(
            exec.array("preexec_functions").unwrap(),
            vec!["solo".to_string()]
        );
        crate::ported::params::setaparam("preexec_functions", Vec::new());
    }

    #[test]
    fn add_zsh_hook_d_removes_target_function_only() {
        let _g = crate::test_util::global_state_lock();
        let mut exec = ShellExecutor::new();
        crate::ported::params::setaparam("zshexit_functions", Vec::new());
        exec.builtin_add_zsh_hook(&["zshexit".to_string(), "keep_a".to_string()]);
        exec.builtin_add_zsh_hook(&["zshexit".to_string(), "drop_me".to_string()]);
        exec.builtin_add_zsh_hook(&["zshexit".to_string(), "keep_b".to_string()]);
        let rc = exec.builtin_add_zsh_hook(&[
            "-d".to_string(),
            "zshexit".to_string(),
            "drop_me".to_string(),
        ]);
        assert_eq!(rc, 0);
        assert_eq!(
            exec.array("zshexit_functions").unwrap(),
            vec!["keep_a".to_string(), "keep_b".to_string()],
            "-d must only remove the named function, not every entry"
        );
        crate::ported::params::setaparam("zshexit_functions", Vec::new());
    }

    #[test]
    fn add_zsh_hook_d_on_missing_function_is_noop() {
        let _g = crate::test_util::global_state_lock();
        let mut exec = ShellExecutor::new();
        crate::ported::params::setaparam("periodic_functions", Vec::new());
        exec.builtin_add_zsh_hook(&["periodic".to_string(), "alone".to_string()]);
        let rc = exec.builtin_add_zsh_hook(&[
            "-d".to_string(),
            "periodic".to_string(),
            "never_registered".to_string(),
        ]);
        assert_eq!(rc, 0);
        assert_eq!(
            exec.array("periodic_functions").unwrap(),
            vec!["alone".to_string()]
        );
        crate::ported::params::setaparam("periodic_functions", Vec::new());
    }

    #[test]
    fn add_zsh_hook_rejects_too_few_args_no_state_change() {
        let _g = crate::test_util::global_state_lock();
        let mut exec = ShellExecutor::new();
        let rc_no_args = exec.builtin_add_zsh_hook(&[]);
        let rc_one_arg = exec.builtin_add_zsh_hook(&["zshaddhistory".to_string()]);
        assert_eq!(rc_no_args, 1);
        assert_eq!(rc_one_arg, 1);
        // Error path must not populate the array.
        assert!(
            exec.array("zshaddhistory_functions").is_none()
                || exec.array("zshaddhistory_functions").unwrap().is_empty()
        );
    }

    #[test]
    fn add_zsh_hook_d_rejects_too_few_args() {
        let _g = crate::test_util::global_state_lock();
        let mut exec = ShellExecutor::new();
        let rc = exec.builtin_add_zsh_hook(&["-d".to_string(), "chpwd".to_string()]);
        assert_eq!(rc, 1);
    }
}

/// zshrs-extension builtins folded into `builtintab` at runtime by
/// `createbuiltintable` (the `znative` package manager, the ztest/zassert
/// framework, `watch`/`sched`) but absent from the static `BUILTINS` port
/// table. `whence`/`type`/`where`/`which` scan `BUILTINS`, so without these
/// helpers they misclassify these first-class builtins as external/"none".
/// Lives outside `src/ported/` (Rust-only, no C counterpart) so the
/// port-drift gate stays clean — same arrangement as the daemon `z*`
/// `is_zshrs_builtin` check that `bin_whence` already calls.
pub fn extension_builtin_defs() -> impl Iterator<Item = &'static crate::ported::zsh_h::builtin> {
    crate::pkg::builtin::bintab
        .iter()
        .chain(crate::extensions::ztest::bintab.iter())
        .chain(crate::ported::modules::watch::bintab.iter())
}

/// True if `name` is a zshrs builtin that the static `BUILTINS` port table
/// does not list, so `whence`/`type`/`command` classify it as `builtin`:
///
///  * the folded extension bintabs (znative, ztest, watch), and
///  * every fusevm-registered builtin — the authoritative VM registry
///    (`fusevm::shell_builtins::is_builtin`) that the compiler resolves a
///    literal command name against. This covers the zshrs-original builtins
///    (`async`, `await`, `barrier`, `peach`, `doctor`, `dbview`, …) AND the
///    coreutils shadows (`cat`, `head`, `sort`, `cut`, …) that zshrs runs
///    in-process. Without this they dispatch on a literal name but report
///    `none`/external — a builtin that `whence` can't see.
///  * host-registered native commands (`extensions/native_cmds.rs`) — the
///    sibling runtimes a fat binary links in (`git`, `arb`, `stryke` in the
///    zshrs-native build). They dispatch through `try_run_registered_builtin`
///    like the rest, so they must classify the same way; the table is empty in
///    the thin shell, where this term is always false.
pub fn is_extension_builtin(name: &str) -> bool {
    fusevm::shell_builtins::is_builtin(name)
        || LOCAL_ONLY_BUILTINS.contains(&name)
        || crate::native_cmds::is_registered(name)
        || extension_builtin_defs().any(|b| b.node.nam == name)
}

/// zshrs builtins that `register_builtins` installs on the VM and
/// `try_run_registered_builtin` dispatches by name, but which the pinned
/// `fusevm` release's shared name registry does not know yet (its
/// `shell_builtins::is_builtin` is the table `is_extension_builtin`
/// consults first). Without this list `whence -w`/`type` report such a
/// name as `none` even though calling it runs the builtin.
///
/// Keep in sync with `fusevm_bridge::try_run_registered_builtin`; an
/// entry graduates off this list once fusevm ships the name.
///
/// The coreutils-shaped entries below are NOT optional bookkeeping. Each
/// one has a dispatch arm in `fusevm_bridge` and therefore ANSWERS when the
/// user types the name — but the pinned fusevm registry doesn't list them,
/// so `whence -w paste` reported `command`, `whence -a paste` showed only
/// `/usr/bin/paste`, and `${+builtins[paste]}` was 0. The shell ran its own
/// implementation while every tool you could ask about it pointed at the
/// system binary, which is the worst of both: `command paste -sd, -` and
/// `paste -sd, -` behaved differently with nothing to explain why. The
/// names registered through `reg_overridable!` (cat, head, sort, …) never
/// had this problem because fusevm knows them.
pub const LOCAL_ONLY_BUILTINS: &[&str] = &[
    "provenance",
    // Dispatched by `fusevm_bridge`'s command-name match arms.
    "arch",
    "base64",
    "cksum",
    "comm",
    "dircolors",
    "env",
    "expand",
    "expr",
    "factor",
    "fold",
    "groups",
    "link",
    "logname",
    "mkfifo",
    "nice",
    "nl",
    "nproc",
    "paste",
    "printenv",
    "sha256sum",
    "shuf",
    "sum",
    "tac",
    "tput",
    "tsort",
    "tty",
    "unexpand",
    "unlink",
    "users",
    "yes",
    "zbuild",
];
