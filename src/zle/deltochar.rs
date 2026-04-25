//! ZLE delete-to-char / zap-to-char widgets
//!
//! Port from zsh/Src/Zle/deltochar.c (141 lines)
//!
//! Implements Emacs-style zap-to-char (M-z) and delete-to-char widgets.

/// Delete from cursor to next occurrence of character (from deltochar.c deltochar)
///
/// If `inclusive` is true (zap-to-char), the target character is also deleted.
/// If `inclusive` is false (delete-to-char), stop before the target character.
/// If `direction` is positive, search forward; negative, search backward.
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

/// Apply delete-to-char: returns the new buffer with the range removed
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
