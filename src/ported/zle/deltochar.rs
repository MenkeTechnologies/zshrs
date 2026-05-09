//! ZLE delete-to-char / zap-to-char widgets — port of
//! `Src/Zle/deltochar.c` (141 lines).
//!
//! C source has zero `struct ...` / `enum ...` definitions. The
//! Rust port matches: zero types. The two static `Widget`
//! variables (`w_deletetochar`, `w_zaptochar`) are local to C's
//! module-loader path and don't translate — zshrs wires the two
//! widgets through `extensions/widget.rs` at build time.

use crate::ported::zle::zle_main::Zle;

/// Port of `deltochar()` from `Src/Zle/deltochar.c:38`. The shared
/// body behind both the `delete-to-char` and `zap-to-char` widgets:
/// reads the next char from input, then for each repeat of `zmult`
/// scans the buffer toward that target and kills the spanned text
/// (forward when `zmult >= 0`, backward when `zmult < 0`). The
/// "zap" mode (binding name `zap-to-char`) includes the target
/// char itself in the killed range.
///
/// C signature: `static int deltochar(char **args)` (args UNUSED).
/// Returns 0 on success (target found, kill performed), 1 on
/// failure (target not in buffer).
pub fn deltochar(zle: &mut Zle) -> i32 {                                 // c:38
    let c = match zle.getfullchar(false) {                               // c:41 getfullchar(0)
        Some(ch) => ch,
        None => return 1,
    };
    let mut dest = zle.zlecs;                                            // c:42
    let mut ok = 0i32;                                                   // c:42
    let mut n: i32 = if zle.zmod.flags.contains(crate::ported::zle::zle_main::ModifierFlags::MULT) {
        zle.zmod.mult
    } else {
        1
    };                                                                   // c:42 zmult
    let zap = zle.bindk.as_ref().map(|t| t.name.as_str()) == Some("zap-to-char"); // c:43

    if n > 0 {                                                           // c:45
        while n > 0 && dest != zle.zlell {                               // c:46 while (n-- && dest != zlell)
            n -= 1;
            while dest != zle.zlell && zle.zleline.get(dest).copied() != Some(c) {
                dest += 1;                                               // c:48 INCPOS(dest)
            }
            if dest != zle.zlell {                                       // c:50
                if !zap || n > 0 {                                       // c:51
                    dest += 1;                                           // c:52 INCPOS(dest)
                }
                if n == 0 {                                              // c:53
                    let ct = (dest as i32) - (zle.zlecs as i32);         // c:54 dest - zlecs
                    crate::ported::zle::zle_utils::forekill(zle, ct, 0); // c:54 CUT_RAW
                    ok += 1;                                             // c:55
                }
            }
        }
    } else {                                                             // c:59
        if dest > 0 {                                                    // c:61
            dest -= 1;                                                   // c:62 DECPOS(dest)
        }
        while n < 0 && dest != 0 {                                       // c:63 while (n++ && dest != 0)
            n += 1;
            while dest != 0 && zle.zleline.get(dest).copied() != Some(c) {
                dest -= 1;                                               // c:65 DECPOS(dest)
            }
            if zle.zleline.get(dest).copied() == Some(c) {               // c:67
                if n == 0 {                                              // c:68
                    let zap_adj = if zap { 1 } else { 0 };               // c:70 trailing combining-char adjust
                    let ct = (zle.zlecs as i32) - (dest as i32) - zap_adj;
                    crate::ported::zle::zle_utils::backkill(zle, ct, 0); // c:70 CUT_RAW|CUT_FRONT
                    ok += 1;                                             // c:71
                }
                if dest > 0 {                                            // c:73
                    dest -= 1;                                           // c:74 DECPOS(dest)
                }
            }
        }
    }
    if ok != 0 { 0 } else { 1 }                                          // c:79 return !ok
}

/// Port of `setup_()` from `Src/Zle/deltochar.c:90`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn setup_() -> i32 {                                                 // c:90
    0                                                                    // c:93
}

/// Port of `features_()` from `Src/Zle/deltochar.c:97`. C body is
/// `*features = featuresarray(m, &module_features); return 0;`.
/// Static-link path: 0.
pub fn features_() -> i32 {                                              // c:97
    0                                                                    // c:101
}

/// Port of `enables_()` from `Src/Zle/deltochar.c:105`. C body is
/// `return handlefeatures(m, &module_features, enables);`.
pub fn enables_() -> i32 {                                               // c:105
    0                                                                    // c:108
}

/// Port of `boot_()` from `Src/Zle/deltochar.c:112`. C registers
/// the `delete-to-char` and `zap-to-char` ZLE functions via
/// `addzlefunction(name, deltochar, ZLE_KILL | ZLE_KEEPSUFFIX)`.
/// zshrs wires both widgets through `extensions/widget.rs` at
/// build time, so this is a no-op port.
pub fn boot_() -> i32 {                                                  // c:112
    0                                                                    // c:124
}

/// Port of `cleanup_()` from `Src/Zle/deltochar.c:129`. C calls
/// `deletezlefunction()` for both widgets and then
/// `setfeatureenables(...)`. zshrs widgets are static-linked and
/// never deleted; no-op port.
pub fn cleanup_() -> i32 {                                               // c:129
    0                                                                    // c:135
}

/// Port of `finish_()` from `Src/Zle/deltochar.c:138`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn finish_() -> i32 {                                                // c:138
    0                                                                    // c:141
}
