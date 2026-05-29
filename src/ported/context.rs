//! context.c - context save and restore
//!
//! Port of Src/context.c
//!
//! This short file provides a home for the stack of saved contexts.
//! The actions for saving and restoring are encapsulated within
//! individual modules. After P7-P8 dissolved the ZshLexer and ZshParser structs into
//! free ported + thread_locals, the save/restore signatures simplified —
//! the lexer/parser parameters went away. Callers just call zcontext_*
//! and the underlying thread_local state is what's saved/restored.

use super::parse::ParseStack;
use crate::hist::{hist_context_restore, hist_context_save};
use crate::lex::{lex_context_restore, lex_context_save};
use crate::parse::{parse_context_restore, parse_context_save};
use crate::ported::zsh_h::{hist_stack, lex_stack, ZCONTEXT_HIST, ZCONTEXT_LEX, ZCONTEXT_PARSE};
use crate::signals_h::{queue_signals, unqueue_signals};
use crate::DPUTS;
use std::sync::Mutex;

/// Port of `struct context_stack` from Src/context.c:38-44.
#[allow(non_camel_case_types)]
pub struct context_stack {
    // c:52
    pub next: Option<Box<context_stack>>, // c:52
    pub hist_stack: hist_stack,           // c:52
    pub lex_stack: lex_stack,             // c:52
    pub parse_stack: ParseStack,          // c:52
}

/// Port of `void zcontext_save_partial(int parts)` from Src/context.c:52.
#[allow(non_snake_case)]
pub fn zcontext_save_partial(parts: i32) {
    // c:52
    queue_signals(); // c:52

    let mut cs = Box::new(context_stack {
        // c:58
        next: None,
        hist_stack: hist_stack {
            histactive: 0,
            histdone: 0,
            stophist: 0,
            hlinesz: 0,
            defev: 0,
            hline: None,
            hptr: None,
            chwords: Vec::new(),
            chwordlen: 0,
            chwordpos: 0,
            csp: 0,
            hist_keep_comment: 0,
        },
        lex_stack: lex_stack::default(),
        parse_stack: ParseStack::default(),
    });

    let mut head = cstack.lock().unwrap();

    let toplevel: i32 = if head.is_none() { 1 } else { 0 }; // !cstack
    if (parts & ZCONTEXT_HIST) != 0 {
        // c:60
        hist_context_save(&mut cs.hist_stack, toplevel); // c:61
    }
    if (parts & ZCONTEXT_LEX) != 0 {
        // c:63
        lex_context_save(&mut cs.lex_stack);
    }
    if (parts & ZCONTEXT_PARSE) != 0 {
        // c:80
        parse_context_save(&mut cs.parse_stack);
    }

    cs.next = head.take(); // c:89
    *head = Some(cs); // c:89

    unqueue_signals(); // c:89
}

/// Port of `void zcontext_save(void)` from Src/context.c:80.
pub fn zcontext_save() {
    // c:80
    zcontext_save_partial(ZCONTEXT_HIST | ZCONTEXT_LEX | ZCONTEXT_PARSE);
}

/// Port of `void zcontext_restore_partial(int parts)` from Src/context.c:89.
pub fn zcontext_restore_partial(parts: i32) {
    // c:89
    let mut head = cstack.lock().unwrap();
    // c:93 — DPUTS(!cstack, "BUG: zcontext_restore() without zcontext_save()")
    DPUTS!(
        head.is_none(),
        "BUG: zcontext_restore() without zcontext_save()"
    ); // c:93
    let mut cs = match head.take() {
        // c:91
        Some(cs) => cs,
        None => {
            return;
        }
    };

    queue_signals(); // c:95
    *head = cs.next.take(); // c:96
    let toplevel: i32 = if head.is_none() { 1 } else { 0 };

    if (parts & ZCONTEXT_HIST) != 0 {
        // c:98
        hist_context_restore(&cs.hist_stack, toplevel); // c:99
    }
    if (parts & ZCONTEXT_LEX) != 0 {
        // c:101
        lex_context_restore(&mut cs.lex_stack);
    }
    if (parts & ZCONTEXT_PARSE) != 0 {
        // c:117
        parse_context_restore(&cs.parse_stack);
    }

    drop(cs); // c:117

    unqueue_signals(); // c:117
}

/// Port of `void zcontext_restore(void)` from Src/context.c:117.
pub fn zcontext_restore() {
    // c:117
    zcontext_restore_partial(ZCONTEXT_HIST | ZCONTEXT_LEX | ZCONTEXT_PARSE);
}

/// Port of `static struct context_stack *cstack` from Src/context.c:52.
static cstack: Mutex<Option<Box<context_stack>>> = Mutex::new(None); // c:52

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lex::{lex_init, set_toklineno, toklineno, LEX_DBPARENS};

    fn reset_cstack() {
        *cstack.lock().unwrap() = None;
    }

    #[test]
    fn save_restore_balances_stack() {
        let _g = crate::test_util::global_state_lock();
        reset_cstack();
        lex_init("");
        zcontext_save();
        assert!(cstack.lock().unwrap().is_some());
        zcontext_restore();
        assert!(cstack.lock().unwrap().is_none());
    }

    #[test]
    fn nested_saves_pop_lifo() {
        let _g = crate::test_util::global_state_lock();
        reset_cstack();
        lex_init("");
        zcontext_save();
        zcontext_save();
        zcontext_restore();
        assert!(cstack.lock().unwrap().is_some());
        zcontext_restore();
        assert!(cstack.lock().unwrap().is_none());
    }

    #[test]
    fn restore_without_save_is_noop() {
        let _g = crate::test_util::global_state_lock();
        reset_cstack();
        lex_init("");
        zcontext_restore();
        assert!(cstack.lock().unwrap().is_none());
    }

    #[test]
    fn lex_save_restore_roundtrips_state() {
        let _g = crate::test_util::global_state_lock();
        reset_cstack();
        lex_init("echo hello");
        LEX_DBPARENS.set(true);
        set_toklineno(42);
        zcontext_save();
        assert!(LEX_DBPARENS.with(|c| c.get()));
        assert_eq!(toklineno(), 42);
        zcontext_restore();
        assert!(LEX_DBPARENS.with(|c| c.get()));
        assert_eq!(toklineno(), 42);
    }

    /// `Src/zsh.h:491-495` — `ZCONTEXT_HIST = (1<<0)`,
    /// `ZCONTEXT_LEX = (1<<1)`, `ZCONTEXT_PARSE = (1<<2)`. The three
    /// bits must be distinct and non-overlapping; `zcontext_save_partial`
    /// at c:60/63/66 AND-tests each independently and a flag collision
    /// would silently double-save one substrate and skip another on
    /// every nested context (heredoc-inside-eval, `read -E`, …).
    #[test]
    fn zcontext_flag_bits_are_distinct_and_nonzero() {
        let _g = crate::test_util::global_state_lock();
        // Pin the exact C constants from Src/zsh.h:491-495.
        assert_eq!(ZCONTEXT_HIST, 1 << 0);
        assert_eq!(ZCONTEXT_LEX, 1 << 1);
        assert_eq!(ZCONTEXT_PARSE, 1 << 2);
        // Bitfield discipline: no overlapping bits.
        assert_eq!(ZCONTEXT_HIST & ZCONTEXT_LEX, 0);
        assert_eq!(ZCONTEXT_HIST & ZCONTEXT_PARSE, 0);
        assert_eq!(ZCONTEXT_LEX & ZCONTEXT_PARSE, 0);
    }

    /// `Src/context.c:52-72` — `zcontext_save_partial(parts)` allocates
    /// the frame BEFORE the `parts &` gates at c:60/63/66, then pushes
    /// at c:70-71 unconditionally (`cs->next = cstack; cstack = cs`).
    /// `parts == 0` is a legitimate caller pattern (probe-save); the
    /// push must still fire. Regression that early-exits on a zero
    /// mask would mis-balance the stack for any such caller.
    #[test]
    fn zcontext_save_partial_with_zero_mask_still_pushes_frame() {
        let _g = crate::test_util::global_state_lock();
        reset_cstack();
        lex_init("");
        zcontext_save_partial(0);
        assert!(
            cstack.lock().unwrap().is_some(),
            "c:70-71 push must fire even when no parts requested"
        );
        zcontext_restore_partial(0);
        assert!(
            cstack.lock().unwrap().is_none(),
            "c:96 pop must mirror the push regardless of parts mask"
        );
    }

    /// `Src/context.c:89-96` — `zcontext_restore_partial` reads
    /// `cs = cstack` at c:91 and a `DPUTS(!cstack, ...)` at c:93 fires
    /// in C debug builds. Release C dereferences a NULL `cs` — UB —
    /// but the Rust port chose to silently no-op on empty stack
    /// (`head.take()` returns None → early return). Pin the no-op so
    /// a refactor doesn't suddenly start panicking on unbalanced
    /// restore calls.
    #[test]
    fn zcontext_restore_partial_on_empty_stack_is_noop() {
        let _g = crate::test_util::global_state_lock();
        reset_cstack();
        zcontext_restore_partial(0);
        zcontext_restore_partial(ZCONTEXT_HIST);
        assert!(cstack.lock().unwrap().is_none());
    }

    /// `Src/context.c:52` — Deep LIFO check. 5 saves pushed; 5
    /// restores must drain to empty. Pin the depth-handling so a
    /// regression that uses a fixed-size buffer instead of the
    /// linked stack would surface on the third+ push.
    #[test]
    fn deep_save_restore_lifo_drains_to_empty() {
        let _g = crate::test_util::global_state_lock();
        reset_cstack();
        lex_init("");
        for _ in 0..5 {
            zcontext_save();
        }
        // Stack now has 5 frames
        for i in (0..5).rev() {
            assert!(
                cstack.lock().unwrap().is_some(),
                "stack must still have entries before restore #{}",
                5 - i
            );
            zcontext_restore();
        }
        assert!(
            cstack.lock().unwrap().is_none(),
            "5 restores must drain the 5 saves to empty"
        );
    }

    /// `Src/context.c:80` — `zcontext_save()` is a convenience
    /// wrapper for `zcontext_save_partial(ALL)`. Pin the alias
    /// contract: a full-mask partial-save followed by any restore
    /// yields the same final state as save→restore.
    #[test]
    fn zcontext_save_equals_save_partial_full_mask() {
        let _g = crate::test_util::global_state_lock();
        let full = ZCONTEXT_HIST | ZCONTEXT_LEX | ZCONTEXT_PARSE;

        reset_cstack();
        lex_init("");
        zcontext_save();
        zcontext_restore();
        let after_full = cstack.lock().unwrap().is_none();

        reset_cstack();
        lex_init("");
        zcontext_save_partial(full);
        zcontext_restore_partial(full);
        let after_partial = cstack.lock().unwrap().is_none();

        assert_eq!(
            after_full, after_partial,
            "save/restore must equal save_partial(ALL)/restore_partial(ALL)"
        );
    }

    /// `Src/context.c:52` — Many save calls with NO matching restores
    /// must not corrupt or panic, just grow the stack. Pin defensive
    /// behavior for shell-script abort paths where partial state
    /// rewinds may skip restores.
    #[test]
    fn many_saves_without_restore_grow_stack_safely() {
        let _g = crate::test_util::global_state_lock();
        reset_cstack();
        lex_init("");
        for _ in 0..20 {
            zcontext_save();
        }
        // Stack should still be intact, top frame accessible
        assert!(cstack.lock().unwrap().is_some());
        // Now drain — must not panic
        for _ in 0..20 {
            zcontext_restore();
        }
        assert!(cstack.lock().unwrap().is_none());
    }

    // ─── zsh-corpus pins for context save/restore ───────────────────

    /// `zcontext_save` then `zcontext_restore` leaves the stack empty.
    #[test]
    fn context_corpus_save_then_restore_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        reset_cstack();
        lex_init("");
        zcontext_save();
        zcontext_restore();
        assert!(
            cstack.lock().unwrap().is_none(),
            "save+restore leaves no stack"
        );
    }

    /// Nested save/restore is correctly LIFO.
    #[test]
    fn context_corpus_nested_save_restore_lifo() {
        let _g = crate::test_util::global_state_lock();
        reset_cstack();
        lex_init("");
        zcontext_save();
        zcontext_save();
        zcontext_save();
        zcontext_restore();
        zcontext_restore();
        zcontext_restore();
        assert!(cstack.lock().unwrap().is_none(), "all 3 levels drained");
    }

    /// `zcontext_save_partial(1)` + `zcontext_restore_partial(1)`
    /// round-trips without panic.
    #[test]
    fn context_corpus_save_partial_round_trip() {
        let _g = crate::test_util::global_state_lock();
        reset_cstack();
        lex_init("");
        zcontext_save_partial(1);
        zcontext_restore_partial(1);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/context.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:80 — `zcontext_save` pushes onto stack (cstack becomes Some).
    #[test]
    fn zcontext_save_pushes_onto_stack() {
        let _g = crate::test_util::global_state_lock();
        reset_cstack();
        lex_init("");
        zcontext_save();
        assert!(
            cstack.lock().unwrap().is_some(),
            "save should leave cstack with one frame"
        );
        zcontext_restore();
    }

    /// c:117 — `zcontext_restore` pops stack (cstack returns to None).
    #[test]
    fn zcontext_restore_pops_to_none_when_balanced() {
        let _g = crate::test_util::global_state_lock();
        reset_cstack();
        lex_init("");
        zcontext_save();
        zcontext_restore();
        assert!(
            cstack.lock().unwrap().is_none(),
            "single save+restore returns cstack to None"
        );
    }

    /// c:80 — multiple saves build a stack of N frames.
    #[test]
    fn zcontext_save_multiple_builds_stack() {
        let _g = crate::test_util::global_state_lock();
        reset_cstack();
        lex_init("");
        zcontext_save();
        zcontext_save();
        zcontext_save();
        // Walk the linked-list and count.
        let head = cstack.lock().unwrap();
        let mut count = 0;
        let mut cur = head.as_ref();
        while let Some(node) = cur {
            count += 1;
            cur = node.next.as_ref();
        }
        drop(head);
        assert_eq!(count, 3, "3 saves → 3-deep stack");
        zcontext_restore();
        zcontext_restore();
        zcontext_restore();
    }

    /// c:52 — `zcontext_save_partial(0)` saves NOTHING from the
    /// constituent sub-stacks but still pushes a frame.
    #[test]
    fn zcontext_save_partial_zero_still_pushes_frame() {
        let _g = crate::test_util::global_state_lock();
        reset_cstack();
        lex_init("");
        zcontext_save_partial(0);
        assert!(
            cstack.lock().unwrap().is_some(),
            "even parts=0 pushes a frame for restore symmetry"
        );
        zcontext_restore_partial(0);
    }

    /// c:89 — `zcontext_restore_partial(0)` on empty stack DPUTS warns
    /// but doesn't panic.
    #[test]
    fn zcontext_restore_on_empty_no_panic() {
        let _g = crate::test_util::global_state_lock();
        reset_cstack();
        // No prior save — restore should not panic.
        zcontext_restore_partial(0);
    }

    /// c:52 — `zcontext_save_partial(ZCONTEXT_HIST)` + restore round-trip.
    #[test]
    fn zcontext_save_hist_only_round_trip() {
        let _g = crate::test_util::global_state_lock();
        reset_cstack();
        lex_init("");
        zcontext_save_partial(crate::ported::zsh_h::ZCONTEXT_HIST);
        zcontext_restore_partial(crate::ported::zsh_h::ZCONTEXT_HIST);
        assert!(cstack.lock().unwrap().is_none());
    }

    /// c:52 — `zcontext_save_partial(ZCONTEXT_LEX)` + restore round-trip.
    #[test]
    fn zcontext_save_lex_only_round_trip() {
        let _g = crate::test_util::global_state_lock();
        reset_cstack();
        lex_init("");
        zcontext_save_partial(crate::ported::zsh_h::ZCONTEXT_LEX);
        zcontext_restore_partial(crate::ported::zsh_h::ZCONTEXT_LEX);
        assert!(cstack.lock().unwrap().is_none());
    }

    /// c:52 — `zcontext_save_partial(ZCONTEXT_PARSE)` + restore round-trip.
    #[test]
    fn zcontext_save_parse_only_round_trip() {
        let _g = crate::test_util::global_state_lock();
        reset_cstack();
        lex_init("");
        zcontext_save_partial(crate::ported::zsh_h::ZCONTEXT_PARSE);
        zcontext_restore_partial(crate::ported::zsh_h::ZCONTEXT_PARSE);
        assert!(cstack.lock().unwrap().is_none());
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/context.c partial save/restore
    // semantics + ZCONTEXT_* flag invariants.
    // ═══════════════════════════════════════════════════════════════════

    /// c:491-495 — ZCONTEXT_HIST/LEX/PARSE are distinct single bits.
    #[test]
    fn zcontext_flag_bits_pin_to_one_two_four() {
        assert_eq!(crate::ported::zsh_h::ZCONTEXT_HIST, 1, "c:491 = 1<<0");
        assert_eq!(crate::ported::zsh_h::ZCONTEXT_LEX, 2, "c:493 = 1<<1");
        assert_eq!(crate::ported::zsh_h::ZCONTEXT_PARSE, 4, "c:495 = 1<<2");
    }

    /// c:491-495 — every ZCONTEXT_* is a single bit (power of two).
    #[test]
    fn zcontext_flags_are_single_bits() {
        for &v in &[
            crate::ported::zsh_h::ZCONTEXT_HIST,
            crate::ported::zsh_h::ZCONTEXT_LEX,
            crate::ported::zsh_h::ZCONTEXT_PARSE,
        ] {
            assert!((v as u32).is_power_of_two(), "{} must be a single bit", v);
        }
    }

    /// c:52 — `zcontext_save_partial(0)` followed by restore should
    /// drain the stack to empty (the c:60-78 branches all skip when
    /// no flags match, but the frame still pushes per c:89).
    #[test]
    fn zcontext_save_partial_zero_round_trip_pops() {
        let _g = crate::test_util::global_state_lock();
        reset_cstack();
        lex_init("");
        zcontext_save_partial(0);
        assert!(cstack.lock().unwrap().is_some(), "zero-mask still pushes");
        zcontext_restore_partial(0);
        assert!(cstack.lock().unwrap().is_none(), "restore drains to None");
    }

    /// c:52 — save HIST then restore with PARSE alone should still
    /// pop the frame (c:91 unconditional take), but NOT restore HIST.
    /// Pins the asymmetric-mask edge case from c:96.
    #[test]
    fn zcontext_save_hist_restore_with_parse_mask_pops_frame() {
        let _g = crate::test_util::global_state_lock();
        reset_cstack();
        lex_init("");
        zcontext_save_partial(crate::ported::zsh_h::ZCONTEXT_HIST);
        assert!(cstack.lock().unwrap().is_some());
        zcontext_restore_partial(crate::ported::zsh_h::ZCONTEXT_PARSE);
        assert!(cstack.lock().unwrap().is_none(), "frame popped regardless");
    }

    /// c:80 — `zcontext_save` and `zcontext_save_partial(HIST|LEX|PARSE)`
    /// must each push exactly one frame.
    #[test]
    fn zcontext_save_shorthand_pushes_one_frame() {
        let _g = crate::test_util::global_state_lock();
        reset_cstack();
        lex_init("");
        zcontext_save();
        // Stack must have one frame (top != None, top.next == None).
        {
            let head = cstack.lock().unwrap();
            assert!(head.is_some(), "save pushed a frame");
            assert!(head.as_ref().unwrap().next.is_none(), "stack depth = 1");
        }
        zcontext_restore();
        assert!(cstack.lock().unwrap().is_none());

        zcontext_save_partial(
            crate::ported::zsh_h::ZCONTEXT_HIST
                | crate::ported::zsh_h::ZCONTEXT_LEX
                | crate::ported::zsh_h::ZCONTEXT_PARSE,
        );
        {
            let head = cstack.lock().unwrap();
            assert!(head.is_some(), "full-mask save_partial pushed a frame");
            assert!(head.as_ref().unwrap().next.is_none(), "stack depth = 1");
        }
        zcontext_restore();
    }

    /// c:117 — `zcontext_restore` shorthand drains exactly one frame.
    #[test]
    fn zcontext_restore_drains_exactly_one_frame() {
        let _g = crate::test_util::global_state_lock();
        reset_cstack();
        lex_init("");
        zcontext_save();
        zcontext_save();
        // Two pushes.
        zcontext_restore();
        // One frame remains.
        assert!(cstack.lock().unwrap().is_some());
        zcontext_restore();
        assert!(cstack.lock().unwrap().is_none());
    }

    /// c:91 — `zcontext_restore` shorthand on empty stack is a no-op
    /// (matches DPUTS-trip path: warn + return without panic).
    #[test]
    fn zcontext_restore_shorthand_on_empty_no_panic() {
        let _g = crate::test_util::global_state_lock();
        reset_cstack();
        zcontext_restore();
        assert!(cstack.lock().unwrap().is_none());
    }

    /// c:52 — `ZCONTEXT_HIST | ZCONTEXT_LEX` combo (no PARSE) round-trips.
    #[test]
    fn zcontext_save_hist_lex_combo_round_trip() {
        let _g = crate::test_util::global_state_lock();
        reset_cstack();
        lex_init("");
        let mask = crate::ported::zsh_h::ZCONTEXT_HIST | crate::ported::zsh_h::ZCONTEXT_LEX;
        zcontext_save_partial(mask);
        zcontext_restore_partial(mask);
        assert!(cstack.lock().unwrap().is_none());
    }

    /// c:52 — `ZCONTEXT_LEX | ZCONTEXT_PARSE` combo (no HIST) round-trips.
    #[test]
    fn zcontext_save_lex_parse_combo_round_trip() {
        let _g = crate::test_util::global_state_lock();
        reset_cstack();
        lex_init("");
        let mask = crate::ported::zsh_h::ZCONTEXT_LEX | crate::ported::zsh_h::ZCONTEXT_PARSE;
        zcontext_save_partial(mask);
        zcontext_restore_partial(mask);
        assert!(cstack.lock().unwrap().is_none());
    }

    /// c:91 — multiple restores on empty stack stay no-op (no DPUTS panic).
    #[test]
    fn many_restores_on_empty_no_panic() {
        let _g = crate::test_util::global_state_lock();
        reset_cstack();
        for _ in 0..10 {
            zcontext_restore_partial(0);
        }
        assert!(cstack.lock().unwrap().is_none());
    }

    /// c:91 — DPUTS WARN-only: restore on empty stack must not crash.
    #[test]
    fn restore_on_empty_via_full_shorthand_no_panic() {
        let _g = crate::test_util::global_state_lock();
        reset_cstack();
        zcontext_restore();
        zcontext_restore_partial(crate::ported::zsh_h::ZCONTEXT_HIST);
        zcontext_restore_partial(
            crate::ported::zsh_h::ZCONTEXT_HIST | crate::ported::zsh_h::ZCONTEXT_LEX,
        );
        assert!(cstack.lock().unwrap().is_none());
    }
}
