//! Direct port of `Src/hashtable.h` — header file for hash table
//! handling code.
//!
//! This file mirrors the entire `hashtable.h` header content. The
//! implementation lives in `crate::ported::hashtable` (port of
//! `Src/hashtable.c`).
//!
//! Per Src/hashtable.h:30-67 — the BIN_* dispatch IDs are used by
//! handler functions that handle more than one builtin (e.g.
//! `bin_break` handles BIN_BREAK / BIN_CONTINUE / BIN_RETURN /
//! BIN_EXIT / BIN_LOGOUT). Builtins not overloaded (like
//! `compctl`) don't get a number.

// =====================================================================
// Builtin function numbers — Src/hashtable.h:34-66.
// =====================================================================
/// `BIN_TYPESET` constant.
pub const BIN_TYPESET: i32 = 0; // c:34
/// `BIN_BG` constant.
pub const BIN_BG: i32 = 1; // c:35
/// `BIN_FG` constant.
pub const BIN_FG: i32 = 2; // c:36
/// `BIN_JOBS` constant.
pub const BIN_JOBS: i32 = 3; // c:37
/// `BIN_WAIT` constant.
pub const BIN_WAIT: i32 = 4; // c:38
/// `BIN_DISOWN` constant.
pub const BIN_DISOWN: i32 = 5; // c:39
/// `BIN_BREAK` constant.
pub const BIN_BREAK: i32 = 6; // c:40
/// `BIN_CONTINUE` constant.
pub const BIN_CONTINUE: i32 = 7; // c:41
/// `BIN_EXIT` constant.
pub const BIN_EXIT: i32 = 8; // c:42
/// `BIN_RETURN` constant.
pub const BIN_RETURN: i32 = 9; // c:43
/// `BIN_CD` constant.
pub const BIN_CD: i32 = 10; // c:44
/// `BIN_POPD` constant.
pub const BIN_POPD: i32 = 11; // c:45
/// `BIN_PUSHD` constant.
pub const BIN_PUSHD: i32 = 12; // c:46
/// `BIN_PRINT` constant.
pub const BIN_PRINT: i32 = 13; // c:47
/// `BIN_EVAL` constant.
pub const BIN_EVAL: i32 = 14; // c:48
/// `BIN_SCHED` constant.
pub const BIN_SCHED: i32 = 15; // c:49
/// `BIN_FC` constant.
pub const BIN_FC: i32 = 16; // c:50
/// `BIN_R` constant.
pub const BIN_R: i32 = 17; // c:51
/// `BIN_PUSHLINE` constant.
pub const BIN_PUSHLINE: i32 = 18; // c:52
/// `BIN_LOGOUT` constant.
pub const BIN_LOGOUT: i32 = 19; // c:53
/// `BIN_TEST` constant.
pub const BIN_TEST: i32 = 20; // c:54
/// `BIN_BRACKET` constant.
pub const BIN_BRACKET: i32 = 21; // c:55
/// `BIN_READONLY` constant.
pub const BIN_READONLY: i32 = 22; // c:56
/// `BIN_ECHO` constant.
pub const BIN_ECHO: i32 = 23; // c:57
/// `BIN_DISABLE` constant.
pub const BIN_DISABLE: i32 = 24; // c:58
/// `BIN_ENABLE` constant.
pub const BIN_ENABLE: i32 = 25; // c:59
/// `BIN_PRINTF` constant.
pub const BIN_PRINTF: i32 = 26; // c:60
/// `BIN_COMMAND` constant.
pub const BIN_COMMAND: i32 = 27; // c:61
/// `BIN_UNHASH` constant.
pub const BIN_UNHASH: i32 = 28; // c:62
/// `BIN_UNALIAS` constant.
pub const BIN_UNALIAS: i32 = 29; // c:63
/// `BIN_UNFUNCTION` constant.
pub const BIN_UNFUNCTION: i32 = 30; // c:64
/// `BIN_UNSET` constant.
pub const BIN_UNSET: i32 = 31; // c:65
/// `BIN_EXPORT` constant.
pub const BIN_EXPORT: i32 = 32; // c:66

// c:69-70 — These currently depend on being 0 and 1.
/// `BIN_SETOPT` constant.
pub const BIN_SETOPT: i32 = 0; // c:69
/// `BIN_UNSETOPT` constant.
pub const BIN_UNSETOPT: i32 = 1; // c:70

#[cfg(test)]
mod tests {
    use super::*;

    /// The overload family (c:34-66) MUST be pairwise distinct so
    /// `bin_break` / `bin_typeset` etc. can disambiguate which builtin
    /// is being dispatched via the funcid. The c:69 comment carves out
    /// SETOPT/UNSETOPT specifically because they re-use 0/1 — that
    /// overlap is intentional and load-bearing; testing distinctness
    /// just within the overload family is the real invariant.
    #[test]
    fn overloaded_bin_ids_are_pairwise_distinct() {
        let _g = crate::test_util::global_state_lock();
        let ids = [
            BIN_TYPESET,
            BIN_BG,
            BIN_FG,
            BIN_JOBS,
            BIN_WAIT,
            BIN_DISOWN,
            BIN_BREAK,
            BIN_CONTINUE,
            BIN_EXIT,
            BIN_RETURN,
            BIN_CD,
            BIN_POPD,
            BIN_PUSHD,
            BIN_PRINT,
            BIN_EVAL,
            BIN_SCHED,
            BIN_FC,
            BIN_R,
            BIN_PUSHLINE,
            BIN_LOGOUT,
            BIN_TEST,
            BIN_BRACKET,
            BIN_READONLY,
            BIN_ECHO,
            BIN_DISABLE,
            BIN_ENABLE,
            BIN_PRINTF,
            BIN_COMMAND,
            BIN_UNHASH,
            BIN_UNALIAS,
            BIN_UNFUNCTION,
            BIN_UNSET,
            BIN_EXPORT,
        ];
        let mut sorted = ids.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len());
    }

    /// `Src/hashtable.h:34-66` — every BIN_* slot has a specific
    /// numeric value the dispatch code uses to switch behavior inside
    /// the shared bodies (`bin_break` switches on `func` between
    /// BIN_BREAK=6 / BIN_CONTINUE=7 / BIN_EXIT=8 / BIN_RETURN=9 /
    /// BIN_LOGOUT=19). Pin every value so a reorder fails loudly.
    #[test]
    fn bin_constants_match_c_source_position_for_position() {
        let _g = crate::test_util::global_state_lock();
        let table = [
            ("BIN_TYPESET", BIN_TYPESET, 0),        // c:34
            ("BIN_BG", BIN_BG, 1),                  // c:35
            ("BIN_FG", BIN_FG, 2),                  // c:36
            ("BIN_JOBS", BIN_JOBS, 3),              // c:37
            ("BIN_WAIT", BIN_WAIT, 4),              // c:38
            ("BIN_DISOWN", BIN_DISOWN, 5),          // c:39
            ("BIN_BREAK", BIN_BREAK, 6),            // c:40
            ("BIN_CONTINUE", BIN_CONTINUE, 7),      // c:41
            ("BIN_EXIT", BIN_EXIT, 8),              // c:42
            ("BIN_RETURN", BIN_RETURN, 9),          // c:43
            ("BIN_CD", BIN_CD, 10),                 // c:44
            ("BIN_POPD", BIN_POPD, 11),             // c:45
            ("BIN_PUSHD", BIN_PUSHD, 12),           // c:46
            ("BIN_PRINT", BIN_PRINT, 13),           // c:47
            ("BIN_EVAL", BIN_EVAL, 14),             // c:48
            ("BIN_SCHED", BIN_SCHED, 15),           // c:49
            ("BIN_FC", BIN_FC, 16),                 // c:50
            ("BIN_R", BIN_R, 17),                   // c:51
            ("BIN_PUSHLINE", BIN_PUSHLINE, 18),     // c:52
            ("BIN_LOGOUT", BIN_LOGOUT, 19),         // c:53
            ("BIN_TEST", BIN_TEST, 20),             // c:54
            ("BIN_BRACKET", BIN_BRACKET, 21),       // c:55
            ("BIN_READONLY", BIN_READONLY, 22),     // c:56
            ("BIN_ECHO", BIN_ECHO, 23),             // c:57
            ("BIN_DISABLE", BIN_DISABLE, 24),       // c:58
            ("BIN_ENABLE", BIN_ENABLE, 25),         // c:59
            ("BIN_PRINTF", BIN_PRINTF, 26),         // c:60
            ("BIN_COMMAND", BIN_COMMAND, 27),       // c:61
            ("BIN_UNHASH", BIN_UNHASH, 28),         // c:62
            ("BIN_UNALIAS", BIN_UNALIAS, 29),       // c:63
            ("BIN_UNFUNCTION", BIN_UNFUNCTION, 30), // c:64
            ("BIN_UNSET", BIN_UNSET, 31),           // c:65
            ("BIN_EXPORT", BIN_EXPORT, 32),         // c:66
        ];
        for (name, got, want) in table {
            assert_eq!(
                got, want,
                "c:34-66 — {} must be {} (C source value)",
                name, want
            );
        }
    }

    /// `Src/hashtable.h:69-70` — `BIN_SETOPT=0`, `BIN_UNSETOPT=1`.
    /// The "// These currently depend on being 0 and 1" comment in
    /// the C source is load-bearing: `bin_setopt` switches on
    /// `func==BIN_SETOPT` to choose set vs unset. Pin the values
    /// AND the intentional overlap with BIN_TYPESET / BIN_BG so a
    /// future "let's renumber" refactor surfaces the dependency.
    #[test]
    fn setopt_unsetopt_pin_to_zero_and_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            BIN_SETOPT, 0,
            "c:69 — BIN_SETOPT must be 0 (load-bearing per c:68 comment)"
        );
        assert_eq!(
            BIN_UNSETOPT, 1,
            "c:70 — BIN_UNSETOPT must be 1 (load-bearing per c:68 comment)"
        );
        // The c:68 comment "These currently depend on being 0 and 1"
        // implies the overlap with BIN_TYPESET / BIN_BG is intentional
        // because setopt/unsetopt dispatch is in a different switch.
        assert_eq!(
            BIN_SETOPT, BIN_TYPESET,
            "c:68 — intentional overlap: setopt/unsetopt vs typeset/bg use disjoint dispatch"
        );
        assert_eq!(BIN_UNSETOPT, BIN_BG);
    }

    // ─── zsh-corpus pins for BIN_* dispatch IDs ────────────────────

    /// BIN_BREAK..BIN_RETURN canonical sequence c:40-43.
    #[test]
    fn hashtable_h_corpus_break_continue_exit_return_sequence() {
        assert_eq!(BIN_BREAK, 6);
        assert_eq!(BIN_CONTINUE, 7);
        assert_eq!(BIN_EXIT, 8);
        assert_eq!(BIN_RETURN, 9);
    }

    /// BIN_CD/POPD/PUSHD sequence c:44-46.
    #[test]
    fn hashtable_h_corpus_directory_sequence() {
        assert_eq!(BIN_CD, 10);
        assert_eq!(BIN_POPD, 11);
        assert_eq!(BIN_PUSHD, 12);
    }

    /// BIN_PRINT/EVAL sequence c:47-48.
    #[test]
    fn hashtable_h_corpus_print_eval_sequence() {
        assert_eq!(BIN_PRINT, 13);
        assert_eq!(BIN_EVAL, 14);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/hashtable.h BIN_* constants.
    // ═══════════════════════════════════════════════════════════════════

    /// c:34 — `BIN_TYPESET` is 0 (canonical first slot).
    #[test]
    fn BIN_TYPESET_is_zero() {
        assert_eq!(BIN_TYPESET, 0);
    }

    /// c:35-39 — job-control builtins (BG/FG/JOBS/WAIT/DISOWN).
    #[test]
    fn job_control_builtin_codes_canonical() {
        assert_eq!(BIN_BG, 1);
        assert_eq!(BIN_FG, 2);
        assert_eq!(BIN_JOBS, 3);
        assert_eq!(BIN_WAIT, 4);
        assert_eq!(BIN_DISOWN, 5);
    }

    /// c:49-53 — sched/fc/r/pushline/logout sequence (15..19).
    #[test]
    fn hist_builtin_codes_canonical() {
        assert_eq!(BIN_SCHED, 15);
        assert_eq!(BIN_FC, 16);
        assert_eq!(BIN_R, 17);
        assert_eq!(BIN_PUSHLINE, 18);
        assert_eq!(BIN_LOGOUT, 19);
    }

    /// c:54-58 — test/[/readonly/echo/disable (20..24).
    #[test]
    fn test_readonly_echo_builtin_codes_canonical() {
        assert_eq!(BIN_TEST, 20);
        assert_eq!(BIN_BRACKET, 21);
        assert_eq!(BIN_READONLY, 22);
        assert_eq!(BIN_ECHO, 23);
        assert_eq!(BIN_DISABLE, 24);
    }

    /// All BIN_* constants are pairwise distinct (no aliasing).
    #[test]
    fn all_bin_codes_pairwise_distinct() {
        let codes = [
            BIN_TYPESET, BIN_BG, BIN_FG, BIN_JOBS, BIN_WAIT, BIN_DISOWN,
            BIN_BREAK, BIN_CONTINUE, BIN_EXIT, BIN_RETURN,
            BIN_CD, BIN_POPD, BIN_PUSHD,
            BIN_PRINT, BIN_EVAL, BIN_SCHED, BIN_FC, BIN_R,
            BIN_PUSHLINE, BIN_LOGOUT, BIN_TEST, BIN_BRACKET,
            BIN_READONLY, BIN_ECHO, BIN_DISABLE,
        ];
        let unique: std::collections::HashSet<_> = codes.iter().copied().collect();
        assert_eq!(unique.len(), codes.len(), "BIN_* must be pairwise distinct");
    }

    /// All BIN_* constants are non-negative (used as array indices).
    #[test]
    fn all_bin_codes_non_negative() {
        for c in [
            BIN_TYPESET, BIN_BG, BIN_FG, BIN_JOBS, BIN_WAIT, BIN_DISOWN,
            BIN_BREAK, BIN_CONTINUE, BIN_EXIT, BIN_RETURN,
            BIN_CD, BIN_POPD, BIN_PUSHD,
            BIN_PRINT, BIN_EVAL,
        ] {
            assert!(c >= 0, "BIN_* code {} must be ≥ 0", c);
        }
    }

    /// c:34-58 — BIN_* form contiguous 0..25 (no holes).
    #[test]
    fn bin_codes_form_contiguous_range() {
        let mut codes: Vec<i32> = vec![
            BIN_TYPESET, BIN_BG, BIN_FG, BIN_JOBS, BIN_WAIT, BIN_DISOWN,
            BIN_BREAK, BIN_CONTINUE, BIN_EXIT, BIN_RETURN,
            BIN_CD, BIN_POPD, BIN_PUSHD,
            BIN_PRINT, BIN_EVAL, BIN_SCHED, BIN_FC, BIN_R,
            BIN_PUSHLINE, BIN_LOGOUT, BIN_TEST, BIN_BRACKET,
            BIN_READONLY, BIN_ECHO, BIN_DISABLE,
        ];
        codes.sort();
        for (i, c) in codes.iter().enumerate() {
            assert_eq!(*c, i as i32, "BIN_* must form contiguous 0..N range");
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/hashtable.h c:34-70 BIN_* slots
    // ═══════════════════════════════════════════════════════════════════

    /// c:58-66 — BIN_DISABLE..BIN_EXPORT form contiguous 24..32 (extends
    /// the prior 0..24 contiguity check to cover the full overload table).
    #[test]
    fn bin_codes_24_through_32_contiguous() {
        let extras: Vec<i32> = vec![
            BIN_DISABLE,    // 24 c:58
            BIN_ENABLE,     // 25 c:59
            BIN_PRINTF,     // 26 c:60
            BIN_COMMAND,    // 27 c:61
            BIN_UNHASH,     // 28 c:62
            BIN_UNALIAS,    // 29 c:63
            BIN_UNFUNCTION, // 30 c:64
            BIN_UNSET,      // 31 c:65
            BIN_EXPORT,     // 32 c:66
        ];
        for (i, &c) in extras.iter().enumerate() {
            assert_eq!(c, 24 + i as i32, "BIN_* tail must be contiguous 24..32");
        }
    }

    /// c:62-66 — `un*` family BIN_* (BIN_UNHASH..BIN_UNSET) are
    /// sequential 28..31.
    #[test]
    fn bin_un_family_sequential() {
        assert_eq!(BIN_UNHASH, 28, "c:62");
        assert_eq!(BIN_UNALIAS, 29, "c:63");
        assert_eq!(BIN_UNFUNCTION, 30, "c:64");
        assert_eq!(BIN_UNSET, 31, "c:65");
    }

    /// c:60-61 — output/exec family BIN_PRINTF, BIN_COMMAND sequential 26,27.
    #[test]
    fn bin_output_exec_family_sequential() {
        assert_eq!(BIN_PRINTF, 26, "c:60");
        assert_eq!(BIN_COMMAND, 27, "c:61");
    }

    /// c:58-59 — enable/disable BIN_* are sequential 24,25.
    #[test]
    fn bin_enable_disable_sequential() {
        assert_eq!(BIN_DISABLE, 24, "c:58");
        assert_eq!(BIN_ENABLE, 25, "c:59");
    }

    /// c:66 — BIN_EXPORT is the highest non-overlapped BIN_* code (= 32).
    /// Pin so adding new entries can't silently push past this without
    /// updating the overload-family assumption.
    #[test]
    fn bin_export_is_max_non_overlapping() {
        let all = [
            BIN_TYPESET, BIN_BG, BIN_FG, BIN_JOBS, BIN_WAIT, BIN_DISOWN,
            BIN_BREAK, BIN_CONTINUE, BIN_EXIT, BIN_RETURN,
            BIN_CD, BIN_POPD, BIN_PUSHD, BIN_PRINT, BIN_EVAL,
            BIN_SCHED, BIN_FC, BIN_R, BIN_PUSHLINE, BIN_LOGOUT,
            BIN_TEST, BIN_BRACKET, BIN_READONLY, BIN_ECHO, BIN_DISABLE,
            BIN_ENABLE, BIN_PRINTF, BIN_COMMAND, BIN_UNHASH, BIN_UNALIAS,
            BIN_UNFUNCTION, BIN_UNSET, BIN_EXPORT,
        ];
        let max = *all.iter().max().unwrap();
        assert_eq!(max, BIN_EXPORT, "BIN_EXPORT must be max non-overlapping slot");
        assert_eq!(BIN_EXPORT, 32, "c:66 = 32");
    }

    /// c:69-70 — BIN_SETOPT/BIN_UNSETOPT INTENTIONALLY overlap with
    /// BIN_TYPESET/BIN_BG slots 0/1; the comment at c:67 calls this out
    /// explicitly. Pin the overlap because removing it would be a
    /// behavioral break the C source has carried for decades.
    #[test]
    fn bin_setopt_unsetopt_overlap_typeset_bg_is_intentional() {
        assert_eq!(BIN_SETOPT, BIN_TYPESET, "c:69 SETOPT overlaps TYPESET");
        assert_eq!(BIN_UNSETOPT, BIN_BG, "c:70 UNSETOPT overlaps BG");
    }

    /// c:34-66 — full BIN_* enumeration has exactly 33 entries
    /// (TYPESET..EXPORT inclusive, plus SETOPT/UNSETOPT which overlap).
    #[test]
    fn bin_overload_family_has_33_distinct_slots() {
        let ids = [
            BIN_TYPESET, BIN_BG, BIN_FG, BIN_JOBS, BIN_WAIT, BIN_DISOWN,
            BIN_BREAK, BIN_CONTINUE, BIN_EXIT, BIN_RETURN,
            BIN_CD, BIN_POPD, BIN_PUSHD, BIN_PRINT, BIN_EVAL,
            BIN_SCHED, BIN_FC, BIN_R, BIN_PUSHLINE, BIN_LOGOUT,
            BIN_TEST, BIN_BRACKET, BIN_READONLY, BIN_ECHO, BIN_DISABLE,
            BIN_ENABLE, BIN_PRINTF, BIN_COMMAND, BIN_UNHASH, BIN_UNALIAS,
            BIN_UNFUNCTION, BIN_UNSET, BIN_EXPORT,
        ];
        assert_eq!(ids.len(), 33, "33 explicitly numbered BIN_* overload slots");
        let unique: std::collections::HashSet<_> = ids.iter().copied().collect();
        assert_eq!(unique.len(), ids.len(), "all 33 must be pairwise distinct");
    }

    /// c:34-66 — every BIN_* value fits in an i32 (used as funcid).
    /// All values must be < 256 to fit a hypothetical u8 funcid table.
    #[test]
    fn all_bin_codes_fit_in_u8_range() {
        let all = [
            BIN_TYPESET, BIN_BG, BIN_FG, BIN_JOBS, BIN_WAIT, BIN_DISOWN,
            BIN_BREAK, BIN_CONTINUE, BIN_EXIT, BIN_RETURN,
            BIN_CD, BIN_POPD, BIN_PUSHD, BIN_PRINT, BIN_EVAL,
            BIN_SCHED, BIN_FC, BIN_R, BIN_PUSHLINE, BIN_LOGOUT,
            BIN_TEST, BIN_BRACKET, BIN_READONLY, BIN_ECHO, BIN_DISABLE,
            BIN_ENABLE, BIN_PRINTF, BIN_COMMAND, BIN_UNHASH, BIN_UNALIAS,
            BIN_UNFUNCTION, BIN_UNSET, BIN_EXPORT, BIN_SETOPT, BIN_UNSETOPT,
        ];
        for c in all {
            assert!(
                (0..256).contains(&c),
                "BIN_* code {} must fit in 0..256 (u8 funcid headroom)",
                c
            );
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/hashtable.h
    // c:34-66 BIN_* per-family canonical groupings + sub-family checks
    // ═══════════════════════════════════════════════════════════════════

    /// c:40-43 — break/continue/exit/return canonical 6/7/8/9.
    #[test]
    fn bin_break_continue_exit_return_sequential_six_through_nine() {
        assert_eq!(BIN_BREAK, 6, "c:40");
        assert_eq!(BIN_CONTINUE, 7, "c:41");
        assert_eq!(BIN_EXIT, 8, "c:42");
        assert_eq!(BIN_RETURN, 9, "c:43");
    }

    /// c:44-46 — cd/popd/pushd canonical 10/11/12.
    #[test]
    fn bin_cd_popd_pushd_sequential_ten_through_twelve() {
        assert_eq!(BIN_CD, 10, "c:44");
        assert_eq!(BIN_POPD, 11, "c:45");
        assert_eq!(BIN_PUSHD, 12, "c:46");
    }

    /// c:47-48 — print/eval canonical 13/14.
    #[test]
    fn bin_print_eval_canonical() {
        assert_eq!(BIN_PRINT, 13, "c:47");
        assert_eq!(BIN_EVAL, 14, "c:48");
    }

    /// c:49-52 — sched/fc/r/pushline canonical 15/16/17/18.
    #[test]
    fn bin_sched_fc_r_pushline_canonical() {
        assert_eq!(BIN_SCHED, 15, "c:49");
        assert_eq!(BIN_FC, 16, "c:50");
        assert_eq!(BIN_R, 17, "c:51");
        assert_eq!(BIN_PUSHLINE, 18, "c:52");
    }

    /// c:53-55 — logout/test/bracket canonical 19/20/21.
    #[test]
    fn bin_logout_test_bracket_canonical() {
        assert_eq!(BIN_LOGOUT, 19, "c:53");
        assert_eq!(BIN_TEST, 20, "c:54");
        assert_eq!(BIN_BRACKET, 21, "c:55");
    }

    /// c:34-66 — all BIN_* fit in i8 signed range (positive).
    #[test]
    fn all_bin_codes_fit_in_i8_positive_range() {
        for c in [BIN_TYPESET, BIN_BG, BIN_FG, BIN_JOBS, BIN_WAIT,
                  BIN_DISOWN, BIN_BREAK, BIN_CONTINUE, BIN_EXIT,
                  BIN_RETURN, BIN_CD, BIN_POPD, BIN_PUSHD, BIN_PRINT,
                  BIN_EVAL, BIN_SCHED, BIN_FC, BIN_R, BIN_PUSHLINE,
                  BIN_LOGOUT, BIN_TEST, BIN_BRACKET, BIN_READONLY,
                  BIN_ECHO, BIN_DISABLE, BIN_ENABLE, BIN_PRINTF,
                  BIN_COMMAND, BIN_UNHASH, BIN_UNALIAS, BIN_UNFUNCTION,
                  BIN_UNSET, BIN_EXPORT] {
            assert!((0..=i8::MAX as i32).contains(&c),
                "BIN_* code {} must fit in i8 positive range", c);
        }
    }

    /// c:34-66 — all BIN_* are i32 (compile-time type pin).
    #[test]
    fn all_bin_codes_are_i32_type() {
        let _: i32 = BIN_TYPESET;
        let _: i32 = BIN_SETOPT;
        let _: i32 = BIN_EXPORT;
    }

    /// c:34-66 — BIN_TYPESET is canonical first slot.
    #[test]
    fn bin_typeset_is_canonical_first_slot() {
        assert_eq!(BIN_TYPESET, 0, "TYPESET claims slot 0");
    }

    /// c:34-39 — job control family (TYPESET..DISOWN) covers slots 0-5.
    #[test]
    fn bin_job_control_family_slots_zero_through_five() {
        assert_eq!(BIN_TYPESET, 0);
        assert_eq!(BIN_BG, 1);
        assert_eq!(BIN_FG, 2);
        assert_eq!(BIN_JOBS, 3);
        assert_eq!(BIN_WAIT, 4);
        assert_eq!(BIN_DISOWN, 5);
    }

    /// c:69-70 — BIN_SETOPT/BIN_UNSETOPT are 0/1 (intentional overlap).
    #[test]
    fn bin_setopt_unsetopt_canonical_zero_one() {
        assert_eq!(BIN_SETOPT, 0, "c:69");
        assert_eq!(BIN_UNSETOPT, 1, "c:70");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/hashtable.h BIN_* constants.
    // ═══════════════════════════════════════════════════════════════════

    /// c:56-60 — readonly/echo/disable/enable/printf canonical 22..26.
    #[test]
    fn bin_readonly_through_printf_canonical() {
        assert_eq!(BIN_READONLY, 22, "c:56");
        assert_eq!(BIN_ECHO, 23, "c:57");
        assert_eq!(BIN_DISABLE, 24, "c:58");
        assert_eq!(BIN_ENABLE, 25, "c:59");
        assert_eq!(BIN_PRINTF, 26, "c:60");
    }

    /// c:61-66 — command..export canonical 27..32.
    #[test]
    fn bin_command_through_export_canonical() {
        assert_eq!(BIN_COMMAND, 27, "c:61");
        assert_eq!(BIN_UNHASH, 28, "c:62");
        assert_eq!(BIN_UNALIAS, 29, "c:63");
        assert_eq!(BIN_UNFUNCTION, 30, "c:64");
        assert_eq!(BIN_UNSET, 31, "c:65");
        assert_eq!(BIN_EXPORT, 32, "c:66");
    }

    /// c:34-66 — overload family covers exactly slots 0..=32 (33 entries).
    /// No gaps, no duplicates within the family.
    #[test]
    fn bin_overload_family_is_dense_0_through_32() {
        let mut codes: Vec<i32> = vec![
            BIN_TYPESET, BIN_BG, BIN_FG, BIN_JOBS, BIN_WAIT,
            BIN_DISOWN, BIN_BREAK, BIN_CONTINUE, BIN_EXIT,
            BIN_RETURN, BIN_CD, BIN_POPD, BIN_PUSHD, BIN_PRINT,
            BIN_EVAL, BIN_SCHED, BIN_FC, BIN_R, BIN_PUSHLINE,
            BIN_LOGOUT, BIN_TEST, BIN_BRACKET, BIN_READONLY,
            BIN_ECHO, BIN_DISABLE, BIN_ENABLE, BIN_PRINTF,
            BIN_COMMAND, BIN_UNHASH, BIN_UNALIAS, BIN_UNFUNCTION,
            BIN_UNSET, BIN_EXPORT,
        ];
        codes.sort();
        let expected: Vec<i32> = (0..=32).collect();
        assert_eq!(codes, expected,
            "overload family must cover exactly 0..=32 with no gaps/dups");
    }

    /// c:69-70 — BIN_SETOPT/BIN_UNSETOPT family is its OWN dispatch
    /// space; the overlap with TYPESET/BG (= 0/1) is intentional per
    /// the c:69 comment "depend on being 0 and 1".
    #[test]
    fn bin_setopt_family_has_distinct_pair() {
        assert_ne!(BIN_SETOPT, BIN_UNSETOPT,
            "SETOPT and UNSETOPT must differ to drive bin_setopt's branch");
    }

    /// c:34 / c:40 — BIN_TYPESET (0) and BIN_BREAK (6) are in different
    /// halves of the overload table — overload[0..6] = job/typeset
    /// family, overload[6..10] = control-flow family.
    #[test]
    fn bin_break_starts_control_flow_family() {
        assert_eq!(BIN_BREAK, 6, "c:40 — BREAK leads control-flow family");
        assert_eq!(BIN_CONTINUE, 7, "c:41");
        assert_eq!(BIN_EXIT, 8, "c:42");
        assert_eq!(BIN_RETURN, 9, "c:43");
    }

    /// c:44-46 — cd/popd/pushd canonical 10/11/12 (dir-stack family).
    #[test]
    fn bin_dir_stack_family_canonical() {
        assert_eq!(BIN_CD, 10, "c:44");
        assert_eq!(BIN_POPD, 11, "c:45");
        assert_eq!(BIN_PUSHD, 12, "c:46");
    }

    /// c:62-64 — unhash/unalias/unfunction family canonical 28/29/30.
    #[test]
    fn bin_un_family_canonical() {
        assert_eq!(BIN_UNHASH, 28, "c:62");
        assert_eq!(BIN_UNALIAS, 29, "c:63");
        assert_eq!(BIN_UNFUNCTION, 30, "c:64");
    }

    /// c:34-66 — BIN_EXPORT is the canonical max (slot 32 — sentinel
    /// for "highest registered overload code").
    #[test]
    fn bin_export_is_canonical_max_slot() {
        for c in [BIN_TYPESET, BIN_BG, BIN_FG, BIN_JOBS, BIN_WAIT,
                  BIN_DISOWN, BIN_BREAK, BIN_CONTINUE, BIN_EXIT,
                  BIN_RETURN, BIN_CD, BIN_POPD, BIN_PUSHD, BIN_PRINT,
                  BIN_EVAL, BIN_SCHED, BIN_FC, BIN_R, BIN_PUSHLINE,
                  BIN_LOGOUT, BIN_TEST, BIN_BRACKET, BIN_READONLY,
                  BIN_ECHO, BIN_DISABLE, BIN_ENABLE, BIN_PRINTF,
                  BIN_COMMAND, BIN_UNHASH, BIN_UNALIAS, BIN_UNFUNCTION,
                  BIN_UNSET] {
            assert!(c <= BIN_EXPORT,
                "BIN_EXPORT (32) must be the max overload code; {} > 32", c);
        }
    }

    /// c:34-66 — overload codes are all non-negative.
    #[test]
    fn bin_overload_codes_all_non_negative() {
        for c in [BIN_TYPESET, BIN_BG, BIN_FG, BIN_JOBS, BIN_WAIT,
                  BIN_DISOWN, BIN_BREAK, BIN_CONTINUE, BIN_EXIT,
                  BIN_RETURN, BIN_CD, BIN_POPD, BIN_PUSHD, BIN_PRINT,
                  BIN_EVAL, BIN_SCHED, BIN_FC, BIN_R, BIN_PUSHLINE,
                  BIN_LOGOUT, BIN_TEST, BIN_BRACKET, BIN_READONLY,
                  BIN_ECHO, BIN_DISABLE, BIN_ENABLE, BIN_PRINTF,
                  BIN_COMMAND, BIN_UNHASH, BIN_UNALIAS, BIN_UNFUNCTION,
                  BIN_UNSET, BIN_EXPORT] {
            assert!(c >= 0, "BIN_* must be non-negative; got {}", c);
        }
    }

    /// c:69-70 — both setopt-family codes also non-negative.
    #[test]
    fn bin_setopt_family_codes_non_negative() {
        assert!(BIN_SETOPT >= 0);
        assert!(BIN_UNSETOPT >= 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional pins for Src/hashtable.h
    // c:34-66 BIN_* main overload codes / c:69-70 BIN_SETOPT family
    // ═══════════════════════════════════════════════════════════════════

    /// c:34-66 — verbatim positional pins: every BIN_* main slot matches
    /// its source-line offset.
    #[test]
    fn bin_main_slot_values_exact() {
        assert_eq!(BIN_TYPESET, 0);
        assert_eq!(BIN_BG, 1);
        assert_eq!(BIN_FG, 2);
        assert_eq!(BIN_JOBS, 3);
        assert_eq!(BIN_WAIT, 4);
        assert_eq!(BIN_DISOWN, 5);
        assert_eq!(BIN_BREAK, 6);
        assert_eq!(BIN_CONTINUE, 7);
        assert_eq!(BIN_EXIT, 8);
        assert_eq!(BIN_RETURN, 9);
        assert_eq!(BIN_CD, 10);
        assert_eq!(BIN_POPD, 11);
        assert_eq!(BIN_PUSHD, 12);
        assert_eq!(BIN_PRINT, 13);
        assert_eq!(BIN_EVAL, 14);
        assert_eq!(BIN_SCHED, 15);
        assert_eq!(BIN_FC, 16);
        assert_eq!(BIN_R, 17);
        assert_eq!(BIN_PUSHLINE, 18);
        assert_eq!(BIN_LOGOUT, 19);
    }

    /// c:54-66 — verbatim positional pins for the rest of BIN_* main slots.
    #[test]
    fn bin_main_slot_values_exact_part_two() {
        assert_eq!(BIN_TEST, 20);
        assert_eq!(BIN_BRACKET, 21);
        assert_eq!(BIN_READONLY, 22);
        assert_eq!(BIN_ECHO, 23);
        assert_eq!(BIN_DISABLE, 24);
        assert_eq!(BIN_ENABLE, 25);
        assert_eq!(BIN_PRINTF, 26);
        assert_eq!(BIN_COMMAND, 27);
        assert_eq!(BIN_UNHASH, 28);
        assert_eq!(BIN_UNALIAS, 29);
        assert_eq!(BIN_UNFUNCTION, 30);
        assert_eq!(BIN_UNSET, 31);
        assert_eq!(BIN_EXPORT, 32);
    }

    /// c:34-66 — 33 main-slot codes form a dense 0..=32 sequence.
    #[test]
    fn bin_main_slots_form_dense_0_to_32() {
        let mut codes = [
            BIN_TYPESET, BIN_BG, BIN_FG, BIN_JOBS, BIN_WAIT, BIN_DISOWN,
            BIN_BREAK, BIN_CONTINUE, BIN_EXIT, BIN_RETURN, BIN_CD,
            BIN_POPD, BIN_PUSHD, BIN_PRINT, BIN_EVAL, BIN_SCHED, BIN_FC,
            BIN_R, BIN_PUSHLINE, BIN_LOGOUT, BIN_TEST, BIN_BRACKET,
            BIN_READONLY, BIN_ECHO, BIN_DISABLE, BIN_ENABLE, BIN_PRINTF,
            BIN_COMMAND, BIN_UNHASH, BIN_UNALIAS, BIN_UNFUNCTION,
            BIN_UNSET, BIN_EXPORT,
        ];
        codes.sort();
        let expected: Vec<i32> = (0..=32).collect();
        assert_eq!(codes.to_vec(), expected,
            "BIN_* main slots must be dense 0..=32 (no gaps)");
    }

    /// c:69-70 — setopt-family pair values pinned verbatim.
    #[test]
    fn bin_setopt_family_exact_values() {
        assert_eq!(BIN_SETOPT, 0);
        assert_eq!(BIN_UNSETOPT, 1);
    }

    /// c:69-70 — setopt-family codes are distinct from each other.
    #[test]
    fn bin_setopt_family_pair_distinct() {
        assert_ne!(BIN_SETOPT, BIN_UNSETOPT,
            "setopt vs unsetopt must be different overload codes");
    }

    /// c:34-66 — main codes fit comfortably in i8 (room for ≥2x growth).
    #[test]
    fn bin_main_codes_fit_in_i8_room_for_growth() {
        for c in [BIN_TYPESET, BIN_EXPORT] {
            assert!(c < i8::MAX as i32,
                "BIN_* main {} must fit in i8 (≥2x growth headroom)", c);
        }
    }

    /// c:34-66 — total count of main codes is 33 (offsets 0..=32, dense).
    #[test]
    fn bin_main_slots_count_is_33() {
        let codes = [
            BIN_TYPESET, BIN_BG, BIN_FG, BIN_JOBS, BIN_WAIT, BIN_DISOWN,
            BIN_BREAK, BIN_CONTINUE, BIN_EXIT, BIN_RETURN, BIN_CD,
            BIN_POPD, BIN_PUSHD, BIN_PRINT, BIN_EVAL, BIN_SCHED, BIN_FC,
            BIN_R, BIN_PUSHLINE, BIN_LOGOUT, BIN_TEST, BIN_BRACKET,
            BIN_READONLY, BIN_ECHO, BIN_DISABLE, BIN_ENABLE, BIN_PRINTF,
            BIN_COMMAND, BIN_UNHASH, BIN_UNALIAS, BIN_UNFUNCTION,
            BIN_UNSET, BIN_EXPORT,
        ];
        assert_eq!(codes.len(), 33,
            "33 BIN_* main slots must be declared");
    }

    /// c:34-66 — BIN_TYPESET is the canonical zero (first slot).
    #[test]
    fn bin_typeset_is_zero_sentinel() {
        assert_eq!(BIN_TYPESET, 0, "BIN_TYPESET must be first slot (= 0)");
    }

    /// c:43 / c:42 — BIN_EXIT (8) and BIN_RETURN (9) are sequential
    /// (zsh's flow-control pair).
    #[test]
    fn bin_exit_return_are_sequential() {
        assert_eq!(BIN_RETURN - BIN_EXIT, 1,
            "BIN_RETURN must immediately follow BIN_EXIT");
    }

    /// c:40-41 — BIN_BREAK (6) and BIN_CONTINUE (7) are sequential
    /// (loop-control pair).
    #[test]
    fn bin_break_continue_are_sequential() {
        assert_eq!(BIN_CONTINUE - BIN_BREAK, 1,
            "BIN_CONTINUE must immediately follow BIN_BREAK");
    }

    /// c:62-66 — un* family (UNHASH, UNALIAS, UNFUNCTION, UNSET) are
    /// 4 consecutive slots (28..=31).
    #[test]
    fn bin_un_family_is_consecutive() {
        let un = [BIN_UNHASH, BIN_UNALIAS, BIN_UNFUNCTION, BIN_UNSET];
        for w in un.windows(2) {
            assert_eq!(w[1] - w[0], 1,
                "un* family must be consecutive: {} → {}", w[0], w[1]);
        }
    }

    /// c:54-55 — BIN_TEST (20) and BIN_BRACKET (21) are sequential
    /// (the `test` vs `[` pair).
    #[test]
    fn bin_test_bracket_are_sequential() {
        assert_eq!(BIN_BRACKET - BIN_TEST, 1,
            "BIN_BRACKET must immediately follow BIN_TEST");
    }
}
