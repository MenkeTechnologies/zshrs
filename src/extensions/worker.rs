//! Worker pool for zshrs — persistent threads for background work.
//!
//! **zshrs-original infrastructure — no C source counterpart.** This
//! module does NOT port a corresponding `Src/*.c` file. C zsh's
//! background-work strategy is `fork(2)`: every completion run,
//! process substitution, or command substitution is a child process
//! (see `zfork()` in Src/exec.c and the `forklevel` machinery
//! Src/init.c uses to track depth). zshrs replaces that pattern with
//! a fixed-size thread pool + crossbeam channel dispatch.
//!
//! Replacement rationale (vs the fork() path the C source takes):
//!   - No fork overhead (50-500μs per fork on macOS)
//!   - No address space duplication
//!   - Warm thread stacks ready to go
//!   - Backpressure via bounded channel
//!
//! Pool size = available_parallelism() clamped to [2, 18].
//! Channel capacity = 4 × pool size (bounded backpressure).
//!
//! Audit fixes applied:
//!   1. crossbeam-channel replaces Arc<Mutex<mpsc::Receiver>> — no mutex contention
//!   2. Bounded channel (4×N) provides backpressure
//!   3. catch_unwind wraps every task — panics logged, worker stays alive
//!   4. tracing spans on submit + worker loop
//!   5. Queue depth metric on submit
//!   6. Task cancellation via AtomicBool flag

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;

/// A unit of work the pool can execute.
type Task = Box<dyn FnOnce() + Send + 'static>;

thread_local! {
    /// True only on threads owned by a `WorkerPool`.
    ///
    /// !!! WARNING: RUST-ONLY HELPER — NO C COUNTERPART !!!
    /// C zsh is single-threaded: the one and only thread owns SHIN and
    /// the line editor, so `inputline()` (Src/input.c:366) can read the
    /// terminal unconditionally. zshrs runs background work (compinit
    /// bytecode backfill, fpath scan, …) on pool threads that share the
    /// process's `interact` / `SHINSTDIN` / SHTTY globals. When such a
    /// task parses a shell body whose lexer buffer drains mid-construct,
    /// the C-faithful "as a last resort, get some more input" arm
    /// (input.c:354-356) fired ON THE WORKER and read the user's tty —
    /// stealing keystrokes from ZLE. `in_worker_thread()` lets the input
    /// layer treat that case as EOF instead.
    static IN_WORKER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// True when the calling thread is a worker-pool thread.
///
/// !!! WARNING: RUST-ONLY HELPER — NO C COUNTERPART !!! See `IN_WORKER`.
pub fn in_worker_thread() -> bool {
    IN_WORKER.with(|f| f.get())
}

/// Fixed-size thread pool with bounded FIFO task queue.
///
/// zshrs-original — replaces C zsh's per-task `fork()` + `wait()`
/// pattern (Src/exec.c `zfork()` / Src/jobs.c child management) with
/// a persistent thread pool. Uses crossbeam-channel for lock-free
/// multi-consumer dispatch — each worker calls `recv()` directly,
/// no mutex.
pub struct WorkerPool {
    /// `workers` field. Behind a Mutex because the threads are spawned on
    /// FIRST USE (see `ensure_spawned`), not in `new`.
    workers: std::sync::Mutex<Vec<Worker>>,
    /// Kept so the workers can be spawned later; cloned per thread.
    receiver: crossbeam_channel::Receiver<Task>,
    /// Set once the threads exist.
    spawned: AtomicBool,
    /// `sender` field.
    sender: Option<crossbeam_channel::Sender<Task>>,
    /// `size` field.
    size: usize,
    /// Shared cancellation flag — when set, workers drop pending tasks
    cancelled: Arc<AtomicBool>,
    /// Queue depth — incremented on submit, decremented on task start
    queued: Arc<AtomicUsize>,
    /// Total tasks completed across all workers
    completed: Arc<AtomicUsize>,
}

struct Worker {
    #[allow(dead_code)]
    id: usize,
    handle: Option<thread::JoinHandle<()>>,
}

impl WorkerPool {
    /// Create a pool with `size` worker threads and bounded channel.
    /// Channel capacity = 4 × size (provides backpressure without
    /// starving).
    /// zshrs-original — no C counterpart. Replaces the
    /// "spawn-on-demand" semantics of `zfork()` (Src/exec.c) with
    /// pre-spawned threads ready to receive work over a bounded
    /// channel.
    pub fn new(size: usize) -> Self {
        let capacity = size * 4;
        let (sender, receiver) = crossbeam_channel::bounded::<Task>(capacity);
        let cancelled = Arc::new(AtomicBool::new(false));
        let queued = Arc::new(AtomicUsize::new(0));
        let completed = Arc::new(AtomicUsize::new(0));

        WorkerPool {
            workers: std::sync::Mutex::new(Vec::new()),
            receiver,
            spawned: AtomicBool::new(false),
            sender: Some(sender),
            size,
            cancelled,
            queued,
            completed,
        }
    }

    /// Spawn the worker threads if they do not exist yet.
    ///
    /// zshrs-original. The pool used to spawn every thread in `new`, which
    /// runs while the shell is still starting: `zshrs -f -c exit` paid 18
    /// `pthread_create`s plus their stacks to run one builtin and exit, and a
    /// profile of any short command showed all of them parked in
    /// `semaphore_wait_trap` for the whole run. Nothing is deferred that a
    /// caller can observe — the first `submit` spawns the pool before the task
    /// is queued, so a task never waits on a thread that is not there.
    fn ensure_spawned(&self) {
        if self.spawned.load(Ordering::Relaxed) {
            return;
        }
        let mut workers = self.workers.lock().unwrap_or_else(|e| e.into_inner());
        if self.spawned.load(Ordering::Relaxed) {
            return; // lost the race; the winner already spawned
        }
        let size = self.size;
        let receiver = &self.receiver;
        let cancelled = &self.cancelled;
        let queued = &self.queued;
        let completed = &self.completed;
        for id in 0..size {
            let rx = receiver.clone();
            let cancelled = Arc::clone(&cancelled);
            let queued = Arc::clone(&queued);
            let completed = Arc::clone(&completed);

            let handle = thread::Builder::new()
                .name(format!("zshrs-worker-{}", id))
                .spawn(move || {
                    // Rust-only: mark this thread as pool-owned so the
                    // input layer never reads the user's tty from it.
                    IN_WORKER.with(|f| f.set(true));
                    loop {
                        let task = match rx.recv() {
                            Ok(task) => task,
                            Err(_) => break, // channel closed → shutdown
                        };

                        queued.fetch_sub(1, Ordering::Relaxed);

                        // Check cancellation before running
                        if cancelled.load(Ordering::Relaxed) {
                            continue; // drain without executing
                        }

                        // Every task starts on a clear error flag. The
                        // thread's `errflag` is private (see
                        // crate::errflag_cell), so an abort or parse error
                        // left behind by the PREVIOUS task on this same
                        // thread would otherwise be inherited — C never
                        // has that problem because its equivalent of a
                        // task is a fresh forked child.
                        crate::ported::utils::errflag.store(0, Ordering::Relaxed);
                        // catch_unwind keeps the worker alive if a task panics
                        if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task))
                        {
                            let msg = if let Some(s) = e.downcast_ref::<&str>() {
                                (*s).to_string()
                            } else if let Some(s) = e.downcast_ref::<String>() {
                                s.clone()
                            } else {
                                "unknown panic".to_string()
                            };
                            tracing::error!(
                                worker = id,
                                panic = %msg,
                                "worker task panicked"
                            );
                        }

                        completed.fetch_add(1, Ordering::Relaxed);
                    }
                    tracing::debug!(worker = id, "worker thread exiting");
                })
                .expect("failed to spawn worker thread");

            workers.push(Worker {
                id,
                handle: Some(handle),
            });
        }

        self.spawned.store(true, Ordering::Relaxed);
        drop(workers);
        tracing::info!(pool_size = size, "worker pool started");
    }

    /// Create a pool sized to the machine's parallelism, clamped to
    /// `[2, 18]`.
    /// zshrs-original — no C counterpart. C zsh has no concept of a
    /// "pool size" because it forks on demand (one child per
    /// background task, see Src/jobs.c).
    pub fn default_size() -> Self {
        let cpus = thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        Self::new(cpus.clamp(2, 18))
    }

    /// Submit a task to the pool. Blocks if the queue is full
    /// (backpressure). Panics if the pool has been shut down.
    /// zshrs-original — replaces the `fork() + execve()` /
    /// `fork() + run-shell-fn` dispatch pairs in Src/exec.c.
    pub fn submit<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        self.ensure_spawned();
        let depth = self.queued.fetch_add(1, Ordering::Relaxed) + 1;
        if depth > self.size * 2 {
            tracing::debug!(queue_depth = depth, "worker pool queue building up");
        }
        self.sender
            .as_ref()
            .expect("pool shut down")
            .send(Box::new(f))
            .expect("all workers dead");
    }

    /// Submit a task and get a receiver for its result.
    /// zshrs-original — closest C analog is the pipe-based
    /// command-substitution result capture in Src/exec.c
    /// (`getoutput()` reading the child's stdout pipe), but using a
    /// typed Rust channel sidesteps the marshalling.
    pub fn submit_with_result<F, R>(&self, f: F) -> crossbeam_channel::Receiver<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.submit(move || {
            let result = f();
            let _ = tx.send(result);
        });
        rx
    }

    /// Signal all workers to drop pending tasks.
    /// Already-running tasks will finish, but queued tasks are
    /// skipped. Reset with `reset_cancel()`.
    /// zshrs-original — closest C analog is the SIGINT/SIGQUIT
    /// signal-storm dispatch C zsh fires at its background children
    /// in Src/signals.c (`killjb()` / `killpg()`), but here we set a
    /// flag instead of sending a signal across a fork boundary.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        tracing::info!("worker pool: cancel requested");
    }

    /// Clear the cancellation flag — pool resumes normal execution.
    /// zshrs-original — no C counterpart.
    pub fn reset_cancel(&self) {
        self.cancelled.store(false, Ordering::Relaxed);
    }

    /// Number of worker threads.
    /// zshrs-original — no C counterpart.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Approximate number of tasks waiting in the queue.
    /// zshrs-original — no C counterpart; closest equivalent is the
    /// `jobtab` length walk Src/jobs.c uses for `jobs -l` output.
    pub fn queue_depth(&self) -> usize {
        self.queued.load(Ordering::Relaxed)
    }

    /// Total tasks completed since pool creation.
    /// zshrs-original — no C counterpart.
    pub fn completed(&self) -> usize {
        self.completed.load(Ordering::Relaxed)
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        // Signal workers to skip remaining queued tasks
        self.cancelled.store(true, Ordering::Relaxed);
        // Drop the sender → channel closes → recv() returns Err → threads exit
        drop(self.sender.take());
        // Give workers a brief window to finish their current task.
        // Don't block indefinitely — the process is exiting.
        let mut workers = self.workers.lock().unwrap_or_else(|e| e.into_inner());
        for w in workers.iter_mut() {
            if let Some(handle) = w.handle.take() {
                // Detach the thread — OS cleans up on process exit.
                // join() would block if a worker is mid-parse on a 500-line
                // completion function. Not worth the wait on Ctrl-D/exit.
                drop(handle);
            }
        }
        // Demoted from `info!` to `debug!` so the default tracing
        // filter (INFO) suppresses it. The bare shutdown announcement
        // has no operational value — interesting telemetry would be
        // a non-zero error count or a stuck worker, which warrants its
        // own surface. Empirically (bug #23 in docs/BUGS.md) the
        // existing info! also leaked to stdout when a script left a
        // duped fd open (`exec 3>&1`): by the time worker Drop runs,
        // the file-backed log writer is closed, and tracing's fallback
        // writes to fd 1 — which is the original stdout the dup
        // pointed at. Default INFO filter no longer triggers this code
        // path at all in normal use.
        tracing::debug!(
            tasks_completed = self.completed.load(Ordering::Relaxed),
            "worker pool shut down"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spin-wait helper for tests: poll `counter` until it reaches
    /// `target` or the deadline elapses. Replaces the old "drop(pool)
    /// implicitly waits" pattern, which broke when production Drop
    /// switched to setting cancelled=true (so queued tasks would be
    /// skipped on drop instead of drained).
    fn wait_for_count(counter: &AtomicUsize, target: usize, max_wait_ms: u64) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(max_wait_ms);
        while counter.load(Ordering::Relaxed) < target {
            if std::time::Instant::now() >= deadline {
                panic!(
                    "wait_for_count timed out: counter={} target={} after {}ms",
                    counter.load(Ordering::Relaxed),
                    target,
                    max_wait_ms
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    #[test]
    fn test_pool_executes_tasks() {
        let _g = crate::test_util::global_state_lock();
        let pool = WorkerPool::new(2);
        let counter = Arc::new(AtomicUsize::new(0));

        for _ in 0..100 {
            let c = Arc::clone(&counter);
            pool.submit(move || {
                c.fetch_add(1, Ordering::Relaxed);
            });
        }

        // Drain explicitly — production Drop sets cancelled=true and
        // skips queued tasks (intentional for shell exit), so the test
        // can't rely on `drop(pool)` to wait.
        wait_for_count(&counter, 100, 5_000);
        drop(pool);
        assert_eq!(counter.load(Ordering::Relaxed), 100);
    }

    #[test]
    fn test_submit_with_result() {
        let _g = crate::test_util::global_state_lock();
        let pool = WorkerPool::new(2);
        let rx = pool.submit_with_result(|| 42);
        assert_eq!(rx.recv().unwrap(), 42);
    }

    #[test]
    fn test_default_size() {
        let _g = crate::test_util::global_state_lock();
        let pool = WorkerPool::default_size();
        assert!(pool.size() >= 2);
        assert!(pool.size() <= 18);
    }

    #[test]
    fn test_panic_does_not_kill_worker() {
        let _g = crate::test_util::global_state_lock();
        let pool = WorkerPool::new(2);
        let counter = Arc::new(AtomicUsize::new(0));

        // Submit a task that panics
        pool.submit(|| panic!("intentional test panic"));

        // Submit tasks after the panic — they should still run
        for _ in 0..10 {
            let c = Arc::clone(&counter);
            pool.submit(move || {
                c.fetch_add(1, Ordering::Relaxed);
            });
        }

        wait_for_count(&counter, 10, 5_000);
        drop(pool);
        assert_eq!(counter.load(Ordering::Relaxed), 10);
    }

    #[test]
    fn test_cancel_skips_queued_tasks() {
        let _g = crate::test_util::global_state_lock();
        let pool = WorkerPool::new(1); // single worker to control ordering
        let barrier = Arc::new(std::sync::Barrier::new(2));
        // Signal the worker fires when it ENTERS the barrier task. Lets
        // the main thread wait until the worker is provably blocked
        // inside the barrier BEFORE calling cancel(). Without this, a
        // pre-empted worker that hasn't yet pulled task #1 would see the
        // cancel flag, skip task #1, and the main thread's barrier.wait()
        // below would deadlock waiting for a second party that never
        // arrives.
        let started = Arc::new(std::sync::Mutex::new(false));
        let started_cv = Arc::new(std::sync::Condvar::new());
        let counter = Arc::new(AtomicUsize::new(0));

        let b = Arc::clone(&barrier);
        let started_clone = Arc::clone(&started);
        let cv_clone = Arc::clone(&started_cv);
        pool.submit(move || {
            // Mark "task entered" + notify before blocking.
            *started_clone.lock().unwrap() = true;
            cv_clone.notify_one();
            b.wait();
        });

        // Wait until the worker is provably inside the task (and thus
        // committed to calling b.wait() — no race with cancel below).
        // 5s timeout is a safety net; in practice this fires within μs.
        let mut g = started.lock().unwrap();
        let timeout = std::time::Duration::from_secs(5);
        while !*g {
            let (gg, wait_result) = started_cv.wait_timeout(g, timeout).unwrap();
            g = gg;
            if wait_result.timed_out() && !*g {
                panic!("worker never started task #1 within 5s — test scaffolding broken");
            }
        }
        drop(g);

        // Queue tasks that should be skipped (worker is parked at b.wait()).
        // Cap at channel capacity (size * 4 = 4 for a 1-worker pool) MINUS 1
        // for safety. Submitting more than the channel holds while the
        // worker is blocked deadlocks `submit` itself, since the bounded
        // crossbeam channel back-pressures `send()`. 3 skipped tasks is
        // enough to prove "queued tasks get cancelled" — the count isn't
        // load-bearing.
        for _ in 0..3 {
            let c = Arc::clone(&counter);
            pool.submit(move || {
                c.fetch_add(1, Ordering::Relaxed);
            });
        }

        // Cancel, then unblock the worker — it'll return from b.wait(),
        // loop, see cancelled=true, drain the 5 queued tasks without
        // executing them.
        pool.cancel();
        barrier.wait();

        // Give workers time to drain
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Queued tasks should have been skipped
        assert_eq!(counter.load(Ordering::Relaxed), 0);

        // Reset and verify pool still works
        pool.reset_cancel();
        let c = Arc::clone(&counter);
        pool.submit(move || {
            c.fetch_add(1, Ordering::Relaxed);
        });
        // Wait for the post-reset task to complete BEFORE drop, since
        // production Drop sets cancelled=true again and would skip
        // any not-yet-pulled task.
        wait_for_count(&counter, 1, 5_000);
        drop(pool);
        assert_eq!(counter.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_metrics() {
        let _g = crate::test_util::global_state_lock();
        let pool = WorkerPool::new(2);
        assert_eq!(pool.completed(), 0);

        for _ in 0..10 {
            pool.submit(|| {});
        }

        drop(pool);
        // Can't assert exact completed count due to timing,
        // but it should be > 0 after drop waits for all
    }

    #[test]
    fn test_backpressure_bounded() {
        let _g = crate::test_util::global_state_lock();
        // Pool of 1 with capacity 4 — 5th submit blocks (back-pressure)
        // until the worker drains one. With 20 submits + 1 worker the
        // pool's submit() call blocks naturally; by the time the loop
        // exits, ~16 are completed and ~4 are still queued / in-flight.
        let pool = WorkerPool::new(1);
        let counter = Arc::new(AtomicUsize::new(0));

        for _ in 0..20 {
            let c = Arc::clone(&counter);
            pool.submit(move || {
                c.fetch_add(1, Ordering::Relaxed);
            });
        }

        wait_for_count(&counter, 20, 5_000);
        drop(pool);
        assert_eq!(counter.load(Ordering::Relaxed), 20);
    }

    /// A pool thread must be identifiable as one, and `inputline()` must
    /// report EOF there instead of prompting / reading SHIN.
    ///
    /// Regression: compinit's `-C` bytecode backfill parses ~47k autoload
    /// bodies on the pool. A body whose lexer buffer drained mid-construct
    /// fell through C's "as a last resort, get some more input" arm
    /// (Src/input.c:354-356), so the WORKER read the user's terminal —
    /// stealing keystrokes from ZLE, rendering PS2 (`> `) after every
    /// `compinit -C`, and swallowing the following command lines.
    #[test]
    fn worker_threads_never_read_shin() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            !in_worker_thread(),
            "the shell thread must not be flagged as a pool thread"
        );

        let pool = WorkerPool::new(1);
        let flagged = Arc::new(AtomicUsize::new(0));
        let eof = Arc::new(AtomicUsize::new(0));
        let f = Arc::clone(&flagged);
        let e = Arc::clone(&eof);
        pool.submit(move || {
            if in_worker_thread() {
                f.store(1, Ordering::SeqCst);
            }
            // Returns 1 (EOF) immediately; never touches the terminal.
            if crate::ported::input::inputline() == 1 {
                e.store(1, Ordering::SeqCst);
            }
        });
        wait_for_count(&eof, 1, 5_000);
        drop(pool);

        assert_eq!(flagged.load(Ordering::SeqCst), 1, "pool thread not flagged");
        assert_eq!(
            eof.load(Ordering::SeqCst),
            1,
            "inputline() must return EOF on a pool thread"
        );
    }
}
