//! Per-shell output formats for builtins whose listing differs between
//! the shells zshrs stands in for.
//!
//! **zshrs-only — no zsh C counterpart.** `Src/builtin.c` prints one
//! format, zsh's. A drop-in has to print what ITS shell prints, because
//! these strings are parsed by scripts (`export -p` is specified to be
//! re-inputtable) and read by people (`times`, `kill -l`).
//!
//! Every format here was captured from the reference binary in an empty
//! environment, and is re-checked against it by the parity harness.

use crate::extensions::emulation_startup::{personality, Personality};

/// How a shell renders one `times` field.
///
/// Captured with `<shell> -c times`:
///
/// | shell | output |
/// |-------|--------|
/// | zsh 5.9 | `0m0.00s 0m0.00s` |
/// | bash 5.3 | `0m0.008s 0m0.011s` |
/// | ksh93u+m | `0m00.000s 0m00.000s` |
/// | mksh | `0m00.00s 0m00.00s` |
/// | dash | `0m0.000000s 0m0.000000s` |
pub fn times_field(ticks: i64, clktck: i64) -> String {
    let clktck = if clktck <= 0 { 100 } else { clktck };
    let mins = ticks / (60 * clktck);
    let p = personality();
    // zsh computes the seconds field as `X/clktck % clktck`
    // (c:Src/builtin.c:7315-7318) — modulo the CLOCK TICK, not 60 — so a
    // shell that has burned 125s of CPU prints `2m25.00s`. Every other
    // shell here takes `% 60` and prints `2m5s`. The zsh arm keeps the
    // upstream arithmetic verbatim; the drop-ins take their own.
    let secs = if p == Personality::Zsh {
        (ticks / clktck) % clktck
    } else {
        (ticks / clktck) % 60
    };
    match p {
        // bash: three decimals (milliseconds).
        Personality::Bash => {
            let ms = (ticks * 1000 / clktck) % 1000;
            format!("{mins}m{secs}.{ms:03}s")
        }
        // dash: six decimals (microseconds).
        Personality::Dash => {
            let us = (ticks * 1_000_000 / clktck) % 1_000_000;
            format!("{mins}m{secs}.{us:06}s")
        }
        // ksh93u+m: milliseconds AND a zero-padded seconds field. (The
        // legacy 93u+ 2012 line printed a labelled `user\t0m0.00s`
        // instead; zshrs follows the maintained fork, as it does for the
        // default alias set.)
        Personality::Ksh93 => {
            let ms = (ticks * 1000 / clktck) % 1000;
            format!("{mins}m{secs:02}.{ms:03}s")
        }
        // mksh: centiseconds, but the SECONDS field is zero-padded to two.
        Personality::Mksh | Personality::Pdksh => {
            let cs = (ticks * 100 / clktck) % 100;
            format!("{mins}m{secs:02}.{cs:02}s")
        }
        // zsh, ksh93 and the POSIX shells: centiseconds, unpadded seconds.
        // c:Src/builtin.c:7315-7318.
        _ => {
            let cs = (ticks * 100 / clktck) % 100;
            format!("{mins}m{secs}.{cs:02}s")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::emulation_startup::set_personality;

    /// One lock for the whole module: these tests move the process-global
    /// personality, so they must not run concurrently with each other.
    static GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// 100 ticks at 100Hz is exactly one second, which pins the seconds
    /// field, the padding and the fraction width in one shot.
    #[test]
    fn times_field_matches_each_shell() {
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let saved = personality();
        for (p, want) in [
            (Personality::Zsh, "0m1.00s"),
            (Personality::Bash, "0m1.000s"),
            (Personality::Dash, "0m1.000000s"),
            (Personality::Mksh, "0m01.00s"),
            (Personality::Ksh93, "0m01.000s"),
        ] {
            set_personality(p);
            assert_eq!(times_field(100, 100), want, "{p:?}");
        }
        set_personality(saved);
    }

    /// A sub-second value exercises the fraction, and a multi-minute one
    /// the minutes field — the two places an off-by-one in the integer
    /// arithmetic would hide.
    #[test]
    fn times_field_fraction_and_minutes() {
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let saved = personality();
        set_personality(Personality::Bash);
        // 1 tick at 100Hz = 10ms.
        assert_eq!(times_field(1, 100), "0m0.010s");
        // 125 seconds = 2m5s in bash …
        assert_eq!(times_field(12500, 100), "2m5.000s");
        set_personality(Personality::Dash);
        assert_eq!(times_field(1, 100), "0m0.010000s");
        // … but zsh's own arithmetic mods the seconds by the clock tick,
        // so the same 125s prints as 2m25s. That is upstream's behaviour
        // and the `--zsh` drop-in has to keep it.
        set_personality(Personality::Zsh);
        assert_eq!(times_field(12500, 100), "2m25.00s");
        set_personality(saved);
    }
}
