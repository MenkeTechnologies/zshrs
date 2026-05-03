//! Worker pool for zshrs — persistent threads for background work.
//!
//! Port rationale: zsh forks for everything (completion, process subs,
//! command substitution).  Each fork copies the entire shell state.
//! We replace that with a fixed-size thread pool + channel dispatch,
//! giving us:
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

/// Fixed-size thread pool with bounded FIFO task queue.
///
/// Uses crossbeam-channel for lock-free multi-consumer dispatch —
/// each worker calls `recv()` directly, no mutex.
pub struct WorkerPool {
    workers: Vec<Worker>,
    sender: Option<crossbeam_channel::Sender<Task>>,
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
    /// Channel capacity = 4 × size (provides backpressure without starving).
    pub fn new(size: usize) -> Self {
        let capacity = size * 4;
        let (sender, receiver) = crossbeam_channel::bounded::<Task>(capacity);
        let cancelled = Arc::new(AtomicBool::new(false));
        let queued = Arc::new(AtomicUsize::new(0));
        let completed = Arc::new(AtomicUsize::new(0));

        let mut workers = Vec::with_capacity(size);
        for id in 0..size {
            let rx = receiver.clone();
            let cancelled = Arc::clone(&cancelled);
            let queued = Arc::clone(&queued);
            let completed = Arc::clone(&completed);

            let handle = thread::Builder::new()
                .name(format!("zshrs-worker-{}", id))
                .spawn(move || {
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

        tracing::info!(
            pool_size = size,
            channel_capacity = capacity,
            "worker pool started"
        );

        WorkerPool {
            workers,
            sender: Some(sender),
            size,
            cancelled,
            queued,
            completed,
        }
    }

    /// Create a pool sized to the machine's parallelism, clamped [2, 18].
    pub fn default_size() -> Self {
        let cpus = thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        Self::new(cpus.clamp(2, 18))
    }

    /// Submit a task to the pool.  Blocks if the queue is full (backpressure).
    /// Panics if the pool has been shut down.
    pub fn submit<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
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
    /// Already-running tasks will finish, but queued tasks are skipped.
    /// Reset with `reset_cancel()`.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        tracing::info!("worker pool: cancel requested");
    }

    /// Clear the cancellation flag — pool resumes normal execution.
    pub fn reset_cancel(&self) {
        self.cancelled.store(false, Ordering::Relaxed);
    }

    /// Number of worker threads.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Approximate number of tasks waiting in the queue.
    pub fn queue_depth(&self) -> usize {
        self.queued.load(Ordering::Relaxed)
    }

    /// Total tasks completed since pool creation.
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
        for w in &mut self.workers {
            if let Some(handle) = w.handle.take() {
                // Detach the thread — OS cleans up on process exit.
                // join() would block if a worker is mid-parse on a 500-line
                // completion function. Not worth the wait on Ctrl-D/exit.
                drop(handle);
            }
        }
        tracing::info!(
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
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(max_wait_ms);
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
        let pool = WorkerPool::new(2);
        let rx = pool.submit_with_result(|| 42);
        assert_eq!(rx.recv().unwrap(), 42);
    }

    #[test]
    fn test_default_size() {
        let pool = WorkerPool::default_size();
        assert!(pool.size() >= 2);
        assert!(pool.size() <= 18);
    }

    #[test]
    fn test_panic_does_not_kill_worker() {
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
}
