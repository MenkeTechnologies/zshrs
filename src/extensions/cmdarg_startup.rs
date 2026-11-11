//! The startup and run sequence of a non-interactive shell: process-entry
//! state, command-line options, signal arming, `-c` parameter seeding, and
//! running the `-c` string to an exit status.
//!
//! !!! RUST-ONLY — no C counterpart as a unit !!! C runs these steps inline in
//! `zsh_main`, `setupvals` and `init_misc` (Src/init.c). The zshrs binary
//! dispatches `-c` and script files without going through `zsh_main`, so the
//! steps live here and are shared by that dispatch (bins/zshrs.rs) and by
//! [`crate::embed::run_program`], which runs the `zsh -f -c` sequence in a
//! forked child.

use crate::vm_helper::ShellExecutor;

/// Process-entry stamps: `$HOME` presence, the inherited signal
/// dispositions, the `$SECONDS` origin, and `$PPID` / `$$`. Must run before
/// any other shell initialization in the process.
pub fn process_entry_stamps() {
    // C's `home` is NULL when the shell starts without `$HOME`, which `cd` reports (c:881).
    crate::ported::params::HOME_NULL.store(
        std::env::var_os("HOME").is_none(),
        std::sync::atomic::Ordering::Relaxed,
    );
    // Read the inherited signal dispositions before anything below installs
    // or resets one (the SIGPIPE reset further down, the init_signals replay).
    // bash's trap builtin needs the original SIG_IGN set (BUGS.md #1109).
    crate::startup_signals::snapshot_inherited_ignores();
    // c:Src/init.c:1121 — `zgettime_monotonic_if_available(&shtimer);
    // /* init $SECONDS */`. C stamps `shtimer` once, at shell start.
    // zshrs's analog is a lazily-primed OnceLock
    // (`params::shtimer_lock()`), and the `-c` driver never reaches
    // `setupvals` (it dispatches straight into `ShellExecutor::new`), so
    // nothing stamped it: the FIRST reader primed it to its own "now",
    // making every elapsed-since-shell-start measurement come out ~0.
    //   zsh   -fc 'sleep 2.2; print $SECONDS'  -> 2
    //   zshrs -fc 'sleep 2.2; print $SECONDS'  -> 0   (before this)
    //   zsh   -fc 'sleep 0.5; time'  -> `shell … 1% cpu 0.511 total`
    //   zshrs -fc 'sleep 0.5; time'  -> `… 1875199% cpu 0.000 total`
    // Merely TOUCHING the lock primes it to the current instant, which is
    // exactly C's stamp; doing it here — the process entry point, before
    // any dispatch path forks off — covers `-c`, script and interactive
    // alike. Must stay the first statement so shell init cannot skew it.
    let _ = crate::ported::params::shtimer_lock(); // c:1121
    // c:Src/init.c:1226-1227 — `ppid = getppid(); mypid = getpid();`, the
    // values `$PPID` / `$$` read for the life of the shell (a forked
    // pipeline stage or `&` job keeps them). `setupvals` stores them too,
    // but the emulation drivers (`--bash`, `--sh`, `--ksh`, …) dispatch
    // without reaching it, which left both at 0 there. Same process-entry
    // placement, and the same reason, as the `shtimer` stamp above.
    crate::ported::params::ppid.store(
        unsafe { libc::getppid() } as i64,
        std::sync::atomic::Ordering::Relaxed,
    ); // c:1226
    crate::ported::params::mypid.store(
        unsafe { libc::getpid() } as i64,
        std::sync::atomic::Ordering::Relaxed,
    ); // c:1227
}

/// Install the process-entry environment that `paramtab` is imported from
/// (c:Src/params.c:893), then restore the default SIGPIPE disposition.
/// `zsh_dropin` is true for `--zsh` / `--zsh-compat`.
pub fn install_entry_environment(mut entry_env: Vec<(String, String)>, zsh_dropin: bool) {
    // zshrs ships zsh's man/info pages (build.rs packs vendor/zsh/{man1,info};
    // bundled_docs writes ~/.zshrs/{man,info}). Their directories have to go
    // onto MANPATH/INFOPATH HERE, while the snapshot is still mutable: once
    // it is frozen, paramtab is built from it and a later setenv can no
    // longer change what `$INFOPATH` reports in the shell.
    //
    // The drop-in flag (set again with the rest of the mode at the CLI-mode
    // block below) has to be known HERE: `--zsh` publishes none of the
    // bundled-docs paths, because zsh itself sets none of them.
    crate::extensions::dash_mode::set_zsh_dropin(zsh_dropin);
    crate::bundled_docs::publish_into(&mut entry_env);
    let _ = crate::ported::params::environ.set(entry_env);
    // Restore default SIGPIPE behavior before anything writes to
    // stdout/stderr. Rust runtime installs SIG_IGN on SIGPIPE in
    // some Linux builds and ignores it on macOS — either way,
    // writes to a closed pipe yield an EPIPE error that bubbles
    // up and panics the println!/writeln! callers in any builtin
    // emitting multi-line output (\`set -o\`, \`setopt\`,
    // \`functions\`, etc.).
    //
    // zsh-the-program writes are bare write() syscalls with no
    // EPIPE recovery, so the C process dies silently with status
    // 141 on SIGPIPE. Match that: install SIG_DFL so we terminate
    // on broken pipe instead of panicking. Test: \`set -o | head -3\`
    // exited 1 with a panic stack trace; with SIG_DFL it exits 141
    // and emits only the head'd lines.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

/// The shell mode and option words the command line selected.
pub struct StartupFlags<'a> {
    pub zsh_compat: bool,
    pub bash_compat: bool,
    pub dash: bool,
    pub posix: bool,
    pub ksh: bool,
    pub xtrace: bool,
    pub verbose: bool,
    pub no_rcs: bool,
    /// `(optno, on)` for every option word, in argv order.
    pub opt_actions: &'a [(i32, bool)],
    /// The deferred `emulate X` of a zsh-style drop-in leg (`--sh --zsh`).
    pub zsh_style_emu: Option<&'a str>,
}

/// Apply the command line's shell mode and options to a fresh executor, for
/// the `-c` and script-file dispatch (never the interactive path, which goes
/// through `ported::init::parseargs`).
pub fn apply_cli_flags(executor: &mut ShellExecutor, flags: &StartupFlags<'_>) {
    // Apply shell mode
    executor.zsh_compat = flags.zsh_compat;
    executor.bash_compat = flags.bash_compat;
    if flags.dash {
        // dash is sh + strict subset; enter_dash_mode sets EMULATE_SH
        // AND raises DASH_STRICT (enter_posix_mode would clear it).
        executor.enter_dash_mode();
    } else if flags.posix {
        executor.enter_posix_mode();
    }
    if flags.ksh {
        executor.enter_ksh_mode();
    }
    // The `emulate` calls above reset the whole option table, so the
    // drop-in's own deltas (set once at mode selection) go back on top.
    crate::emulation_startup::apply_personality_option_deltas();
    crate::emulation_startup::apply_personality_param_deltas();
    if flags.xtrace {
        crate::ported::options::opt_state_set("xtrace", true);
    }
    if flags.verbose {
        crate::ported::options::opt_state_set("verbose", true);
    }
    if flags.no_rcs {
        // Match zsh -f: RCS is turned off so `setopt` lists `norcs`.
        // zsh keeps globalrcs on (only the user-rcs files are
        // skipped). HASHDIRS is not touched here — it derives from
        // INTERACTIVE below, the way c:314-315 does it.
        crate::ported::options::opt_state_set("rcs", false);
    }
    // c:Src/init.c:298 and c:526 — both the script-file dispatch
    // (`opts[INTERACTIVE] &= 1`) and `-c` (`new_opts[INTERACTIVE]
    // &= 1`) clear the default-on sentinel 2 → 0. Only an explicit
    // `-i` (which writes 1) survives that mask, so establish OFF
    // here and let the replayed option words below override it.
    // `apply_cli_flags` only runs on those two dispatch paths; the
    // no-script/no-command path goes through
    // `ported::init::parseargs`, which keeps the full 0/1/2 model.
    crate::ported::options::opt_state_set("interactive", false);
    // c:Src/init.c:368 — `opts[USEZLE] = 1;` in
    // `parseopts_setemulate`, i.e. before the option words are read,
    // so an explicit `-Z` / `+Z` overrides it and `init_io` gets the
    // final say below.
    crate::ported::options::opt_state_set("zle", true);
    // c:Src/init.c:501 / c:526 — `dosetopt(optno, action, toplevel,
    // new_opts)` for every option word, in argv order. `force` is
    // the C `toplevel` flag (1 here), so the startup-only options
    // INTERACTIVE / SHINSTDIN / SINGLECOMMAND / USEZLE are settable
    // from the command line exactly as they are in C's parseargs.
    for &(optno, action) in flags.opt_actions {
        crate::ported::options::dosetopt(optno, action as i32, 1);
    }
    // c:Src/init.c:312-315 — `if (opts[MONITOR] == 2)
    //   opts[MONITOR] = opts[INTERACTIVE]; if (opts[HASHDIRS]
    //   == 2) opts[HASHDIRS] = opts[INTERACTIVE];`. Both
    // MONITOR and HASHDIRS auto-derive from INTERACTIVE state.
    // Under `-c` mode, stdin is parsed from the cmd arg (not
    // tty) so INTERACTIVE=false, which means HASHDIRS=false.
    // Without this derivation, `setopt` no-args under `-c`
    // emitted nothing because hashdirs stayed at its emulate-
    // ZSH default ON, matching the no-divergence filter at
    // Src/options.c:462. Bug #87 in docs/BUGS.md.
    let interactive = crate::ported::options::opt_state_get("interactive").unwrap_or(false);
    crate::ported::options::opt_state_set("monitor", interactive);
    crate::ported::options::opt_state_set("hashdirs", interactive);
    // c:Src/init.c:703-710 — `init_io`:
    //   if (interact) { init_shout();
    //                   if (!SHTTY || !shout) opts[USEZLE] = 0; }
    //   else opts[USEZLE] = 0;
    // The comment above it says "only use zle if SHTTY != -1", but
    // the code tests `!SHTTY`, i.e. SHTTY == 0 — and `movefd` never
    // hands back fd 0. `!shout` cannot fire either: `init_shout`
    // falls back to `shout = stderr` when SHTTY == -1 (c:735-740,
    // "Since we're interactive, it's nice to have somewhere to
    // write"). So the branch collapses to `USEZLE = interact`, which
    // is why `zsh -f -i -c …` reports `zle` on even with no
    // controlling terminal at all. Reproduce the observable rule,
    // not the comment.
    if !interactive {
        crate::ported::options::opt_state_set("zle", false); // c:707/710
    }
    // c:Src/init.c:715-718 — `if (opts[MONITOR]) { if (SHTTY == -1)
    //   opts[MONITOR] = 0; … }`. MONITOR, unlike USEZLE, really does
    // need a terminal. SHTTY is whatever `init_io` would have
    // acquired: fd 0 or fd 1 when either is a tty (c:640, c:678),
    // else `/dev/tty` (c:682). The `-c` / script dispatch never
    // reaches `init_io` — only the no-command path does, through
    // `ported::init::zsh_main` — so make the same call here and
    // close the probe fd again, since this path never edits a line.
    let has_shtty = unsafe {
        libc::isatty(0) != 0 || libc::isatty(1) != 0 || {
            let fd = libc::open(c"/dev/tty".as_ptr(), libc::O_RDWR | libc::O_NOCTTY);
            if fd >= 0 {
                libc::close(fd);
                true
            } else {
                false
            }
        }
    };
    if !has_shtty {
        crate::ported::options::opt_state_set("monitor", false); // c:718
    }
    // c:Src/init.c:1907-1908 + 716-722 — zsh_main runs init_io after the
    // options are parsed, so an interactive shell with MONITOR acquires the
    // terminal's process group (`origpgrp = GETPGRP(); acquire_pgrp();`).
    // This path ran init_io before `-i` turned MONITOR on, which left
    // `mypgrp` 0, and the first `attachtty(mypgrp)` after a foreground job
    // failed with "can't set tty pgrp". init_io is idempotent.
    if crate::ported::options::opt_state_get("monitor").unwrap_or(false) {
        crate::ported::init::init_io(None);
    }
    // c:Src/init.c:1913 — `init_builtins()`: zsh_main runs it after
    // parseargs installed the emulation, so `zsh --emulate sh -c …`
    // (and a `--sh` drop-in) starts with the `repeat` reserved word
    // disabled. This dispatch path never reaches zsh_main. The zsh-style
    // leg skips it: that reference is a zsh that ran init_builtins under
    // EMULATE_ZSH and then typed `emulate sh`, so `repeat` stays reserved.
    if flags.zsh_style_emu.is_none() {
        crate::ported::builtin::init_builtins();
    }
    // zsh-STYLE leg (`--sh --zsh` / `--ksh --zsh`): install the emulation
    // HERE, last, and NOT fully — the reference is a zsh that finished
    // startup and then ran `emulate X` as its first command, so the option
    // words above were resolved with `zshletters` and only the OPT_EMULATE
    // options get reset (c:Src/options.c:516). See the deferral note at the
    // mode-selection site.
    if let Some(sub) = flags.zsh_style_emu {
        crate::ported::options::emulate(sub, false);
    }
}

/// Arm the startup signal handlers. Runs after [`apply_cli_flags`].
pub fn arm_dispatch_signals() {
    // c:Src/init.c:1427-1470 (`init_signals`) — C reaches it through
    // `zsh_main` on EVERY invocation; this dispatch path never calls
    // that function, so run the same sequence here. It must come
    // AFTER `apply_cli_flags`, which is what settles INTERACTIVE and
    // MONITOR, because C's gates read them.
    crate::startup_signals::init_dispatch_signals();
    // c:Src/init.c:1455 — `install_handler(SIGCHLD);`, the one line
    // of `init_signals` that `init_dispatch_signals` still leaves
    // out. It lives here rather than there because it is not a
    // disposition policy that could differ per emulation: it is the
    // shell's ONLY reaper, and without it every background child of
    // a `-c` run stays a zombie for the life of the shell, so the
    // process table grows with each one. `sleep 0.2 & ; sleep 1; ps
    // -o stat= -p $!` printed `Z` where zsh prints nothing.
    //
    // Arming it makes the reaper race the pipeline's own targeted
    // `waitpid` for the forked stages; the reaped-status ring in
    // `extensions/reaped_status.rs` is what keeps `$pipestatus` intact through
    // that race (see `waitpid_eintr` in fusevm_bridge.rs).
    crate::ported::signals::install_handler(libc::SIGCHLD);
    // The drop-ins' own arming (bash's termsig_handler shape); a no-op in zsh mode.
    crate::extensions::emulation_startup::init_dropin_signals();
}

/// Seed the parameters a `-c` run starts with: `$ZSH_EXECUTION_STRING`,
/// `$ZSH_EXEPATH` (resolved from `exe_argv0`; `None` leaves it unset), the
/// `cmdarg` eval context, `$0` (`zero`), `$1…` (`pparams`, present when a
/// name word followed the command string), and the script name used for
/// `%N` and error prefixes.
pub fn seed_cmdarg(
    executor: &mut ShellExecutor,
    code: &str,
    exe_argv0: Option<&str>,
    zero: String,
    pparams: Option<Vec<String>>,
    basename: String,
) {
    // c:Src/init.c:1340 — `if (cmd)
    //                       setsparam("ZSH_EXECUTION_STRING",
    //                                 ztrdup_metafy(cmd));`
    // ZSH_EXECUTION_STRING carries the -c argument so user scripts
    // can introspect what they're running. p10k probes this for the
    // initial prompt-deferral decision; pre-c-mode-aware tooling
    // reads it to log invocations. Previously only setupvals() in
    // ported/init.rs set the env var, but setupvals isn't wired
    // into the bin entry path so the value was always empty.
    crate::ported::params::setsparam("ZSH_EXECUTION_STRING", code);
    // c:Src/init.c:1315-1326 — `/* ZSH_EXEPATH */`, the same setupvals
    // block this path skips: argv[0] resolved against the start-up cwd.
    if let Some(p) = exe_argv0.and_then(|argv0| {
        crate::ported::init::getmypath(
            Some(argv0),
            crate::ported::params::getsparam("PWD").as_deref(),
        )
    }) {
        crate::ported::params::setsparam("ZSH_EXEPATH", &p);
    }
    // c:Src/init.c:1535 — `execstring(cmd, 0, 1, "cmdarg")` pushes
    // "cmdarg" onto the zsh_eval_context stack BEFORE running the
    // -c command. ZSH_EVAL_CONTEXT is the `:`-joined view of that
    // stack tied to the `zsh_eval_context` array. For a non-nested
    // -c invocation the stack is just ["cmdarg"], so the readable
    // form is the literal string "cmdarg". Plugins (zinit's
    // `[[ $ZSH_EVAL_CONTEXT == *cmdarg* ]]` predicate, p10k's
    // turbo-mode hooks) gate on this.
    // ZSH_EVAL_CONTEXT carries PM_READONLY (declared at
    // params.rs special_params:836), so setsparam would be
    // rejected by assignstrvalue's PM_READONLY guard. Write
    // u_str directly — same pattern BUILTIN_SET_LINENO uses
    // for the LINENO bypass. C zsh's PM_SPECIAL GSU setfn
    // handles this implicitly; the Rust port lacks the GSU
    // vtable so internal writes bypass via direct paramtab
    // mutation.
    //
    // Push "cmdarg" onto the static `zsh_eval_context` stack AND
    // mirror into the tied `zsh_eval_context` array + the
    // `ZSH_EVAL_CONTEXT` scalar so `${zsh_eval_context[*]}`
    // expansion reads it. Bug #262 in docs/BUGS.md. Inlined here
    // (formerly a shared vm_helper fn; relocated into doshfunc +
    // this bin-entry callsite). No nested process below pops it —
    // the `-c` invocation owns the stack for the whole run.
    if let Ok(mut ctx) = crate::ported::exec::zsh_eval_context.lock() {
        ctx.push("cmdarg".to_string());
        let joined = ctx.join(":");
        if let Ok(mut tab) = crate::ported::params::paramtab().write() {
            if let Some(pm) = tab.get_mut("zsh_eval_context") {
                pm.u_arr = Some(ctx.clone());
                pm.node.flags &= !(crate::ported::zsh_h::PM_UNSET as i32);
            }
            if let Some(pm) = tab.get_mut("ZSH_EVAL_CONTEXT") {
                pm.u_str = Some(joined);
                pm.node.flags &= !(crate::ported::zsh_h::PM_UNSET as i32);
            }
        }
    }
    // POSIX `sh -c script [name [args...]]` semantics (Src/init.c:271 +
    // 479): the next non-option arg AFTER the command string becomes $0;
    // remaining args become $1, $2, …
    if let Some(params) = pparams {
        executor.set_pparams(params);
    }
    executor.set_scalar("0".to_string(), zero.clone());
    // Wire the GSU dispatch: `argzerogetfn` reads
    // `utils::argzero()` which returns this value via
    // `lookup_special_var("0")`. Without `set_argzero`, the GSU
    // dispatch returns None and reads fall back to
    // `executor.variables`, but the explicit-name case above
    // depends on the GSU side knowing the value too.
    crate::ported::utils::set_argzero(Some(zero.clone()));
    // c:Src/init.c:305-306 — `posixzero = *argv;` when a `-c` name word
    // follows the command string; otherwise it stays argv[0] (c:282).
    // `$0` reads it under POSIX_ARGZERO (argzerogetfn, params.c:4908).
    crate::ported::utils::set_posixzero(Some(zero.clone()));
    crate::extensions::dash_mode::set_dash_c_string(true);

    // Per Src/init.c:479 — `-c` mode hardcodes
    //   `scriptname = scriptfilename = ztrdup("zsh")`
    // (literal short name, NOT argzero). In `--zsh` parity mode,
    // match C zsh exactly so PS4 / `%N` / xtrace prefixes byte-
    // match the reference. Plain zshrs `-c` keeps the binary
    // basename for branding. `$0` is argv[0] either way.
    executor.scriptname = Some(basename.clone());
    // scriptfilename mirrors C init.c:479 `scriptname =
    // scriptfilename = ztrdup("zsh")` — both seeded to the
    // basename in -c mode. scriptfilename stays put when we
    // enter a function (only scriptname mutates), so PS4's
    // %x continues to show the file basename inside function
    // bodies.
    executor.scriptfilename = Some(basename.clone());
    // Propagate to the canonical `SCRIPTNAME` static read by
    // prompt expansion (`%N`/`%x`) via `utils::scriptname_get`.
    // Without this, `printprompt4` saw None and `%N`/`%x` fell
    // back to argzero — the full binary path — making xtrace
    // emit `/Users/.../zshrs` instead of `zsh`.
    crate::ported::utils::set_scriptname(Some(basename));
}

/// Run the `-c` string (c:Src/init.c:1568 `execstring(cmd, 0, 1, "cmdarg")`)
/// with SHIN on /dev/null. `Err(status)` is a csh drop-in translation
/// failure, already reported on stderr, after which the shell exits with
/// `status` at once.
pub fn exec_cmdarg(executor: &mut ShellExecutor, code: &str) -> Result<Result<i32, String>, i32> {
    // c:Src/init.c:1563-1566 — init_misc, after the init scripts and
    // before `execstring(cmd, 0, 1, "cmdarg")`:
    //     if (SHIN >= 10) close(SHIN);
    //     SHIN = movefd(open("/dev/null", O_RDONLY | O_NOCTTY));
    // The descriptor is the shell's input for a `-c` run and occupies the
    // first internal slot, so the first `exec {fd}>file` gets 11, as in zsh.
    {
        let old_shin = crate::ported::input::SHIN.with(|s| s.get());
        if old_shin >= 10 {
            let _ = crate::ported::utils::zclose(old_shin); // c:1565
        }
        let devnull = unsafe {
            libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY | libc::O_NOCTTY)
        };
        let shin = crate::ported::utils::movefd(devnull); // c:1566
        crate::ported::input::SHIN.with(|s| s.set(shin));
    }
    match crate::emulation_startup::csh_input(code, crate::emulation_startup::translate_script) {
        Ok(text) => Ok(executor.execute_cmdarg(&text)),
        Err(msg) => {
            eprintln!("{msg}");
            Err(crate::emulation_startup::translation_error_status())
        }
    }
}

/// The exit status of a finished `-c` run (c:Src/init.c:1572 — `zexit(lastval,
/// ZEXIT_NORMAL)`), reporting a compile or run error on stderr first.
pub fn cmdarg_exit_status(
    executor: &ShellExecutor,
    result: Result<i32, String>,
    zsh_style_emu: Option<&str>,
) -> i32 {
    if let Err(e) = result {
        // c:Src/init.c — `__SILENCED__` sentinel from
        // `execute_script_zsh_pipeline` means the parser already
        // emitted the diagnostic via zerr; binary-side print
        // would double it. Bug #142 in docs/BUGS.md.
        if e != "__SILENCED__" {
            eprintln!("zshrs: {}", e);
        }
        // bash: a syntax error exits 2
        return crate::extensions::dash_mode::fatal_error_status().unwrap_or(if crate::extensions::dash_mode::bash_mode() { 2 } else { 1 });
    }
    // !!! DASH-FAMILY GATE — see dash_mode::fatal_error_status !!!
    // The end of a `-c` script is the second place dash's
    // `exraise(EXERROR)` unwind lands (the other is the `( … )`
    // boundary, handled in fusevm_bridge::subshell_end). dash reports 2
    // for a fatal expansion / assignment / arithmetic error; zsh reports
    // `lastval == ERRFLAG_ERROR == 1`. No-op in every other mode.
    {
        use std::sync::atomic::Ordering::Relaxed;
        let ef = crate::ported::utils::errflag.load(Relaxed);
        let fatal = crate::ported::zsh_h::ERRFLAG_ERROR | crate::ported::zsh_h::ERRFLAG_HARD;
        if ef & fatal != 0 {
            if let Some(st) = crate::extensions::dash_mode::fatal_error_status() {
                return st;
            }
            // A zsh-STYLE leg stands in for `zsh -c 'emulate X -c "$script"'`: `emulate` is a
            // builtin, and a builtin that leaves an error pending reports 1
            // (c:Src/builtin.c bin_eval: `if (errflag && !lastval) lastval = errflag`) even though the script's own status was 0.
            if zsh_style_emu.is_some() && executor.last_status() == 0 {
                return 1;
            }
        }
    }
    executor.last_status()
}
