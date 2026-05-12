//! Phase-5 invariant test: bucket-1 holders preserve zsh's
//! per-evaluator semantics under multi-threading.
//!
//! What this test verifies (PORT_PLAN.md bucket-1 contract):
//!   - `Src/math.c` file-statics (`unary`, `noeval`, `lastbase`,
//!     `yyval`, `yylval`, operand `stack`) are scratch state for
//!     one math-expression evaluation. In single-threaded zsh, one
//!     global suffices. In zshrs, each worker thread evaluates
//!     independently — so each must have its own copy via
//!     `thread_local!`. Two threads each running `$((expr))` must
//!     not corrupt each other's operand stack or eval state.
//!
//! Why this test is load-bearing: if a future refactor silently
//! promotes a bucket-1 holder to a `static Mutex<...>` (the bucket-2
//! storage), parallel math evaluation either deadlocks under nested
//! recursion or serializes pointlessly. If it gets promoted further
//! to a non-locked global, threads corrupt each other's eval state.
//! Either failure mode is invisible in single-thread tests but breaks
//! every parallel-arith workload. This test fires the moment such a
//! demotion happens.
//!
//! See `docs/PORT_PLAN.md` Phase 5 + the "Bucket 1 — Per-evaluator C
//! globals" section for the canonical bucket-1 rationale.

use std::sync::{Arc, Barrier};
use std::thread;

use zsh::ported::math::mathevali;

const N_WORKERS: usize = 8;

/// Each worker evaluates a unique deterministic expression and
/// asserts the result. If any worker's TLS state leaked into
/// another's eval, the assertions would mismatch.
#[test]
fn parallel_math_eval_returns_correct_per_worker_results() {
    let barrier = Arc::new(Barrier::new(N_WORKERS));
    let mut handles = Vec::with_capacity(N_WORKERS);

    for tid in 0..N_WORKERS {
        let b = Arc::clone(&barrier);
        handles.push(thread::spawn(move || -> (usize, Result<i64, String>) {
            b.wait();
            // Distinct expression per worker. The 31 + 7*tid term
            // and the 2-3 nested precedence levels ensure parser
            // and operand-stack are exercised, not just the literal.
            let expr = format!("(31 + {} * 7) * 2 - {}", tid, tid);
            let expected = ((31 + 7 * (tid as i64)) * 2) - tid as i64;
            let got = mathevali(&expr);
            (tid, got.map(|v| (v, expected)).map_err(|e| e).and_then(|(g, e)| {
                if g == e { Ok(g) } else { Err(format!("tid={tid} expr=`{expr}` expected={e} got={g}")) }
            }))
        }));
    }
    for h in handles {
        let (tid, res) = h.join().expect("math worker panicked");
        assert!(
            res.is_ok(),
            "math TLS leak suspected: worker {tid} got {res:?}. \
             A correct bucket-1 port has each thread's M_* TLS \
             isolated; failure here means a bucket-1 holder was \
             demoted to shared (Mutex/static) storage and another \
             thread's eval state bled into this one. See \
             PORT_PLAN.md Phase 1 + Src/math.c file-statics.",
        );
    }
}

/// Nested-recursion stress: each worker recurses through a chain
/// of math evaluations. C's `mathevall` saves/restores its file-
/// statics around recursive calls (math.c:387 onward). The Rust
/// TLS port must preserve that save/restore *per thread*. If the
/// save/restore is global, two threads recursing simultaneously
/// would corrupt each other's saved stack-locals.
#[test]
fn nested_math_eval_does_not_corrupt_across_threads() {
    let n_iters = 32;
    let mut handles = Vec::with_capacity(N_WORKERS);

    for tid in 0..N_WORKERS {
        handles.push(thread::spawn(move || -> Vec<i64> {
            let mut results = Vec::with_capacity(n_iters);
            for i in 0..n_iters {
                // Nested expression — exercises the operand stack,
                // unary/binary ops, parentheses, mixed precedence.
                let expr = format!(
                    "((({tid} + {i}) * 2) - 1) * (({i} + 1) % 5 + 1)",
                    tid = tid, i = i
                );
                let expected: i64 = {
                    let a = (((tid as i64) + i as i64) * 2) - 1;
                    let b = ((i as i64 + 1) % 5) + 1;
                    a * b
                };
                let got = mathevali(&expr)
                    .unwrap_or_else(|e| panic!("tid={tid} i={i} eval error: {e}"));
                assert_eq!(
                    got, expected,
                    "math TLS corruption: tid={tid} i={i} expr=`{expr}` \
                     expected={expected} got={got}. Bucket-1 contract: \
                     each thread owns its own M_UNARY/M_NOEVAL/operand \
                     stack via thread_local.",
                );
                results.push(got);
            }
            results
        }));
    }
    for h in handles {
        h.join().expect("nested-math worker panicked");
    }
}

/// Pathological-expression stress: every worker evaluates the same
/// expression that hits multiple operators (unary, binary, paren,
/// bit, comparison, ternary). If the operand stack is shared, the
/// loser writes the winner's intermediate value mid-evaluation.
#[test]
fn parallel_same_complex_expression_is_deterministic() {
    let expr = "(5 + 3 * 2 - 1) | (4 & 7) ^ ((10 > 3) ? 100 : 200)";
    let expected = ((5 + 3 * 2 - 1) | (4 & 7)) ^ 100;

    let mut handles = Vec::with_capacity(N_WORKERS);
    for tid in 0..N_WORKERS {
        let e = expr.to_string();
        handles.push(thread::spawn(move || -> (usize, i64) {
            // Each worker computes the same thing 16x to multiply
            // the contention window.
            let mut last = 0;
            for _ in 0..16 {
                last = mathevali(&e)
                    .unwrap_or_else(|err| panic!("tid={tid} eval error: {err}"));
            }
            (tid, last)
        }));
    }
    for h in handles {
        let (tid, got) = h.join().expect("worker panicked");
        assert_eq!(
            got, expected,
            "math TLS corruption: tid={tid} expr=`{expr}` got={got} \
             expected={expected}. Same expression, deterministic \
             answer — divergence means one thread's operand stack \
             stomped another's mid-eval. Promoting bucket-1 M_* \
             holders to shared storage causes exactly this failure.",
        );
    }
}
