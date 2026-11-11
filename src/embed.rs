//! Run a shell program from a Rust host and collect what it printed.
//!
//! !!! RUST-ONLY — no C counterpart !!! The model is C zsh's `getoutput`
//! (Src/exec.c:4816): fork, point the child's fds at pipes, run the program
//! in the child, read the pipes in the parent. Here the child is a fresh
//! `zsh -f -c` shell rather than a subshell of a running one.
//!
//! Every call forks, so each program starts from a clean shell: zshrs keeps
//! parameters, options, aliases and functions in process-wide tables
//! (`paramtab`, `OPTS_LIVE`, `aliastab`, `shfunctab`) with no reset path, and
//! a separate process is what isolates one call from the next. The host
//! process never touches those tables, so nothing it does can leak in either.
//!
//! # Fork safety
//!
//! `fork()` copies only the calling thread. A lock that another host thread
//! held at that instant stays locked forever in the child, so the child must
//! not take any lock the host may hold. What `run_program` does about each
//! one it can reach:
//!
//! - Rust's stdout/stderr locks: held by the forking thread across `fork()`
//!   (stdout flushed first, so the child inherits an empty buffer). The child
//!   keeps both guards for its whole life; the locks are re-entrant, so every
//!   later write in the child re-enters them instead of waiting.
//! - `tracing`: the child installs a no-op thread default dispatcher before
//!   any shell code runs, so no event reaches the host's subscriber and its
//!   writer lock. Registering a new callsite still read-locks tracing-core's
//!   dispatcher list when the host has registered more than one dispatcher.
//! - The environment: read in the parent, before `fork()`, and handed to the
//!   child, which still reads single variables with `std::env::var` while
//!   starting up. A host must not call `std::env::set_var` concurrently with
//!   `run_program` (already unsound in a threaded program).
//! - Thread-locals: the fork happens on a thread spawned for this call, so the
//!   child's only thread has never run shell code.
//! - The rayon pool: `fork_safe_pool::pool` builds a new pool per pid.
//! - The allocator: the system allocators (macOS libmalloc, glibc) reset
//!   their locks across `fork()`. A custom global allocator must do the same.
//!
//! Children of the child (pipelines, `$(…)`, externals) go through the
//! shell's ordinary `zfork` path.

use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;

/// Stack of the forking thread, which becomes the child's only thread. Shell
/// function calls grow their own stack (`dispatch_function_call`); this
/// covers parsing and compiling, which do not.
const CHILD_STACK_BYTES: usize = 16 * 1024 * 1024;

/// Run `src` as a complete program with `input` as its stdin and return
/// everything it wrote to stdout.
///
/// The program runs in a forked child as `zsh -f -c src` would: no startup
/// files, not interactive, no ZLE, no job control. Its fds 0, 1 and 2 are
/// pipes, so builtins and external commands alike read `input` and have
/// their output captured; nothing reaches the host's terminal.
///
/// Returns `Ok(stdout)` when the program exits 0, including through an
/// explicit `exit 0`. A nonzero exit status (a parse error, a failed last
/// command, `exit 3`) or death by a signal returns `Err` naming the status or
/// signal, followed by everything the program wrote to stderr.
///
/// Calls from different threads run in parallel, each in its own process.
/// The call returns once the program exits and every process holding its
/// stdout or stderr has closed them, as `$(…)` does.
pub fn run_program(src: &str, input: &str) -> Result<String, String> {
    let src = src.to_owned();
    let input = input.as_bytes().to_vec();
    let env: Vec<(String, String)> = std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
        .collect();
    std::thread::Builder::new()
        .name("zshrs-embed".into())
        .stack_size(CHILD_STACK_BYTES)
        .spawn(move || fork_and_collect(&src, input, env))
        .map_err(|e| format!("zshrs: cannot spawn thread: {e}"))?
        .join()
        .map_err(|_| "zshrs: run_program thread panicked".to_string())?
}

fn fork_and_collect(
    src: &str,
    input: Vec<u8>,
    env: Vec<(String, String)>,
) -> Result<String, String> {
    let pipes = || io::pipe().map_err(|e| format!("zshrs: pipe: {e}"));
    let (stdin_r, mut stdin_w) = pipes()?;
    let (mut stdout_r, stdout_w) = pipes()?;
    let (mut stderr_r, stderr_w) = pipes()?;

    let pid = {
        let mut out = io::stdout().lock();
        let _ = out.flush();
        let err = io::stderr().lock();
        // SAFETY: the child runs only `run_child`, which never returns.
        let pid = unsafe { libc::fork() };
        if pid == 0 {
            std::mem::forget(out);
            std::mem::forget(err);
            run_child(
                src,
                [
                    stdin_r.as_raw_fd(),
                    stdout_w.as_raw_fd(),
                    stderr_w.as_raw_fd(),
                ],
                env,
            );
        }
        pid
    };
    if pid < 0 {
        return Err(format!("zshrs: fork: {}", io::Error::last_os_error()));
    }
    // The child holds its own copies; the parent's would keep the read ends
    // from ever seeing EOF.
    drop((stdin_r, stdout_w, stderr_w));

    let writer = std::thread::spawn(move || {
        // A program that exits without reading all of `input` closes the
        // pipe under this write; that EPIPE is not an error of the program.
        // SIGPIPE is blocked on this thread so the host is not killed by it.
        // SAFETY: changes this thread's own signal mask only.
        unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGPIPE);
            libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        }
        let _ = stdin_w.write_all(&input);
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr_r.read_to_end(&mut buf);
        buf
    });
    let mut stdout = Vec::new();
    let _ = stdout_r.read_to_end(&mut stdout);
    let stderr = stderr_reader.join().unwrap_or_default();
    let _ = writer.join();

    let mut status = 0;
    // SAFETY: waits for the child forked above.
    while unsafe { libc::waitpid(pid, &mut status, 0) } < 0 {
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(format!("zshrs: waitpid: {e}"));
        }
    }

    let stderr = String::from_utf8_lossy(&stderr);
    if libc::WIFSIGNALED(status) {
        return Err(format!(
            "killed by signal {}\n{stderr}",
            libc::WTERMSIG(status)
        ));
    }
    match libc::WEXITSTATUS(status) {
        0 => Ok(String::from_utf8_lossy(&stdout).into_owned()),
        code => Err(format!("exit status {code}\n{stderr}")),
    }
}

/// The child: put the pipes on fds 0-2, drop everything else inherited from
/// the host, and run `src` the way the binary runs `zsh -f -c src`.
fn run_child(src: &str, pipe_fds: [i32; 3], env: Vec<(String, String)>) -> ! {
    // SAFETY: fd and signal-state calls on this process only.
    unsafe {
        // Lift the pipes above 2 first: a host that had closed one of fds
        // 0-2 got a pipe on it, and a plain dup2 sequence could overwrite
        // a pipe before it is moved into place.
        let lifted = pipe_fds.map(|fd| libc::fcntl(fd, libc::F_DUPFD, 3));
        for (target, fd) in lifted.into_iter().enumerate() {
            libc::dup2(fd, target as i32);
        }
        close_inherited_fds();
        reset_inherited_signal_state();
    }
    crate::atexit_teardown::mark_embedded_child();
    std::mem::forget(tracing::dispatcher::set_default(&tracing::Dispatch::none()));
    std::panic::set_hook(Box::new(|info| {
        let _ = writeln!(io::stderr(), "zshrs: internal error: {info}");
    }));
    let status = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_cmdarg(src, env)))
        .unwrap_or(1);
    crate::atexit_teardown::exit(status)
}

/// The binary's `-c` sequence (bins/zshrs.rs) with `-f` and no other option
/// words: `zshrs` as `$0` and script name, no `$ZSH_EXEPATH` (the executable
/// is the host's), and no startup files.
fn run_cmdarg(src: &str, env: Vec<(String, String)>) -> i32 {
    use crate::cmdarg_startup as startup;
    startup::process_entry_stamps();
    startup::install_entry_environment(env, false);
    let mut executor = crate::vm_helper::ShellExecutor::new();
    startup::apply_cli_flags(
        &mut executor,
        &startup::StartupFlags {
            zsh_compat: false,
            bash_compat: false,
            dash: false,
            posix: false,
            ksh: false,
            xtrace: false,
            verbose: false,
            no_rcs: true,
            opt_actions: &[],
            zsh_style_emu: None,
        },
    );
    startup::arm_dispatch_signals();
    startup::seed_cmdarg(
        &mut executor,
        src,
        None,
        "zshrs".into(),
        None,
        "zshrs".into(),
    );
    match startup::exec_cmdarg(&mut executor, src) {
        Ok(result) => startup::cmdarg_exit_status(&executor, result, None),
        Err(status) => status,
    }
}

/// Close every fd above 2. Without this the child would hold the host's
/// sockets and files open, and the pipe ends of every other `run_program`
/// call in flight: a concurrent call's stdin writer would then never see its
/// reader close, nor its stdout reader see EOF, until this child exits.
unsafe fn close_inherited_fds() {
    let dir = if cfg!(target_os = "linux") {
        "/proc/self/fd"
    } else {
        "/dev/fd"
    };
    let fds: Vec<i32> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok())
            .collect(),
        Err(_) => (3..libc::sysconf(libc::_SC_OPEN_MAX).clamp(3, 65536) as i32).collect(),
    };
    // The listing's own fd is in `fds` and already closed; closing it again
    // is a harmless EBADF.
    for fd in fds.into_iter().filter(|&fd| fd > 2) {
        libc::close(fd);
    }
}

/// Give the child the signal state an `exec` would: caught signals back to
/// their default (the handlers are host code), ignored ones left ignored,
/// and an empty mask (the forking thread's mask is the host's choice).
unsafe fn reset_inherited_signal_state() {
    // 64 covers the classic and real-time signals of Linux and macOS; an
    // out-of-range number just fails the query.
    for sig in 1..=64 {
        let mut cur: libc::sigaction = std::mem::zeroed();
        if libc::sigaction(sig, std::ptr::null(), &mut cur) == 0
            && cur.sa_sigaction != libc::SIG_DFL
            && cur.sa_sigaction != libc::SIG_IGN
        {
            libc::signal(sig, libc::SIG_DFL);
        }
    }
    let mut none: libc::sigset_t = std::mem::zeroed();
    libc::sigemptyset(&mut none);
    libc::pthread_sigmask(libc::SIG_SETMASK, &none, std::ptr::null_mut());
}
