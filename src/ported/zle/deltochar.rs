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

/// Apply the deltochar range to a buffer, returning the trimmed copy
/// and the cursor position.
///
/// Convenience wrapper around `deltochar` that mirrors the
/// drain-the-range step the C source does inline at the end of
/// `deltochar()` in Src/Zle/deltochar.c — kept separate here so
/// callers can inspect the range first.
pub fn apply_deltochar(
    buffer: &str,
    cursor: usize,
    target: char,
    direction: i32,
    inclusive: bool,
) -> Option<(String, usize)> {
    let (start, end) = deltochar(buffer, cursor, target, direction, inclusive)?;
    let mut result = String::with_capacity(buffer.len());
    result.push_str(&buffer[..start]);
    result.push_str(&buffer[end..]);
    Some((result, start))
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
        let (result, cursor) = apply_deltochar("hello world", 0, 'o', 1, true).unwrap();
        assert_eq!(result, " world");
        assert_eq!(cursor, 0);
    }
}

/// Port of `boot_()` from Src/Zle/deltochar.c:112. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn boot_() -> i32 { 0 }

/// Port of `cleanup_()` from Src/Zle/deltochar.c:129. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn cleanup_() -> i32 { 0 }

/// Port of `enables_()` from Src/Zle/deltochar.c:105. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn enables_() -> i32 { 0 }

/// Port of `features_()` from Src/Zle/deltochar.c:97. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn features_() -> i32 { 0 }

/// Port of `finish_()` from Src/Zle/deltochar.c:138. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn finish_() -> i32 { 0 }

/// Port of `setup_()` from Src/Zle/deltochar.c:90. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn setup_() -> i32 { 0 }
