//! ZLE parameters
//!
//! Direct port from zsh/Src/Zle/zle_params.c
//!
//! Special parameters that expose ZLE state to shell scripts

use super::zle_main::Zle;

// `pub mod names` removed — Rust-fabricated namespace wrapping
// string literals. C source uses bare `"BUFFER"`/`"CURSOR"`/etc.
// in the `zleparams[]` table at Src/Zle/zle_params.c:38 directly.
// The mod had no callers.

// Each accessor below corresponds to one of the special parameters
// zsh exposes via Src/Zle/zle_params.c. The C source registers them
// through the `zleparams[]` table at zle_params.c:38; widget bodies
// (and shell scripts running inside ZLE) read or assign to them
// through the parameter system.
// ro means parameters are readonly, used from completion              // c:190
impl Zle {
    /// `$BUFFER` accessor — full edited line as a String.
    /// Port of `get_buffer(pm)` from Src/Zle/zle_params.c (the
    /// `BUFFER` getfn entry in `zleparams[]`).
    pub fn get_buffer(&self) -> String {                                    // c:258
        self.zleline.iter().collect()
    }

    /// `$BUFFER=s` setter — replace the full edited line.
    /// Port of `set_buffer(x)` from Src/Zle/zle_params.c (the
    /// `BUFFER` setfn entry); zsh clamps the cursor to the new
    /// length, mirrored here.
    pub fn set_buffer(&mut self, s: &str) {                                 // c:245
        self.zleline = s.chars().collect();
        self.zlell = self.zleline.len();
        self.zlecs = self.zlecs.min(self.zlell);
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// `$CURSOR` accessor — current cursor position (0-indexed).
    /// Port of `get_cursor(pm)` from Src/Zle/zle_params.c.
    pub fn get_cursor(&self) -> usize {                                     // c:281
        self.zlecs
    }

    /// `$CURSOR=pos` setter — clamped to buffer length.
    /// Port of `set_cursor(x)` from Src/Zle/zle_params.c.
    pub fn set_cursor(&mut self, pos: usize) {                              // c:267
        self.zlecs = pos.min(self.zlell);
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// `$LBUFFER` accessor — text before the cursor.
    /// Port of `get_lbuffer(pm)` from Src/Zle/zle_params.c.
    pub fn get_lbuffer(&self) -> String {                                   // c:355
        self.zleline[..self.zlecs].iter().collect()
    }

    /// `$LBUFFER=s` setter — replace text before the cursor; cursor
    /// lands at the new lbuffer's end.
    /// Port of `set_lbuffer(x)` from Src/Zle/zle_params.c.
    pub fn set_lbuffer(&mut self, s: &str) {                                // c:332
        let rbuf: String = self.zleline[self.zlecs..].iter().collect();
        self.zleline = s.chars().chain(rbuf.chars()).collect();
        self.zlell = self.zleline.len();
        self.zlecs = s.chars().count();
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// `$RBUFFER` accessor — text after the cursor.
    /// Port of `get_rbuffer(pm)` from Src/Zle/zle_params.c.
    pub fn get_rbuffer(&self) -> String {                                   // c:384
        self.zleline[self.zlecs..].iter().collect()
    }

    /// `$RBUFFER=s` setter — replace text after the cursor.
    /// Port of `set_rbuffer(x)` from Src/Zle/zle_params.c.
    pub fn set_rbuffer(&mut self, s: &str) {                                // c:364
        let lbuf: String = self.zleline[..self.zlecs].iter().collect();
        self.zleline = lbuf.chars().chain(s.chars()).collect();
        self.zlell = self.zleline.len();
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// `$CUTBUFFER` accessor — most-recent kill-ring entry.
    /// Port of `get_cutbuffer(pm)` from Src/Zle/zle_params.c which
    /// reads `cutbuf` (the unnamed kill register).
    pub fn get_cutbuffer(&self) -> String {                                 // c:619
        self.killring
            .front()
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }

    /// `$CUTBUFFER=s` setter — overwrite the front of the kill ring.
    /// Port of `set_cutbuffer(x)` from Src/Zle/zle_params.c.
    pub fn set_cutbuffer(&mut self, s: &str) {                              // c:629
        let chars: Vec<char> = s.chars().collect();
        if self.killring.is_empty() {
            self.killring.push_front(chars);
        } else {
            self.killring[0] = chars;
        }
    }

    /// `$MARK` accessor — current mark position.
    /// Port of `get_mark(pm)` from Src/Zle/zle_params.c.
    pub fn get_mark(&self) -> usize {                                       // c:311
        self.mark
    }

    /// `$MARK=pos` setter — clamp to buffer length.
    /// Port of `set_mark(x)` from Src/Zle/zle_params.c.
    pub fn set_mark(&mut self, pos: usize) {                                // c:299
        self.mark = pos.min(self.zlell);
    }

    /// `$BUFFERLINES` accessor — number of newline-separated lines.
    /// Port of `get_bufferlines(pm)` from Src/Zle/zle_params.c.
    pub fn get_bufferlines(&self) -> usize {                                // c:521
        self.zleline.iter().filter(|&&c| c == '\n').count() + 1
    }

    /// `$PENDING` accessor — bytes waiting in the input queue.
    /// Port of `get_pending(pm)` from Src/Zle/zle_params.c which
    /// returns `kungetct` (the unget-buffer fill).
    pub fn get_pending(&self) -> usize {                                    // c:528
        0 // unget_buf is private; future expansion can expose its len
    }

    /// `$KEYMAP` accessor — currently-active keymap name.
    /// Port of `get_keymap(pm)` from Src/Zle/zle_params.c.
    pub fn get_keymap(&self) -> String {                                    // c:456
        crate::ported::zle::zle_keymap::curkeymapname().clone()
    }

    /// `$NUMERIC` accessor — numeric prefix when set.
    /// Port of `get_numeric(pm)` from Src/Zle/zle_params.c which
    /// returns `zmod.mult` only when `MOD_MULT` is set, otherwise
    /// the parameter is unset.
    pub fn get_numeric(&self) -> Option<i32> {                              // c:485
        if self.zmod.flags & super::zle_h::MOD_MULT != 0 {
            Some(self.zmod.mult)
        } else {
            None
        }
    }

    /// `$ZLE_STATE` insert/overwrite component — true for insert.
    /// Sub-port of `get_zle_state(pm)` (Src/Zle/zle_params.c) which
    /// emits "insert" / "overwrite" + " " + "vicmd" / "main".
    pub fn is_insert_mode(&self) -> bool {
        (crate::ported::zle::zle_main::INSMODE.load(std::sync::atomic::Ordering::SeqCst) != 0)
    }

    /// `$REGION_ACTIVE` accessor — non-zero when a visual selection
    /// is active.
    /// Port of `get_region_active(pm)` from Src/Zle/zle_params.c. The
    /// C source returns 1/2 (charwise/linewise); our simplified
    /// boolean compares mark vs cursor.
    pub fn is_region_active(&self) -> bool {
        self.mark != self.zlecs
    }

    /// `$ZLE_STATE` accessor — "insert"|"overwrite" + ":" + keymap.
    /// Port of `get_zle_state(pm)` from Src/Zle/zle_params.c. The C
    /// source emits a space-separated list of state words; our
    /// minimal version covers the two most-consulted fields.
    pub fn get_zle_state(&self) -> String {
        let mut state = String::new();

        if (crate::ported::zle::zle_main::INSMODE.load(std::sync::atomic::Ordering::SeqCst) != 0) {
            state.push_str("insert");
        } else {
            state.push_str("overwrite");
        }

        // Add keymap info
        state.push(':');
        state.push_str(&crate::ported::zle::zle_keymap::curkeymapname());

        state
    }
}

/// Port of `free_prepostdisplay()` from Src/Zle/zle_params.c:914.
pub fn free_prepostdisplay() {                                               // c:914
    use crate::ported::zle::zle_misc::{POSTDISPLAY, PREDISPLAY};
    use std::sync::Mutex;
    // c:916-917 — `if (predisplaylen) set_prepost(&predisplay, &predisplaylen, NULL)`.
    PREDISPLAY.get_or_init(|| Mutex::new(String::new())).lock().unwrap().clear();
    // c:918-919 — same for postdisplay.
    POSTDISPLAY.get_or_init(|| Mutex::new(String::new())).lock().unwrap().clear();
}

/// Port of `get_context(pm)` from Src/Zle/zle_params.c:942.
pub fn get_context(zle: &crate::ported::zle::zle_main::Zle) -> &'static str {  // c:942
    use crate::ported::zsh_h::{ZLCON_LINE_CONT, ZLCON_SELECT, ZLCON_VARED};
    // c:944-958 — switch on zlecontext → "cont" / "select" / "vared" / "line".
    match crate::ported::zle::zle_main::ZLECONTEXT.load(std::sync::atomic::Ordering::SeqCst) {
        x if x == ZLCON_LINE_CONT => "cont",                                  // c:945-946
        x if x == ZLCON_SELECT    => "select",                                // c:949-950
        x if x == ZLCON_VARED     => "vared",                                 // c:953-954
        _                         => "line",                                  // c:957-958 default
    }
}

/// Port of `get_histno(pm)` from Src/Zle/zle_params.c:514.
pub fn get_histno(zle: &crate::ported::zle::zle_main::Zle) -> i64 {          // c:513
    // c:516 — `return histline`. zshrs tracks the editing history
    // line via the History.cursor field (offset into entries Vec).
    zle.history.cursor as i64
}

/// Port of `get_isearchmatchactive(pm)` from Src/Zle/zle_params.c:591.
pub fn get_isearchmatchactive() -> i64 {                                     // c:590
    use std::sync::atomic::Ordering;
    crate::ported::zle::zle_hist::ISEARCH_ACTIVE.load(Ordering::Relaxed) as i64  // c:593
}

/// Port of `get_isearchmatchend(pm)` from Src/Zle/zle_params.c:584.
pub fn get_isearchmatchend() -> i64 {                                        // c:583
    use std::sync::atomic::Ordering;
    crate::ported::zle::zle_hist::ISEARCH_ENDPOS.load(Ordering::Relaxed) as i64  // c:586
}

/// Port of `get_isearchmatchstart(pm)` from Src/Zle/zle_params.c:577.
pub fn get_isearchmatchstart() -> i64 {                                      // c:576
    use std::sync::atomic::Ordering;
    crate::ported::zle::zle_hist::ISEARCH_STARTPOS.load(Ordering::Relaxed) as i64  // c:579
}

/// Port of `get_keys(pm)` from Src/Zle/zle_params.c:463.
pub fn get_keys(zle: &crate::ported::zle::zle_main::Zle) -> Vec<u8> {        // c:462
    // c:465 — `return keybuf`. The active keymap-walk byte buffer.
    let _ = zle;
    crate::ported::zle::zle_keymap::keybuf.lock().unwrap().clone()
}

/// Port of `get_keys_queued_count(pm)` from Src/Zle/zle_params.c:470.
pub fn get_keys_queued_count(zle: &crate::ported::zle::zle_main::Zle) -> i64 {  // c:469
    // c:472 — `return kungetct`. Bytes pending in the unget queue.
    zle.unget_buf.len() as i64
}

/// Port of `get_killring(pm)` from Src/Zle/zle_params.c:705.
pub fn get_killring(zle: &crate::ported::zle::zle_main::Zle) -> Vec<String> {  // c:704
    // c:706-733 — return kring entries with most-recently-killed
    // first. Empty entries returned as "" so the array length always
    // equals kringsize. zshrs holds the kill ring as
    // VecDeque<ZleString> where push_front puts newest at index 0,
    // so we iterate forward.
    zle.killring.iter()
        .map(|entry| entry.iter().collect::<String>())
        .collect()
}

/// Port of `get_lasearch(pm)` from Src/Zle/zle_params.c:924.
pub fn get_lasearch() -> String {                                            // c:923
    use crate::ported::zle::zle_misc::PREVIOUS_ABORTED_SEARCH;
    use std::sync::Mutex;
    // c:926-928 — `previous_aborted_search ? : ""`.
    PREVIOUS_ABORTED_SEARCH.get_or_init(|| Mutex::new(String::new()))
        .lock().unwrap().clone()
}

/// Port of `get_lsearch(pm)` from Src/Zle/zle_params.c:933.
pub fn get_lsearch() -> String {                                             // c:932
    use crate::ported::zle::zle_misc::PREVIOUS_SEARCH;
    use std::sync::Mutex;
    // c:935-937 — `previous_search ? : ""`.
    PREVIOUS_SEARCH.get_or_init(|| Mutex::new(String::new()))
        .lock().unwrap().clone()
}

/// Port of `get_lwidget(pm)` from Src/Zle/zle_params.c:449.
pub fn get_lwidget(zle: &crate::ported::zle::zle_main::Zle) -> String {      // c:448
    // c:451 — `return (lbindk ? lbindk->nam : "")`.
    zle.lbindk.as_ref().map(|t| t.nam.clone()).unwrap_or_default()
}

/// Port of `get_postdisplay(pm)` from Src/Zle/zle_params.c:907.
pub fn get_postdisplay() -> String {                                         // c:906
    use crate::ported::zle::zle_misc::POSTDISPLAY;
    use std::sync::Mutex;
    // c:909 — `return get_prepost(postdisplay, postdisplaylen)` →
    // zlelineasstring(...). Return the raw String.
    POSTDISPLAY.get_or_init(|| Mutex::new(String::new()))
        .lock().unwrap().clone()
}

/// Port of `get_prebuffer(pm)` from Src/Zle/zle_params.c:394.
pub fn get_prebuffer(zle: &crate::ported::zle::zle_main::Zle) -> String {    // c:394
    // C body c:396-410 — `if (!stackhist) return ztrdup("");
    //                     dputs(...prepended buffer...)`. Returns the
    //                     stacked-line buffer (multi-line input not
    //                     yet committed to current zleline). Without
    //                     stackhist tracking we return empty.
    let _ = zle;
    String::new()
}

/// Port of `get_predisplay(pm)` from Src/Zle/zle_params.c:893.
pub fn get_predisplay() -> String {                                          // c:892
    use crate::ported::zle::zle_misc::PREDISPLAY;
    use std::sync::Mutex;
    PREDISPLAY.get_or_init(|| Mutex::new(String::new()))
        .lock().unwrap().clone()
}

/// Port of `get_prepost(text, len)` from Src/Zle/zle_params.c:879.
pub fn get_prepost(text: &str, len: usize) -> String {                       // c:878
    // c:881 — `return zlelineasstring(text, len, 0, NULL, NULL, 1)`.
    // In Rust the caller already owns a String; just truncate to len.
    text.chars().take(len).collect()
}

/// Port of `get_recursive(pm)` from `Src/Zle/zle_params.c:534`.
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
    crate::ported::zle::zle_main::ZLE_RECURSIVE.load(std::sync::atomic::Ordering::SeqCst) as i64                                                 // c:537 return zle_recursive
}

/// Port of `get_region_active(pm)` from `Src/Zle/zle_params.c:324`.
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

/// Port of `get_registers(name)` from Src/Zle/zle_params.c:807.
pub fn get_registers(zle: &crate::ported::zle::zle_main::Zle, name: &str) -> Option<String> {  // c:806
    // c:815-820 — name[1] non-zero → invalid; '0'..'9' → idx = name-'0'+26;
    // 'a'..'z' → idx = name-'a'.
    let bytes = name.as_bytes();
    if bytes.len() != 1 {
        return None;
    }
    let c = bytes[0];
    let idx: i32 = if c.is_ascii_digit() {
        (c - b'0') as i32 + 26
    } else if c.is_ascii_lowercase() {
        (c - b'a') as i32
    } else {
        return None;                                                         // c:822-824 (vbuf==-1)
    };
    // c:798 — `pm->u.str = zlelineasstring(vibuf[i].buf, ...)`.
    if (idx as usize) < zle.vibuf.len() {
        Some(zle.vibuf[idx as usize].iter().collect::<String>())
    } else {
        None
    }
}

/// Port of `get_suffixactive(pm)` from `Src/Zle/zle_params.c:611`.
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

/// Port of `get_suffixend(pm)` from `Src/Zle/zle_params.c:604`.
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

/// Port of `get_suffixstart(pm)` from `Src/Zle/zle_params.c:597`.
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

/// Port of `get_widget(pm)` from Src/Zle/zle_params.c:414.
pub fn get_widget(zle: &crate::ported::zle::zle_main::Zle) -> String {       // c:413
    // c:416 — `return bindk ? bindk->nam : ""`.
    zle.bindk.as_ref().map(|t| t.nam.clone()).unwrap_or_default()
}

/// Port of `get_widgetfunc(pm)` from Src/Zle/zle_params.c:421.
pub fn get_widgetfunc(zle: &crate::ported::zle::zle_main::Zle) -> String {   // c:420
    use crate::ported::zle::widget::{WidgetFlags, WidgetFunc};
    // c:423-430 — read bindk->widget. C union dispatches:
    //   WIDGET_INT  → ".internal"  (c:426-427)
    //   WIDGET_NCOMP → comp.func   (c:428-429)
    //   else → fnnam               (c:430)
    let Some(t) = zle.bindk.as_ref() else {
        return String::new();
    };
    let Some(w) = t.widget.as_ref() else {
        return String::new();
    };
    if w.flags.contains(WidgetFlags::INT) {
        return ".internal".to_string();
    }
    // No NCOMP comp.func/wid in current Widget shape (would be in
    // WidgetFunc::Comp variant); collapse to the User-fn case.
    match &w.func {
        WidgetFunc::User(name) => name.clone(),
        WidgetFunc::Internal(_) => ".internal".to_string(),
    }
}

/// Port of `get_widgetstyle(pm)` from Src/Zle/zle_params.c:435.
pub fn get_widgetstyle(zle: &crate::ported::zle::zle_main::Zle) -> String {  // c:434
    use crate::ported::zle::widget::WidgetFlags;
    // c:437-444 — read bindk->widget. INT → ".internal"; NCOMP →
    // comp.wid (the underlying widget name); else "".
    let Some(t) = zle.bindk.as_ref() else {
        return String::new();
    };
    let Some(w) = t.widget.as_ref() else {
        return String::new();
    };
    if w.flags.contains(WidgetFlags::INT) {
        return ".internal".to_string();
    }
    // No NCOMP comp.wid in current shape — would be t.nam for
    // a -C-bound completion widget. Fall through to "".
    String::new()                                                            // c:444
}

/// Port of `get_yankactive(pm)` from Src/Zle/zle_params.c:556.
pub fn get_yankactive(zle: &crate::ported::zle::zle_main::Zle) -> i64 {      // c:555
    // c:558 — `return !!(lastcmd & ZLE_YANK) + !!(lastcmd & ZLE_YANKAFTER)`.
    use crate::ported::zle::widget::WidgetFlags;
    let _ = zle;
    let last = WidgetFlags::from_bits_truncate(
        crate::ported::zle::zle_main::LASTCMD.load(std::sync::atomic::Ordering::SeqCst),
    );
    let yank      = last.contains(WidgetFlags::YANK)      as i64;
    let yankafter = last.contains(WidgetFlags::YANKAFTER) as i64;
    yank + yankafter
}

/// Port of `get_yankend(pm)` from Src/Zle/zle_params.c:549.
pub fn get_yankend(zle: &crate::ported::zle::zle_main::Zle) -> i64 {         // c:548
    // c:551 — `return yanke`.
    zle.yank_end as i64
}

/// Port of `get_yankstart(pm)` from Src/Zle/zle_params.c:542.
pub fn get_yankstart(zle: &crate::ported::zle::zle_main::Zle) -> i64 {       // c:541
    // c:544 — `return yankb`.
    zle.yank_start as i64
}

/// Direct port of `void makezleparams(int ro)` from
/// `Src/Zle/zle_params.c:194-228`. Registers the `$BUFFER`,
/// `$LBUFFER`, `$RBUFFER`, `$CURSOR`, `$MARK`, `$NUMERIC`,
/// `$REGION_ACTIVE`, `$WIDGET`, `$LASTWIDGET`, `$KEYS`,
/// `$BUFFERLINES`, `$CONTEXT`, `$HISTNO`, `$WIDGETSTYLE`,
/// `$WIDGETFUNC` parameters in the param table for the duration
/// of a widget call.
///
/// Full GSU custom-getter dispatch (c:196-228) requires
/// per-param Param.gsu hooks; the Rust port writes the current
/// ZLE state snapshot directly via setsparam / setiparam so user
/// widget functions see live values. When a widget mutates
/// $BUFFER it goes through the canonical paramtab write path
/// that already exists.
pub fn makezleparams(_ro: i32) {                                             // c:194
    use crate::ported::zle::compcore::{ZLECS, ZLELINE, ZMULT};

    let line = ZLELINE.get_or_init(|| std::sync::Mutex::new(String::new()))
        .lock().map(|g| g.clone()).unwrap_or_default();
    let cs = ZLECS.load(std::sync::atomic::Ordering::Relaxed) as usize;
    let (lbuf, rbuf) = if cs <= line.len() {
        (line[..cs].to_string(), line[cs..].to_string())
    } else {
        (line.clone(), String::new())
    };

    let _ = crate::ported::params::setsparam("BUFFER", &line);              // c:zleparams[0]
    let _ = crate::ported::params::setsparam("LBUFFER", &lbuf);             // c:zleparams[1]
    let _ = crate::ported::params::setsparam("RBUFFER", &rbuf);             // c:zleparams[2]
    let _ = crate::ported::params::setiparam(
        "CURSOR",
        ZLECS.load(std::sync::atomic::Ordering::Relaxed) as i64,
    );                                                                       // c:zleparams[3]
    let _ = crate::ported::params::setiparam("NUMERIC", ZMULT.load(
        std::sync::atomic::Ordering::Relaxed,
    ) as i64);                                                               // c:zleparams[7]
    // $BUFFERLINES — count of newlines in BUFFER + 1.
    let lines = line.chars().filter(|c| *c == '\n').count() as i64 + 1;
    let _ = crate::ported::params::setiparam("BUFFERLINES", lines);          // c:zleparams[10]
}

/// Port of `scan_registers(func, flags)` from Src/Zle/zle_params.c:784.
pub fn scan_registers(_t: i32, _flags: i32) {                                // c:784
    // C body c:786-840 — walks vibuf[0..36] enumerating non-empty
    //                    vi register names ('a'..'z', '0'..'9') for
    //                    `printf -v` and `(${(@k)registers})` queries.
    //                    Without param-table hashparam node integration:
    //                    no-op.
}

/// Port of `set_histno(x)` from Src/Zle/zle_params.c:503.
pub fn set_histno(zle: &mut crate::ported::zle::zle_main::Zle, x: i64) {     // c:502
    // c:505-509 — `Histent he = quietgethist(x); if (!he) return;
    //              zle_setline(he)`.
    // zshrs uses History.cursor as the active history index. Clamp
    // to entries.len() when x is out of range (matches the
    // quietgethist NULL-result early-return).
    let idx = x.max(0) as usize;
    if idx <= zle.history.entries.len() {
        zle.history.cursor = idx;
    }
}

/// Port of `set_killring(x)` from Src/Zle/zle_params.c:661.
pub fn set_killring(zle: &mut crate::ported::zle::zle_main::Zle, x: Option<&[String]>) {  // c:660
    // c:667-672 — `if (kring) { free each kptr->buf; zfree(kring) }`.
    // Then either rebuild from `x` or leave NULL.
    zle.killring.clear();
    if let Some(arr) = x {
        for entry in arr {
            zle.killring.push_back(entry.chars().collect());
        }
    }
}

/// Port of `set_numeric(x)` from Src/Zle/zle_params.c:477.
pub fn set_numeric(zle: &mut crate::ported::zle::zle_main::Zle, x: i64) {   // c:476
    use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
    // c:479 — `zmult = x`. zmult is zmod.mult.
    zle.zmod.mult = x as i32;
    // c:480 — `zmod.flags = MOD_MULT`. Replaces the whole flags
    // bitfield with just MOD_MULT (not OR — the C is a plain `=`).
    zle.zmod.flags = MOD_MULT;
}

/// Port of `set_postdisplay(x)` from Src/Zle/zle_params.c:900.
pub fn set_postdisplay(x: Option<&str>) {                                    // c:899
    use crate::ported::zle::zle_misc::POSTDISPLAY;
    use std::sync::Mutex;
    let g = POSTDISPLAY.get_or_init(|| Mutex::new(String::new()));
    let mut buf = g.lock().unwrap();
    buf.clear();
    if let Some(s) = x {
        buf.push_str(s);
    }
}

/// Port of `set_predisplay(x)` from Src/Zle/zle_params.c:886.
pub fn set_predisplay(x: Option<&str>) {                                     // c:885
    use crate::ported::zle::zle_misc::PREDISPLAY;
    use std::sync::Mutex;
    let g = PREDISPLAY.get_or_init(|| Mutex::new(String::new()));
    let mut buf = g.lock().unwrap();
    buf.clear();
    if let Some(s) = x {
        buf.push_str(s);
    }
}

/// Port of `set_prepost(textvar, lenvar, x)` from Src/Zle/zle_params.c:865.
pub fn set_prepost(textvar: &mut String, lenvar: &mut usize, x: Option<&str>) {  // c:864
    // c:867-871 — `if (*lenvar) free(*textvar); *textvar=NULL; *lenvar=0`.
    if *lenvar != 0 {
        textvar.clear();
        *lenvar = 0;
    }
    // c:872-874 — if x: `*textvar = stringaszleline(x, 0, lenvar, ...)`.
    if let Some(s) = x {
        textvar.push_str(s);
        *lenvar = s.chars().count();
    }
}

/// Port of `set_region_active(x)` from `Src/Zle/zle_params.c:317`.
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

/// Port of `set_register(pm, value)` from Src/Zle/zle_params.c:751.
pub fn set_register(zle: &mut crate::ported::zle::zle_main::Zle, name: char, value: &str) -> i32 {  // c:750
    // c:759-763 — '0'..'9' → offset = '0' - 26;  'a'..'z' → offset = 'a'.
    // (Vi register table layout: 0..25 = a..z, 26..35 = 0..9.)
    let idx: i32 = if ('0'..='9').contains(&name) {
        // c:760 — `offset = '0' - 26` → idx = name - '0' + 26.
        name as i32 - b'0' as i32 + 26
    } else if ('a'..='z').contains(&name) {
        // c:761-762 — `offset = 'a'` → idx = name - 'a'.
        name as i32 - b'a' as i32
    } else {
        // c:765 — invalid register; C reports zerr and returns.
        return 1;
    };
    // c:769-772 — `vbuf = &vibuf[name-offset]; if (*value)
    //              vbuf->buf = stringaszleline(value, 0, &n, ...);
    //              vbuf->len = n`.
    if (idx as usize) < zle.vibuf.len() {
        zle.vibuf[idx as usize] = value.chars().collect();
    }
    0
}

/// Port of `set_registers(pm, ht)` from Src/Zle/zle_params.c:833.
pub fn set_registers(zle: &mut crate::ported::zle::zle_main::Zle,            // c:833
                     map: &std::collections::HashMap<String, String>) {
    // C body c:835-855 — for each (name, value) in the assoc-array
    //                    being assigned to $registers, invoke
    //                    set_register. Names outside [a-z0-9] beep.
    for (name, value) in map {
        if let Some(ch) = name.chars().next() {
            let _ = set_register(zle, ch, value);
        }
    }
}

/// Port of `set_yankend(i)` from Src/Zle/zle_params.c:570.
pub fn set_yankend(zle: &mut crate::ported::zle::zle_main::Zle, i: i64) {    // c:569
    // c:572 — `yanke = i`.
    zle.yank_end = i.max(0) as usize;
}

/// Port of `set_yankstart(i)` from Src/Zle/zle_params.c:563.
pub fn set_yankstart(zle: &mut crate::ported::zle::zle_main::Zle, i: i64) {  // c:562
    // c:565 — `yankb = i`.
    zle.yank_start = i.max(0) as usize;
}

/// Port of `unset_cutbuffer(pm, exp)` from Src/Zle/zle_params.c:647.
pub fn unset_cutbuffer(zle: &mut crate::ported::zle::zle_main::Zle, exp: i32) {  // c:646
    // c:649-655 — `if (exp) { stdunsetfn; if (cutbuf.buf) { free; NULL; len=0 } }`.
    if exp != 0 {
        // zshrs uses VecDeque for the kill ring; the "primary" cut
        // buffer is the front entry. Clearing means popping it.
        zle.killring.pop_front();
    }
}

/// Port of `unset_killring(pm, exp)` from Src/Zle/zle_params.c:741.
pub fn unset_killring(zle: &mut crate::ported::zle::zle_main::Zle, exp: i32) {  // c:740
    // c:743-746 — `if (exp) { set_killring(pm, NULL); stdunsetfn(...) }`.
    if exp != 0 {
        set_killring(zle, None);
        // stdunsetfn handles param-table bookkeeping — substrate.
    }
}

/// Direct port of `static void unset_numeric(Param pm, int exp)` from
/// `Src/Zle/zle_params.c:491-499`.
/// ```c
/// stdunsetfn(pm, exp);
/// if (exp) {
///     zmod.flags &= ~(MOD_MULT|MOD_TMULT);
///     zmod.mult = 1;
/// }
/// ```
///
/// The Rust call signature here takes `&mut Zle` instead of `Param`
/// because the canonical Rust home for `zmod` is `Zle.zmod`. The
/// `stdunsetfn` half of the C body fires from the Param.gsu.unsetfn
/// vtable hook upstream — this fn just performs the zmod side.
/// Port of `unset_numeric` from `Src/Zle/zle_params.c:491`.
pub fn unset_numeric(zle: &mut crate::ported::zle::zle_main::Zle, exp: i32) { // c:491
    use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
    if exp != 0 {                                                            // c:494
        zle.zmod.flags = 0;                             // c:496
        zle.zmod.mult = 1;                                                   // c:497
    }
}

/// Port of `unset_register(pm)` from Src/Zle/zle_params.c:777.
pub fn unset_register(zle: &mut crate::ported::zle::zle_main::Zle, name: char, _exp: i32) {  // c:776
    // c:778-779 — `set_register(pm, "")`. Single-line body.
    let _ = set_register(zle, name, "");
}

/// Port of `unset_registers(pm, exp)` from Src/Zle/zle_params.c:857.
pub fn unset_registers(zle: &mut crate::ported::zle::zle_main::Zle, exp: i32) { // c:857
    // C body c:859-870 — `if (exp) { for (i...) { vibuf[i].buf=NULL;
    //                              vibuf[i].len = 0; } stdunsetfn(...) }`.
    if exp != 0 {
        for buf in zle.vibuf.iter_mut() {
            buf.clear();
        }
    }
}

/// Direct port of `static void zleunsetfn(Param pm, int exp)` from
/// `Src/Zle/zle_params.c:237-242`.
/// ```c
/// stdunsetfn(pm, exp);
/// pm->gsu.s = &nullsetscalar_gsu;
/// ```
/// Called when one of ZLE's special parameters ($BUFFER etc.) is
/// `unset`. C swaps the GSU to the null-setter so subsequent
/// reads return empty and writes are dropped.
pub fn zleunsetfn(pm: &mut crate::ported::zsh_h::param, exp: i32) {          // c:237
    crate::ported::params::stdunsetfn(pm, exp);                              // c:239
    // c:240 — `pm->gsu.s = &nullsetscalar_gsu`. The GSU vtable swap
    // requires the canonical Rust Param.gsu field which is part of
    // the params.rs port. The stdunsetfn call above already sets
    // PM_UNSET; further reads return empty via the default getter.
}

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
    use crate::ported::zle::zle_main::Zle; use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};

    #[test]
    fn set_numeric_sets_mult_and_replaces_flags() {
        // c:479-480 — `zmult=x; zmod.flags = MOD_MULT` (assignment,
        // not OR). Pre-existing flags get wiped.
        let mut z = Zle::new();
        z.zmod.flags |= MOD_TMULT | MOD_NEG;
        z.zmod.mult = 99;
        set_numeric(&mut z, 7);
        assert_eq!(z.zmod.mult, 7);
        // Only MULT remains; TMULT and NEG are gone.
        assert!(z.zmod.flags & MOD_MULT != 0);
        assert_eq!(z.zmod.flags & MOD_TMULT, 0);
        assert_eq!(z.zmod.flags & MOD_NEG, 0);
    }

    #[test]
    fn unset_numeric_resets_when_exp_nonzero() {
        // c:494-498 — only resets when exp != 0.
        let mut z = Zle::new();
        z.zmod.flags |= MOD_MULT;
        z.zmod.mult = 5;
        unset_numeric(&mut z, 1);
        assert_eq!(z.zmod.mult, 1);
        assert_eq!(z.zmod.flags, 0);
    }

    #[test]
    fn unset_numeric_noop_when_exp_zero() {
        // c:494 — `if (exp)` skips when exp == 0.
        let mut z = Zle::new();
        z.zmod.flags |= MOD_MULT;
        z.zmod.mult = 5;
        unset_numeric(&mut z, 0);
        // Unchanged.
        assert_eq!(z.zmod.mult, 5);
        assert!(z.zmod.flags & MOD_MULT != 0);
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
        let z = Zle::default();
        crate::ported::zle::zle_main::ZLE_RECURSIVE.store(0, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(get_recursive(&z), 0);
        crate::ported::zle::zle_main::ZLE_RECURSIVE.store(5, std::sync::atomic::Ordering::SeqCst);
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

#[cfg(test)]
mod batch_getters_tests {
    use super::*;
    use crate::ported::zle::widget::WidgetFlags;
    use crate::ported::zle::zle_main::Zle;

    #[test]
    fn get_histno_reads_history_cursor() {
        let mut z = Zle::default();
        z.history.cursor = 7;
        assert_eq!(get_histno(&z), 7);
    }

    #[test]
    fn get_keys_returns_keybuf_clone() {
        let z = Zle::default();
        *crate::ported::zle::zle_keymap::keybuf.lock().unwrap() = vec![0x1b, b'a'];
        assert_eq!(get_keys(&z), vec![0x1b, b'a']);
    }

    #[test]
    fn get_keys_queued_count_returns_unget_len() {
        let mut z = Zle::default();
        z.unget_buf.push_back(b'a');
        z.unget_buf.push_back(b'b');
        z.unget_buf.push_back(b'c');
        assert_eq!(get_keys_queued_count(&z), 3);
    }

    #[test]
    fn get_yankactive_reads_lastcmd_flags() {
        let z = Zle::default();
        use crate::ported::zle::zle_main::LASTCMD;
        use std::sync::atomic::Ordering;
        LASTCMD.store(WidgetFlags::empty().bits(), Ordering::SeqCst);
        assert_eq!(get_yankactive(&z), 0);
        LASTCMD.store(WidgetFlags::YANK.bits(), Ordering::SeqCst);
        // YANK = YANKAFTER | YANKBEFORE; both bits set so contains
        // YANK and contains YANKAFTER → 1+1 = 2.
        assert_eq!(get_yankactive(&z), 2);
        LASTCMD.store(WidgetFlags::YANKBEFORE.bits(), Ordering::SeqCst);
        // YANKBEFORE only: contains(YANK) checks both bits set, so it's
        // false; contains(YANKAFTER) is also false → 0+0 = 0.
        assert_eq!(get_yankactive(&z), 0);
    }

    #[test]
    fn get_yankstart_yankend_read_fields() {
        let mut z = Zle::default();
        z.yank_start = 3;
        z.yank_end = 8;
        assert_eq!(get_yankstart(&z), 3);
        assert_eq!(get_yankend(&z), 8);
    }

    #[test]
    fn set_yankstart_yankend_write_fields() {
        let mut z = Zle::default();
        set_yankstart(&mut z, 5);
        set_yankend(&mut z, 11);
        assert_eq!(z.yank_start, 5);
        assert_eq!(z.yank_end, 11);
    }
}

#[cfg(test)]
mod keybuf_tests {
    use crate::ported::zle::zle_keymap::{addkeybuf, freekeynode, KeyBinding};
    use crate::ported::zle::zle_main::Zle;

    #[test]
    fn addkeybuf_plain_byte() {
        let mut z = Zle::default();
        crate::ported::zle::zle_keymap::keybuf.lock().unwrap().clear();
        addkeybuf(&mut z, b'a' as i32);
        assert_eq!(*crate::ported::zle::zle_keymap::keybuf.lock().unwrap(), vec![b'a']);
    }

    #[test]
    fn addkeybuf_meta_quoted() {
        let mut z = Zle::default();
        // 0xa0 needs Meta-quoting → 0x83 then (0xa0 ^ 0x20) = 0x80
        crate::ported::zle::zle_keymap::keybuf.lock().unwrap().clear();
        addkeybuf(&mut z, 0xa0);
        assert_eq!(*crate::ported::zle::zle_keymap::keybuf.lock().unwrap(), vec![0x83, 0x80]);
    }

    #[test]
    fn freekeynode_consumes_binding() {
        // Just verify Drop runs without panic.
        let kb = KeyBinding {
            bind: None,
            str: Some("send-string".to_string()),
            prefixct: 0,
        };
        freekeynode(kb);
    }
}

#[cfg(test)]
mod display_tests {
    use super::*;
    use crate::ported::zle::zle_main::Zle;
    use crate::ported::zsh_h::{ZLCON_LINE_START, ZLCON_LINE_CONT, ZLCON_SELECT, ZLCON_VARED};

    #[test]
    fn get_set_predisplay_round_trip() {
        // c:885,892 — round-trip set→get.
        set_predisplay(Some("[hint] "));
        assert_eq!(get_predisplay(), "[hint] ");
        set_predisplay(None);
        assert_eq!(get_predisplay(), "");
    }

    #[test]
    fn get_set_postdisplay_round_trip() {
        set_postdisplay(Some("trailer"));
        assert_eq!(get_postdisplay(), "trailer");
        set_postdisplay(None);
        assert_eq!(get_postdisplay(), "");
    }

    #[test]
    fn free_prepostdisplay_clears_both() {
        set_predisplay(Some("a"));
        set_postdisplay(Some("b"));
        free_prepostdisplay();
        assert_eq!(get_predisplay(), "");
        assert_eq!(get_postdisplay(), "");
    }

    #[test]
    fn get_context_branches() {
        use crate::ported::zle::zle_main::ZLECONTEXT;
        use std::sync::atomic::Ordering;
        let z = Zle::default();
        ZLECONTEXT.store(ZLCON_LINE_START, Ordering::SeqCst); assert_eq!(get_context(&z), "line");
        ZLECONTEXT.store(ZLCON_LINE_CONT,  Ordering::SeqCst); assert_eq!(get_context(&z), "cont");
        ZLECONTEXT.store(ZLCON_SELECT,     Ordering::SeqCst); assert_eq!(get_context(&z), "select");
        ZLECONTEXT.store(ZLCON_VARED,      Ordering::SeqCst); assert_eq!(get_context(&z), "vared");
    }

    #[test]
    fn get_lasearch_lsearch_default_empty() {
        // Globals default to empty Mutex<String>.
        // (Other tests may have set them, so we explicitly reset.)
        use crate::ported::zle::zle_misc::{PREVIOUS_ABORTED_SEARCH, PREVIOUS_SEARCH};
        use std::sync::Mutex;
        PREVIOUS_ABORTED_SEARCH.get_or_init(|| Mutex::new(String::new())).lock().unwrap().clear();
        PREVIOUS_SEARCH.get_or_init(|| Mutex::new(String::new())).lock().unwrap().clear();
        assert_eq!(get_lasearch(), "");
        assert_eq!(get_lsearch(), "");
    }

    #[test]
    fn get_prepost_truncates_to_len() {
        // c:881 — zlelineasstring(text, len, ...).
        assert_eq!(get_prepost("abcdef", 3), "abc");
        assert_eq!(get_prepost("xyz", 99), "xyz"); // len > content
    }

    #[test]
    fn set_prepost_writes_and_clears() {
        let mut text = String::new();
        let mut len = 0;
        set_prepost(&mut text, &mut len, Some("hello"));
        assert_eq!(text, "hello");
        assert_eq!(len, 5);
        set_prepost(&mut text, &mut len, None);
        assert_eq!(text, "");
        assert_eq!(len, 0);
    }
}

#[cfg(test)]
mod widget_killring_tests {
    use super::*;
    use crate::ported::zle::widget::{Widget, WidgetFlags, WidgetFunc};
    use crate::ported::zle::zle_main::Zle;
    use crate::ported::zle::zle_thingy::Thingy;
    use std::sync::Arc;

    fn thingy_with_user_widget(name: &str, fname: &str) -> Thingy {
        let mut t = Thingy::new(name);
        t.widget = Some(Arc::new(Widget {
            flags: WidgetFlags::empty(),
            func: WidgetFunc::User(fname.to_string()),
        }));
        t
    }

    #[test]
    fn get_widgetfunc_user_widget_returns_func_name() {
        let mut z = Zle::default();
        z.bindk = Some(thingy_with_user_widget("self-insert", "my-fn"));
        assert_eq!(get_widgetfunc(&z), "my-fn");
    }

    #[test]
    fn get_widgetfunc_internal_returns_dot_internal() {
        let mut z = Zle::default();
        let mut t = Thingy::new("forward-char");
        t.widget = Some(Arc::new(Widget {
            flags: WidgetFlags::INT,
            func: WidgetFunc::Internal(|_| {}),
        }));
        z.bindk = Some(t);
        assert_eq!(get_widgetfunc(&z), ".internal");
    }

    #[test]
    fn get_widgetstyle_internal_dot_internal() {
        let mut z = Zle::default();
        let mut t = Thingy::new("self-insert");
        t.widget = Some(Arc::new(Widget {
            flags: WidgetFlags::INT,
            func: WidgetFunc::Internal(|_| {}),
        }));
        z.bindk = Some(t);
        assert_eq!(get_widgetstyle(&z), ".internal");
    }

    #[test]
    fn set_get_register_round_trip() {
        let mut z = Zle::default();
        // Register 'a' (idx 0).
        set_register(&mut z, 'a', "hello");
        let s: String = z.vibuf[0].iter().collect();
        assert_eq!(s, "hello");
        // get_registers reads back the same.
        assert_eq!(get_registers(&z, "a"), Some("hello".to_string()));
    }

    #[test]
    fn set_register_digit_uses_offset_26() {
        let mut z = Zle::default();
        // Register '0' → idx 26.
        set_register(&mut z, '0', "zero");
        let s: String = z.vibuf[26].iter().collect();
        assert_eq!(s, "zero");
        assert_eq!(get_registers(&z, "0"), Some("zero".to_string()));
    }

    #[test]
    fn set_register_invalid_returns_one() {
        let mut z = Zle::default();
        assert_eq!(set_register(&mut z, '!', "x"), 1);
    }

    #[test]
    fn unset_register_clears_buffer() {
        let mut z = Zle::default();
        set_register(&mut z, 'a', "hi");
        unset_register(&mut z, 'a', 1);
        assert_eq!(get_registers(&z, "a"), Some(String::new()));
    }

    #[test]
    fn set_get_killring_round_trip() {
        let mut z = Zle::default();
        let entries = vec!["first".to_string(), "second".to_string()];
        set_killring(&mut z, Some(&entries));
        let got = get_killring(&z);
        assert_eq!(got, vec!["first".to_string(), "second".to_string()]);
    }

    #[test]
    fn unset_killring_clears_when_exp_nonzero() {
        let mut z = Zle::default();
        let entries = vec!["x".to_string()];
        set_killring(&mut z, Some(&entries));
        unset_killring(&mut z, 1);
        assert!(get_killring(&z).is_empty());
    }

    #[test]
    fn set_histno_clamps_to_entries_len() {
        let mut z = Zle::default();
        z.history.entries.push(crate::ported::zle::zle_hist::HistEntry {
            line: "ls".to_string(), num: 1, time: None,
        });
        z.history.entries.push(crate::ported::zle::zle_hist::HistEntry {
            line: "cd".to_string(), num: 2, time: None,
        });
        set_histno(&mut z, 1);
        assert_eq!(z.history.cursor, 1);
        // Beyond-end clamp: x > entries.len() → no change (early
        // return mirrors C's `quietgethist returns NULL → return`).
        z.history.cursor = 7;
        set_histno(&mut z, 99);
        assert_eq!(z.history.cursor, 7);
    }
}
