//! ZLE delete-to-char / zap-to-char widgets
//!
//! Port from zsh/Src/Zle/deltochar.c (141 lines)
//!
//! Implements Emacs-style zap-to-char (M-z) and delete-to-char widgets.

/// Compute the (start, end) range to delete from cursor toward the
/// next occurrence of `target`.
///
/// Port of `deltochar()` from Src/Zle/deltochar.c. `inclusive=true`
/// matches zsh's `zaptochar` (M-z) which removes the target char too;
/// `inclusive=false` is `delete-to-char` which stops just before. The
/// `direction` arg corresponds to zsh's `zmult` sign.
pub fn deltochar(
    buffer: &str,
    cursor: usize,
    target: char,
    direction: i32,
    inclusive: bool,
) -> Option<(usize, usize)> {
    if direction >= 0 {
        // Search forward
        let search_area = &buffer[cursor..];
        if let Some(pos) = search_area.find(target) {
            let end = cursor + pos + if inclusive { target.len_utf8() } else { 0 };
            Some((cursor, end))
        } else {
            None
        }
    } else {
        // Search backward
        let search_area = &buffer[..cursor];
        if let Some(pos) = search_area.rfind(target) {
            let start = if inclusive {
                pos
            } else {
                pos + target.len_utf8()
            };
            Some((start, cursor))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deltochar_forward() {
        // "hello world" with cursor at 0, delete to 'o' (inclusive)
        let (start, end) = deltochar("hello world", 0, 'o', 1, true).unwrap();
        assert_eq!(start, 0);
        assert_eq!(end, 5); // includes 'o'
    }

    #[test]
    fn test_deltochar_forward_exclusive() {
        let (start, end) = deltochar("hello world", 0, 'o', 1, false).unwrap();
        assert_eq!(start, 0);
        assert_eq!(end, 4); // stops before 'o'
    }

    #[test]
    fn test_deltochar_backward() {
        let (start, end) = deltochar("hello world", 11, 'o', -1, true).unwrap();
        assert_eq!(start, 7); // includes 'o'
        assert_eq!(end, 11);
    }

    #[test]
    fn test_deltochar_not_found() {
        assert!(deltochar("hello", 0, 'z', 1, true).is_none());
    }

    #[test]
    fn test_apply_deltochar() {
        let buffer = "hello world";
        let (start, end) = deltochar(buffer, 0, 'o', 1, true).unwrap();
        let mut result = String::with_capacity(buffer.len());
        result.push_str(&buffer[..start]);
        result.push_str(&buffer[end..]);
        assert_eq!(result, " world");
        assert_eq!(start, 0);
    }
}

/// Port of `setup_()` from `Src/Zle/deltochar.c:90`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn setup_() -> i32 {                                                 // c:90
    0                                                                    // c:93
}

/// Port of `features_()` from `Src/Zle/deltochar.c:97`. C body is
/// `*features = featuresarray(m, &module_features); return 0;` —
/// returns the per-module feature table. zshrs links statically so
/// the table is build-time-fixed; this hook returns 0 to mirror C.
pub fn features_() -> i32 {                                              // c:97
    0                                                                    // c:101
}

/// Port of `enables_()` from `Src/Zle/deltochar.c:105`. C body is
/// `return handlefeatures(m, &module_features, enables);` — wires
/// the per-module enable bits to the loader. zshrs's static link
/// returns 0 (success, no enables changed).
pub fn enables_() -> i32 {                                               // c:105
    0                                                                    // c:108
}

/// Port of `boot_()` from `Src/Zle/deltochar.c:112`. C registers
/// the `delete-to-char` and `zap-to-char` ZLE functions via
/// `addzlefunction(...)` with `ZLE_KILL | ZLE_KEEPSUFFIX`. zshrs
/// registers ZLE widgets via `extensions/widget.rs` at build time,
/// so this is a no-op port — both widgets are already wired.
pub fn boot_() -> i32 {                                                  // c:112
    // C creates `w_deletetochar` and `w_zaptochar` thingies and
    // wires the `deltochar` C handler. zshrs widget dispatch lives
    // in `extensions/widget.rs` (`widget_delete_to_char`,
    // `widget_zap_to_char`); registration happens at startup, not
    // at module-boot time. Match C's success exit.
    0                                                                    // c:124
}

/// Port of `cleanup_()` from `Src/Zle/deltochar.c:129`. C calls
/// `deletezlefunction()` for both registered widgets and then
/// `setfeatureenables(...)`. zshrs widgets are static-linked and
/// never deleted, so this is a no-op.
pub fn cleanup_() -> i32 {                                               // c:129
    0                                                                    // c:135
}

/// Port of `finish_()` from `Src/Zle/deltochar.c:138`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn finish_() -> i32 {                                                // c:138
    0                                                                    // c:141
}
