//! Phase-5 invariant test: bucket-2 holders preserve zsh's single-process
//! "all callers see the same data" semantics under multi-threading.
//!
//! What this test verifies (PORT_PLAN.md bucket-2 contract):
//!   - `Src/params.c paramtab` — `setsparam("FOO", "bar")` from any
//!     worker thread must be visible via `getsparam("FOO")` from any
//!     other thread.
//!   - `Src/options.c opts[]` — `opt_state_set("ERREXIT", true)` from
//!     any worker thread must be visible via `opt_state_get("ERREXIT")`
//!     from any other thread.
//!
//! Why this test is load-bearing: if a future refactor silently demotes
//! a bucket-2 holder to `thread_local!` (the bucket-1 storage), the
//! single-shell-identity contract breaks — thread A's `export FOO=1`
//! becomes invisible to thread B. The bug is invisible in single-thread
//! tests but corrupts every multi-worker workload. This test fires the
//! moment that demotion happens.
//!
//! See `docs/PORT_PLAN.md` Phase 5 + the "Where 100% Faithful Port
//! Cannot Work" section for the canonical bucket-2 rationale.

use std::sync::{Arc, Barrier};
use std::thread;

use zsh::ported::params::{getsparam, setsparam};
use zsh::ported::options::{opt_state_get, opt_state_set};

const N_WORKERS: usize = 8;

/// Each worker writes a unique paramtab entry. After all writers
/// finish, an observer (the main thread) reads every entry and
/// asserts visibility. Failure mode caught: paramtab silently demoted
/// to `thread_local!` — each writer's key would survive only inside
/// its own thread.
#[test]
fn paramtab_writes_visible_across_threads() {
    let barrier = Arc::new(Barrier::new(N_WORKERS));
    let mut handles = Vec::with_capacity(N_WORKERS);

    for tid in 0..N_WORKERS {
        let b = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            b.wait(); // align all writers
            let key = format!("ZSHRS_SHARED_PARAM_{}", tid);
            let val = format!("worker_{}_value", tid);
            setsparam(&key, &val);
        }));
    }
    for h in handles {
        h.join().expect("worker panicked");
    }

    // Observer reads from the main thread — would fail with TLS storage.
    for tid in 0..N_WORKERS {
        let key = format!("ZSHRS_SHARED_PARAM_{}", tid);
        let expected = format!("worker_{}_value", tid);
        let actual = getsparam(&key);
        assert_eq!(
            actual,
            Some(expected.clone()),
            "paramtab regression: key {key} written by worker {tid} \
             not visible to observer (got {actual:?}, expected {expected:?}). \
             Likely cause: paramtab demoted from RwLock to thread_local. \
             See PORT_PLAN.md Phase 3 + Src/params.c:515 mod_export \
             HashTable paramtab.",
        );
    }
}

/// Cross-thread reader sees a write from a different thread. This is
/// the tighter invariant: writer and reader are *different* threads,
/// not just same-thread-after-join. TLS would always fail this.
#[test]
fn paramtab_writes_observed_by_other_worker() {
    let barrier = Arc::new(Barrier::new(2));
    let writer_barrier = Arc::clone(&barrier);
    let reader_barrier = Arc::clone(&barrier);

    let writer = thread::spawn(move || {
        setsparam("ZSHRS_CROSS_THREAD_KEY", "written_by_writer");
        writer_barrier.wait(); // signal reader can proceed
    });

    let reader = thread::spawn(move || {
        reader_barrier.wait(); // wait for writer to finish
        getsparam("ZSHRS_CROSS_THREAD_KEY")
    });

    writer.join().expect("writer panicked");
    let observed = reader.join().expect("reader panicked");
    assert_eq!(
        observed,
        Some("written_by_writer".to_string()),
        "paramtab regression: cross-thread write not observed \
         (got {observed:?}). The bucket-2 contract guarantees one shared \
         paramtab across workers — see PORT_PLAN.md.",
    );
}

/// Same invariant for the option-state map (`Src/options.c opts[]`).
/// `setopt ERREXIT` from any thread must be visible everywhere.
#[test]
fn options_writes_visible_across_threads() {
    let barrier = Arc::new(Barrier::new(N_WORKERS));
    let mut handles = Vec::with_capacity(N_WORKERS);

    for tid in 0..N_WORKERS {
        let b = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            b.wait();
            let opt = format!("ZSHRS_SHARED_OPT_{}", tid);
            opt_state_set(&opt, true);
        }));
    }
    for h in handles {
        h.join().expect("worker panicked");
    }

    for tid in 0..N_WORKERS {
        let opt = format!("ZSHRS_SHARED_OPT_{}", tid);
        let actual = opt_state_get(&opt);
        assert_eq!(
            actual,
            Some(true),
            "options regression: opt `{opt}` set by worker {tid} not \
             visible to observer (got {actual:?}). Likely cause: \
             OPTS_LIVE demoted from RwLock to thread_local. See \
             PORT_PLAN.md Phase 3 + Src/options.c:36 opts[].",
        );
    }
}

/// Parallel reads + parallel writes from N threads stress the
/// RwLock implementation. Catches deadlocks and torn reads if the
/// holder ever loses its lock primitive.
#[test]
fn paramtab_parallel_read_write_stress() {
    let n_iters = 64;
    let mut handles = Vec::with_capacity(N_WORKERS * 2);

    // N writers, each writing distinct keys, n_iters times.
    for tid in 0..N_WORKERS {
        handles.push(thread::spawn(move || {
            for i in 0..n_iters {
                let key = format!("ZSHRS_STRESS_W_{}_{}", tid, i);
                let val = format!("v_{}_{}", tid, i);
                setsparam(&key, &val);
            }
        }));
    }
    // N readers, hammering the same key space.
    for tid in 0..N_WORKERS {
        handles.push(thread::spawn(move || {
            for i in 0..n_iters {
                let key = format!("ZSHRS_STRESS_W_{}_{}", tid, i);
                let _ = getsparam(&key);
            }
        }));
    }
    for h in handles {
        h.join().expect("stress worker panicked — possible deadlock");
    }

    // Final visibility check: at least the last-written key from each
    // writer must be observable.
    for tid in 0..N_WORKERS {
        let key = format!("ZSHRS_STRESS_W_{}_{}", tid, n_iters - 1);
        let expected = format!("v_{}_{}", tid, n_iters - 1);
        assert_eq!(getsparam(&key), Some(expected));
    }
}
