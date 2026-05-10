//! ZLE parameters
//!
//! Direct port from zsh/Src/Zle/zle_params.c
//!
//! Special parameters that expose ZLE state to shell scripts

use super::zle_main::Zle;

/// ZLE parameter names
pub mod names {
    pub const BUFFER: &str = "BUFFER";
    pub const CURSOR: &str = "CURSOR";
    pub const LBUFFER: &str = "LBUFFER";
    pub const RBUFFER: &str = "RBUFFER";
    pub const PREBUFFER: &str = "PREBUFFER";
    pub const WIDGET: &str = "WIDGET";
    pub const LASTWIDGET: &str = "LASTWIDGET";
    pub const KEYMAP: &str = "KEYMAP";
    pub const KEYS: &str = "KEYS";
    pub const NUMERIC: &str = "NUMERIC";
    pub const HISTNO: &str = "HISTNO";
    pub const BUFFERLINES: &str = "BUFFERLINES";
    pub const PENDING: &str = "PENDING";
    pub const CUTBUFFER: &str = "CUTBUFFER";
    pub const KILLRING: &str = "killring";
    pub const MARK: &str = "MARK";
    pub const REGION_ACTIVE: &str = "REGION_ACTIVE";
    pub const ZLE_STATE: &str = "ZLE_STATE";
}

// Each accessor below corresponds to one of the special parameters
// zsh exposes via Src/Zle/zle_params.c. The C source registers them
// through the `zleparams[]` table at zle_params.c:38; widget bodies
// (and shell scripts running inside ZLE) read or assign to them
// through the parameter system.
// ro means parameters are readonly, used from completion              // c:190
impl Zle {
    /// `$BUFFER` accessor — full edited line as a String.
    /// Port of `get_buffer()` from Src/Zle/zle_params.c (the
    /// `BUFFER` getfn entry in `zleparams[]`).
    pub fn get_buffer(&self) -> String {
        self.zleline.iter().collect()
    }

    /// `$BUFFER=s` setter — replace the full edited line.
    /// Port of `set_buffer()` from Src/Zle/zle_params.c (the
    /// `BUFFER` setfn entry); zsh clamps the cursor to the new
    /// length, mirrored here.
    pub fn set_buffer(&mut self, s: &str) {
        self.zleline = s.chars().collect();
        self.zlell = self.zleline.len();
        self.zlecs = self.zlecs.min(self.zlell);
        self.resetneeded = true;
    }

    /// `$CURSOR` accessor — current cursor position (0-indexed).
    /// Port of `get_cursor()` from Src/Zle/zle_params.c.
    pub fn get_cursor(&self) -> usize {
        self.zlecs
    }

    /// `$CURSOR=pos` setter — clamped to buffer length.
    /// Port of `set_cursor()` from Src/Zle/zle_params.c.
    pub fn set_cursor(&mut self, pos: usize) {
        self.zlecs = pos.min(self.zlell);
        self.resetneeded = true;
    }

    /// `$LBUFFER` accessor — text before the cursor.
    /// Port of `get_lbuffer()` from Src/Zle/zle_params.c.
    pub fn get_lbuffer(&self) -> String {
        self.zleline[..self.zlecs].iter().collect()
    }

    /// `$LBUFFER=s` setter — replace text before the cursor; cursor
    /// lands at the new lbuffer's end.
    /// Port of `set_lbuffer()` from Src/Zle/zle_params.c.
    pub fn set_lbuffer(&mut self, s: &str) {
        let rbuf: String = self.zleline[self.zlecs..].iter().collect();
        self.zleline = s.chars().chain(rbuf.chars()).collect();
        self.zlell = self.zleline.len();
        self.zlecs = s.chars().count();
        self.resetneeded = true;
    }

    /// `$RBUFFER` accessor — text after the cursor.
    /// Port of `get_rbuffer()` from Src/Zle/zle_params.c.
    pub fn get_rbuffer(&self) -> String {
        self.zleline[self.zlecs..].iter().collect()
    }

    /// `$RBUFFER=s` setter — replace text after the cursor.
    /// Port of `set_rbuffer()` from Src/Zle/zle_params.c.
    pub fn set_rbuffer(&mut self, s: &str) {
        let lbuf: String = self.zleline[..self.zlecs].iter().collect();
        self.zleline = lbuf.chars().chain(s.chars()).collect();
        self.zlell = self.zleline.len();
        self.resetneeded = true;
    }

    /// `$CUTBUFFER` accessor — most-recent kill-ring entry.
    /// Port of `get_cutbuffer()` from Src/Zle/zle_params.c which
    /// reads `cutbuf` (the unnamed kill register).
    pub fn get_cutbuffer(&self) -> String {
        self.killring
            .front()
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }

    /// `$CUTBUFFER=s` setter — overwrite the front of the kill ring.
    /// Port of `set_cutbuffer()` from Src/Zle/zle_params.c.
    pub fn set_cutbuffer(&mut self, s: &str) {
        let chars: Vec<char> = s.chars().collect();
        if self.killring.is_empty() {
            self.killring.push_front(chars);
        } else {
            self.killring[0] = chars;
        }
    }

    /// `$MARK` accessor — current mark position.
    /// Port of `get_mark()` from Src/Zle/zle_params.c.
    pub fn get_mark(&self) -> usize {
        self.mark
    }

    /// `$MARK=pos` setter — clamp to buffer length.
    /// Port of `set_mark()` from Src/Zle/zle_params.c.
    pub fn set_mark(&mut self, pos: usize) {
        self.mark = pos.min(self.zlell);
    }

    /// `$BUFFERLINES` accessor — number of newline-separated lines.
    /// Port of `get_bufferlines()` from Src/Zle/zle_params.c.
    pub fn get_bufferlines(&self) -> usize {
        self.zleline.iter().filter(|&&c| c == '\n').count() + 1
    }

    /// `$PENDING` accessor — bytes waiting in the input queue.
    /// Port of `get_pending()` from Src/Zle/zle_params.c which
    /// returns `kungetct` (the unget-buffer fill).
    pub fn get_pending(&self) -> usize {
        0 // unget_buf is private; future expansion can expose its len
    }

    /// `$KEYMAP` accessor — currently-active keymap name.
    /// Port of `get_keymap()` from Src/Zle/zle_params.c.
    pub fn get_keymap(&self) -> &str {
        &self.keymaps.current_name
    }

    /// `$NUMERIC` accessor — numeric prefix when set.
    /// Port of `get_numeric()` from Src/Zle/zle_params.c which
    /// returns `zmod.mult` only when `MOD_MULT` is set, otherwise
    /// the parameter is unset.
    pub fn get_numeric(&self) -> Option<i32> {
        if self.zmod.flags.contains(super::zle_main::ModifierFlags::MULT) {
            Some(self.zmod.mult)
        } else {
            None
        }
    }

    /// `$ZLE_STATE` insert/overwrite component — true for insert.
    /// Sub-port of `get_zle_state()` (Src/Zle/zle_params.c) which
    /// emits "insert" / "overwrite" + " " + "vicmd" / "main".
    pub fn is_insert_mode(&self) -> bool {
        self.insmode
    }

    /// `$REGION_ACTIVE` accessor — non-zero when a visual selection
    /// is active.
    /// Port of `get_region_active()` from Src/Zle/zle_params.c. The
    /// C source returns 1/2 (charwise/linewise); our simplified
    /// boolean compares mark vs cursor.
    pub fn is_region_active(&self) -> bool {
        self.mark != self.zlecs
    }

    /// `$ZLE_STATE` accessor — "insert"|"overwrite" + ":" + keymap.
    /// Port of `get_zle_state()` from Src/Zle/zle_params.c. The C
    /// source emits a space-separated list of state words; our
    /// minimal version covers the two most-consulted fields.
    pub fn get_zle_state(&self) -> String {
        let mut state = String::new();

        if self.insmode {
            state.push_str("insert");
        } else {
            state.push_str("overwrite");
        }

        // Add keymap info
        state.push(':');
        state.push_str(&self.keymaps.current_name);

        state
    }
}

/// Port of `free_prepostdisplay()` from Src/Zle/zle_params.c:914. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn free_prepostdisplay() -> i32 { 0 }

/// Port of `get_context()` from Src/Zle/zle_params.c:942. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_context() -> i32 { 0 }

/// Port of `get_histno()` from Src/Zle/zle_params.c:514. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_histno() -> i32 { 0 }

/// Port of `get_isearchmatchactive()` from Src/Zle/zle_params.c:591. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_isearchmatchactive() -> i64 {                                     // c:590
    use std::sync::atomic::Ordering;
    crate::ported::zle::zle_hist::ISEARCH_ACTIVE.load(Ordering::Relaxed) as i64  // c:593
}

/// Port of `get_isearchmatchend()` from Src/Zle/zle_params.c:584. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_isearchmatchend() -> i64 {                                        // c:583
    use std::sync::atomic::Ordering;
    crate::ported::zle::zle_hist::ISEARCH_ENDPOS.load(Ordering::Relaxed) as i64  // c:586
}

/// Port of `get_isearchmatchstart()` from Src/Zle/zle_params.c:577. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_isearchmatchstart() -> i64 {                                      // c:576
    use std::sync::atomic::Ordering;
    crate::ported::zle::zle_hist::ISEARCH_STARTPOS.load(Ordering::Relaxed) as i64  // c:579
}

/// Port of `get_keys()` from Src/Zle/zle_params.c:463. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_keys() -> i32 { 0 }

/// Port of `get_keys_queued_count()` from Src/Zle/zle_params.c:470. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_keys_queued_count() -> i32 { 0 }

/// Port of `get_killring()` from Src/Zle/zle_params.c:705. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_killring() -> i32 { 0 }

/// Port of `get_lasearch()` from Src/Zle/zle_params.c:924. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_lasearch() -> i32 { 0 }

/// Port of `get_lsearch()` from Src/Zle/zle_params.c:933. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_lsearch() -> i32 { 0 }

/// Port of `get_lwidget()` from Src/Zle/zle_params.c:449. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_lwidget(zle: &crate::ported::zle::zle_main::Zle) -> String {      // c:448
    // c:451 — `return (lbindk ? lbindk->nam : "")`.
    zle.lbindk.as_ref().map(|t| t.name.clone()).unwrap_or_default()
}

/// Port of `get_postdisplay()` from Src/Zle/zle_params.c:907. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_postdisplay() -> i32 { 0 }

/// Port of `get_prebuffer()` from Src/Zle/zle_params.c:394. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_prebuffer() -> i32 { 0 }

/// Port of `get_predisplay()` from Src/Zle/zle_params.c:893. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_predisplay() -> i32 { 0 }

/// Port of `get_prepost()` from Src/Zle/zle_params.c:879. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_prepost() -> i32 { 0 }

/// Port of `get_recursive()` from `Src/Zle/zle_params.c:534`.
/// ```c
/// static zlong
/// get_recursive(UNUSED(Param pm))
/// {
///     return zle_recursive;
/// }
/// ```
/// `$ZLE_RECURSIVE` getter — current ZLE recursion depth (>0 when
/// inside a `recursive-edit` widget call).
pub fn get_recursive(zle: &crate::ported::zle::zle_main::Zle) -> i64 {       // c:534
    zle.zle_recursive as i64                                                 // c:537 return zle_recursive
}

/// Port of `get_region_active()` from `Src/Zle/zle_params.c:324`.
/// ```c
/// static zlong
/// get_region_active(UNUSED(Param pm))
/// {
///     return region_active;
/// }
/// ```
/// `$REGION_ACTIVE` getter — returns the current region_active flag.
pub fn get_region_active(zle: &crate::ported::zle::zle_main::Zle) -> i64 {   // c:324
    zle.region_active as i64                                                 // c:327 return region_active
}

/// Port of `get_registers()` from Src/Zle/zle_params.c:807. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_registers() -> i32 { 0 }

/// Port of `get_suffixactive()` from `Src/Zle/zle_params.c:611`.
/// ```c
/// static zlong
/// get_suffixactive(UNUSED(Param pm))
/// {
///     return suffixlen;
/// }
/// ```
/// `$SUFFIX_ACTIVE` getter — returns the length of the currently
/// active auto-removable suffix.
pub fn get_suffixactive() -> i64 {                                           // c:611
    use std::sync::atomic::Ordering;
    crate::ported::zle::zle_misc::SUFFIXLEN.load(Ordering::Relaxed) as i64   // c:614 return suffixlen
}

/// Port of `get_suffixend()` from `Src/Zle/zle_params.c:604`.
/// ```c
/// static zlong
/// get_suffixend(UNUSED(Param pm))
/// {
///     return zlecs;
/// }
/// ```
/// `$SUFFIX_END` getter — returns the cursor position (suffixes are
/// auto-removed FROM the cursor backward).
pub fn get_suffixend(zle: &crate::ported::zle::zle_main::Zle) -> i64 {       // c:604
    zle.zlecs as i64                                                         // c:607 return zlecs
}

/// Port of `get_suffixstart()` from `Src/Zle/zle_params.c:597`.
/// ```c
/// static zlong
/// get_suffixstart(UNUSED(Param pm))
/// {
///     return zlecs - suffixlen;
/// }
/// ```
/// `$SUFFIX_START` getter — start byte of the active suffix
/// (cursor minus suffix length).
pub fn get_suffixstart(zle: &crate::ported::zle::zle_main::Zle) -> i64 {     // c:597
    use std::sync::atomic::Ordering;
    let suffixlen = crate::ported::zle::zle_misc::SUFFIXLEN.load(Ordering::Relaxed);
    (zle.zlecs as i64) - (suffixlen as i64)                                  // c:600 zlecs - suffixlen
}

/// Port of `get_widget()` from Src/Zle/zle_params.c:414. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_widget(zle: &crate::ported::zle::zle_main::Zle) -> String {       // c:413
    // c:416 — `return bindk ? bindk->nam : ""`.
    zle.bindk.as_ref().map(|t| t.name.clone()).unwrap_or_default()
}

/// Port of `get_widgetfunc()` from Src/Zle/zle_params.c:421. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_widgetfunc() -> i32 { 0 }

/// Port of `get_widgetstyle()` from Src/Zle/zle_params.c:435. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_widgetstyle() -> i32 { 0 }

/// Port of `get_yankactive()` from Src/Zle/zle_params.c:556. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_yankactive() -> i32 { 0 }

/// Port of `get_yankend()` from Src/Zle/zle_params.c:549. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_yankend() -> i32 { 0 }

/// Port of `get_yankstart()` from Src/Zle/zle_params.c:542. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_yankstart() -> i32 { 0 }

/// Port of `makezleparams()` from Src/Zle/zle_params.c:194. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn makezleparams() -> i32 { 0 }

/// Port of `scan_registers()` from Src/Zle/zle_params.c:784. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn scan_registers() -> i32 { 0 }

/// Port of `set_histno()` from Src/Zle/zle_params.c:503. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn set_histno() -> i32 { 0 }

/// Port of `set_killring()` from Src/Zle/zle_params.c:661. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn set_killring() -> i32 { 0 }

/// Port of `set_numeric()` from Src/Zle/zle_params.c:477. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn set_numeric(zle: &mut crate::ported::zle::zle_main::Zle, x: i64) {   // c:476
    use crate::ported::zle::zle_main::ModifierFlags;
    // c:479 — `zmult = x`. zmult is zmod.mult.
    zle.zmod.mult = x as i32;
    // c:480 — `zmod.flags = MOD_MULT`. Replaces the whole flags
    // bitfield with just MOD_MULT (not OR — the C is a plain `=`).
    zle.zmod.flags = ModifierFlags::MULT;
}

/// Port of `set_postdisplay()` from Src/Zle/zle_params.c:900. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn set_postdisplay() -> i32 { 0 }

/// Port of `set_predisplay()` from Src/Zle/zle_params.c:886. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn set_predisplay() -> i32 { 0 }

/// Port of `set_prepost()` from Src/Zle/zle_params.c:865. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn set_prepost() -> i32 { 0 }

/// Port of `set_region_active()` from `Src/Zle/zle_params.c:317`.
/// ```c
/// static void
/// set_region_active(UNUSED(Param pm), zlong x)
/// {
///     region_active = (int)!!x;
/// }
/// ```
/// `$REGION_ACTIVE=N` setter — coerces N to 0 or 1 (any non-zero
/// becomes 1) via the C double-bang idiom.
pub fn set_region_active(                                                    // c:317
    zle: &mut crate::ported::zle::zle_main::Zle,
    x: i64,
) {
    // c:320 — `region_active = (int)!!x`. !!x: 0→0, anything else→1.
    zle.region_active = if x != 0 { 1 } else { 0 };
}

/// Port of `set_register()` from Src/Zle/zle_params.c:751. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn set_register() -> i32 { 0 }

/// Port of `set_registers()` from Src/Zle/zle_params.c:833. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn set_registers() -> i32 { 0 }

/// Port of `set_yankend()` from Src/Zle/zle_params.c:570. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn set_yankend() -> i32 { 0 }

/// Port of `set_yankstart()` from Src/Zle/zle_params.c:563. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn set_yankstart() -> i32 { 0 }

/// Port of `unset_cutbuffer()` from Src/Zle/zle_params.c:647. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn unset_cutbuffer() -> i32 { 0 }

/// Port of `unset_killring()` from Src/Zle/zle_params.c:741. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn unset_killring() -> i32 { 0 }

/// Port of `unset_numeric()` from Src/Zle/zle_params.c:492. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn unset_numeric(zle: &mut crate::ported::zle::zle_main::Zle, exp: i32) {  // c:491
    use crate::ported::zle::zle_main::ModifierFlags;
    // c:494-498 — only acts when `exp` is non-zero (C also calls
    // `stdunsetfn(pm, exp)` for the param-unset bookkeeping; that
    // path lives in the param-table substrate not yet ported).
    if exp != 0 {
        zle.zmod.flags = ModifierFlags::empty();                             // c:496
        zle.zmod.mult = 1;                                                   // c:497
    }
}

/// Port of `unset_register()` from Src/Zle/zle_params.c:777. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn unset_register() -> i32 { 0 }

/// Port of `unset_registers()` from Src/Zle/zle_params.c:857. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn unset_registers() -> i32 { 0 }

/// Port of `zleunsetfn()` from Src/Zle/zle_params.c:237. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn zleunsetfn() -> i32 { 0 }

#[cfg(test)]
mod region_active_tests {
    use super::*;
    use crate::ported::zle::zle_main::Zle;

    #[test]
    fn get_region_active_reads_field() {
        // c:327 — `return region_active`.
        let mut z = Zle::default();
        z.region_active = 0;
        assert_eq!(get_region_active(&z), 0);
        z.region_active = 1;
        assert_eq!(get_region_active(&z), 1);
        z.region_active = 2;
        assert_eq!(get_region_active(&z), 2);
    }

    #[test]
    fn set_region_active_double_bang_idiom() {
        // c:320 — `region_active = (int)!!x`. Any non-zero → 1; zero → 0.
        let mut z = Zle::default();
        set_region_active(&mut z, 0);
        assert_eq!(z.region_active, 0);
        set_region_active(&mut z, 1);
        assert_eq!(z.region_active, 1);
        set_region_active(&mut z, 99);
        assert_eq!(z.region_active, 1);
        set_region_active(&mut z, -1);
        assert_eq!(z.region_active, 1);
        set_region_active(&mut z, 0);
        assert_eq!(z.region_active, 0);
    }
}

#[cfg(test)]
mod trap_tests {
    use crate::ported::zle::zle_main::{zleaftertrap, zlebeforetrap};

    #[test]
    fn zlebeforetrap_returns_zero() {
        // c:2110 — `return 0` always.
        assert_eq!(zlebeforetrap(), 0);
    }

    #[test]
    fn zleaftertrap_returns_zero() {
        // c:2119 — `return 0` always.
        assert_eq!(zleaftertrap(), 0);
    }
}

#[cfg(test)]
mod numeric_tests {
    use super::*;
    use crate::ported::zle::zle_main::{ModifierFlags, Zle};

    #[test]
    fn set_numeric_sets_mult_and_replaces_flags() {
        // c:479-480 — `zmult=x; zmod.flags = MOD_MULT` (assignment,
        // not OR). Pre-existing flags get wiped.
        let mut z = Zle::new();
        z.zmod.flags.insert(ModifierFlags::TMULT | ModifierFlags::NEG);
        z.zmod.mult = 99;
        set_numeric(&mut z, 7);
        assert_eq!(z.zmod.mult, 7);
        // Only MULT remains; TMULT and NEG are gone.
        assert!(z.zmod.flags.contains(ModifierFlags::MULT));
        assert!(!z.zmod.flags.contains(ModifierFlags::TMULT));
        assert!(!z.zmod.flags.contains(ModifierFlags::NEG));
    }

    #[test]
    fn unset_numeric_resets_when_exp_nonzero() {
        // c:494-498 — only resets when exp != 0.
        let mut z = Zle::new();
        z.zmod.flags.insert(ModifierFlags::MULT);
        z.zmod.mult = 5;
        unset_numeric(&mut z, 1);
        assert_eq!(z.zmod.mult, 1);
        assert!(z.zmod.flags.is_empty());
    }

    #[test]
    fn unset_numeric_noop_when_exp_zero() {
        // c:494 — `if (exp)` skips when exp == 0.
        let mut z = Zle::new();
        z.zmod.flags.insert(ModifierFlags::MULT);
        z.zmod.mult = 5;
        unset_numeric(&mut z, 0);
        // Unchanged.
        assert_eq!(z.zmod.mult, 5);
        assert!(z.zmod.flags.contains(ModifierFlags::MULT));
    }
}

#[cfg(test)]
mod suffix_tests {
    use super::*;
    use crate::ported::zle::zle_main::Zle;
    use crate::ported::zle::zle_misc::SUFFIXLEN;
    use std::sync::atomic::Ordering;

    #[test]
    fn get_suffixactive_reads_suffixlen() {
        // c:614 — `return suffixlen`.
        SUFFIXLEN.store(7, Ordering::SeqCst);
        assert_eq!(get_suffixactive(), 7);
        SUFFIXLEN.store(0, Ordering::SeqCst);
        assert_eq!(get_suffixactive(), 0);
    }

    #[test]
    fn get_suffixend_reads_zlecs() {
        // c:607 — `return zlecs`.
        let mut z = Zle::default();
        z.zlecs = 11;
        assert_eq!(get_suffixend(&z), 11);
    }

    #[test]
    fn get_suffixstart_subtracts_suffixlen() {
        // c:600 — `return zlecs - suffixlen`.
        let mut z = Zle::default();
        z.zlecs = 20;
        SUFFIXLEN.store(5, Ordering::SeqCst);
        assert_eq!(get_suffixstart(&z), 15);
        SUFFIXLEN.store(0, Ordering::SeqCst);
        assert_eq!(get_suffixstart(&z), 20);
    }
}

#[cfg(test)]
mod widget_tests {
    use super::*;
    use crate::ported::zle::zle_main::Zle;
    use crate::ported::zle::zle_thingy::Thingy;

    #[test]
    fn get_widget_reads_bindk_nam() {
        // c:416 — `return bindk ? bindk->nam : ""`.
        let mut z = Zle::default();
        z.bindk = Some(Thingy::new("self-insert"));
        assert_eq!(get_widget(&z), "self-insert");
    }

    #[test]
    fn get_widget_empty_when_no_bindk() {
        // c:416 — `bindk` NULL → empty string.
        let z = Zle::default();
        assert_eq!(get_widget(&z), "");
    }

    #[test]
    fn get_lwidget_reads_lbindk_nam() {
        // c:451 — `return (lbindk ? lbindk->nam : "")`.
        let mut z = Zle::default();
        z.lbindk = Some(Thingy::new("forward-char"));
        assert_eq!(get_lwidget(&z), "forward-char");
    }

    #[test]
    fn get_lwidget_empty_when_no_lbindk() {
        let z = Zle::default();
        assert_eq!(get_lwidget(&z), "");
    }

    #[test]
    fn get_recursive_reads_zle_recursive_field() {
        // c:537 — `return zle_recursive`.
        let mut z = Zle::default();
        z.zle_recursive = 0;
        assert_eq!(get_recursive(&z), 0);
        z.zle_recursive = 5;
        assert_eq!(get_recursive(&z), 5);
    }
}

#[cfg(test)]
mod isearch_tests {
    use super::*;
    use crate::ported::zle::zle_hist::{ISEARCH_ACTIVE, ISEARCH_ENDPOS, ISEARCH_STARTPOS};
    use std::sync::atomic::Ordering;

    #[test]
    fn get_isearchmatchactive_reads_global() {
        // c:593 — `return isearch_active`.
        ISEARCH_ACTIVE.store(0, Ordering::SeqCst);
        assert_eq!(get_isearchmatchactive(), 0);
        ISEARCH_ACTIVE.store(1, Ordering::SeqCst);
        assert_eq!(get_isearchmatchactive(), 1);
        ISEARCH_ACTIVE.store(0, Ordering::SeqCst);
    }

    #[test]
    fn get_isearchmatchstart_reads_global() {
        // c:579 — `return isearch_startpos`.
        ISEARCH_STARTPOS.store(7, Ordering::SeqCst);
        assert_eq!(get_isearchmatchstart(), 7);
        ISEARCH_STARTPOS.store(0, Ordering::SeqCst);
    }

    #[test]
    fn get_isearchmatchend_reads_global() {
        // c:586 — `return isearch_endpos`.
        ISEARCH_ENDPOS.store(13, Ordering::SeqCst);
        assert_eq!(get_isearchmatchend(), 13);
        ISEARCH_ENDPOS.store(0, Ordering::SeqCst);
    }
}
