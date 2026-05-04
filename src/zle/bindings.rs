//! ZLE key bindings
//!
//! Direct port from zsh/Src/Zle/zle_bindings.c

use super::keymap::KeymapManager;
use super::thingy::Thingy;

/// Initialize default key bindings
pub fn init_default_bindings(km: &mut KeymapManager) {
    // The default bindings are set up in KeymapManager::create_default_keymaps
    // This function is for additional runtime binding setup
    let _ = km;
}

/// Parse a key sequence string
/// Supports:
/// - ^X for control characters
/// - \e for escape
/// - \M- for meta (escape prefix)
/// - \C- for control
/// - Literal characters
pub fn parse_key_sequence(s: &str) -> Vec<u8> {
    let mut result = Vec::new();
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '^' => {
                // Control character
                if let Some(&next) = chars.peek() {
                    chars.next();
                    if next == '?' {
                        result.push(0x7f); // DEL
                    } else if next == '[' {
                        result.push(0x1b); // ESC
                    } else {
                        result.push((next.to_ascii_uppercase() as u8).wrapping_sub(b'@'));
                    }
                }
            }
            '\\' => {
                // Escape sequence
                match chars.peek() {
                    Some(&'e') | Some(&'E') => {
                        chars.next();
                        result.push(0x1b); // ESC
                    }
                    Some(&'n') => {
                        chars.next();
                        result.push(b'\n');
                    }
                    Some(&'t') => {
                        chars.next();
                        result.push(b'\t');
                    }
                    Some(&'r') => {
                        chars.next();
                        result.push(b'\r');
                    }
                    Some(&'M') => {
                        chars.next();
                        if chars.peek() == Some(&'-') {
                            chars.next();
                            // Meta prefix (escape + char)
                            result.push(0x1b);
                            if let Some(next) = chars.next() {
                                result.push(next as u8);
                            }
                        }
                    }
                    Some(&'C') => {
                        chars.next();
                        if chars.peek() == Some(&'-') {
                            chars.next();
                            // Control
                            if let Some(next) = chars.next() {
                                result.push((next.to_ascii_uppercase() as u8).wrapping_sub(b'@'));
                            }
                        }
                    }
                    Some(&'x') => {
                        chars.next();
                        // Hex escape
                        let mut hex = String::new();
                        for _ in 0..2 {
                            if let Some(&c) = chars.peek() {
                                if c.is_ascii_hexdigit() {
                                    hex.push(c);
                                    chars.next();
                                } else {
                                    break;
                                }
                            }
                        }
                        if let Ok(n) = u8::from_str_radix(&hex, 16) {
                            result.push(n);
                        }
                    }
                    Some(&c) => {
                        chars.next();
                        result.push(c as u8);
                    }
                    None => {
                        result.push(b'\\');
                    }
                }
            }
            _ => {
                result.push(c as u8);
            }
        }
    }

    result
}

/// Format a key sequence for display
pub fn format_key_sequence(seq: &[u8]) -> String {
    let mut result = String::new();
    let mut i = 0;

    while i < seq.len() {
        let b = seq[i];
        match b {
            0x1b => {
                // Escape — render as `^[` regardless of whether more
                // bytes follow; downstream caller emits the rest.
                result.push_str("^[");
            }
            0x00..=0x1f => {
                // Control character
                result.push('^');
                result.push((b + b'@') as char);
            }
            0x7f => {
                result.push_str("^?");
            }
            0x80..=0xff => {
                // High byte
                result.push_str(&format!("\\x{:02x}", b));
            }
            _ => {
                result.push(b as char);
            }
        }
        i += 1;
    }

    result
}

/// Bind a key sequence in a named keymap (port of bindkey from
/// Src/Zle/zle_keymap.c). Returns true if the keymap exists and the binding
/// is installed. Uses `Arc::make_mut` to copy-on-write the wrapped Keymap so
/// the mutation respects the existing Arc-shared layout.
pub fn bind_key(km: &mut KeymapManager, keymap: &str, seq: &str, widget: &str) -> bool {
    let seq_bytes = parse_key_sequence(seq);
    let map = match km.keymaps.get_mut(keymap) {
        Some(m) => m,
        None => return false,
    };
    let inner = std::sync::Arc::make_mut(map);
    inner.bind_seq(&seq_bytes, Thingy::new(widget));
    true
}

/// Remove a binding in a named keymap (port of `bindkey -r` from zle_keymap.c).
pub fn unbind_key(km: &mut KeymapManager, keymap: &str, seq: &str) -> bool {
    let seq_bytes = parse_key_sequence(seq);
    let map = match km.keymaps.get_mut(keymap) {
        Some(m) => m,
        None => return false,
    };
    let inner = std::sync::Arc::make_mut(map);
    inner.unbind_seq(&seq_bytes);
    true
}

/// List bindings in a keymap
pub fn list_bindings(km: &KeymapManager, keymap: &str) -> Vec<(String, String)> {
    let mut bindings = Vec::new();

    if let Some(map) = km.keymaps.get(keymap) {
        // Single character bindings
        for (i, thingy) in map.first.iter().enumerate() {
            if let Some(t) = thingy {
                let seq = format_key_sequence(&[i as u8]);
                bindings.push((seq, t.name.clone()));
            }
        }

        // Multi-character bindings
        for (seq, binding) in &map.multi {
            if let Some(t) = &binding.bind {
                let seq_str = format_key_sequence(seq);
                bindings.push((seq_str, t.name.clone()));
            } else if let Some(s) = &binding.str {
                let seq_str = format_key_sequence(seq);
                bindings.push((seq_str, format!("send-string \"{}\"", s)));
            }
        }
    }

    bindings.sort_by(|a, b| a.0.cmp(&b.0));
    bindings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_key_returns_false_for_unknown_keymap() {
        let mut km = KeymapManager::new();
        assert!(!bind_key(&mut km, "no-such-keymap", "^A", "self-insert"));
    }

    #[test]
    fn bind_key_then_unbind_round_trips_through_emacs_keymap() {
        let mut km = KeymapManager::new();
        // Pick a sequence unlikely to clash with the default emacs map.
        // \M-z = ESC z = bytes 0x1B 0x7A.
        assert!(bind_key(&mut km, "emacs", "\\ez", "self-insert"));
        // Verify the binding shows up in list_bindings.
        let listed = list_bindings(&km, "emacs");
        let seq = format_key_sequence(&[0x1b, 0x7a]);
        assert!(
            listed.iter().any(|(k, v)| k == &seq && v == "self-insert"),
            "bound sequence missing from list: {:?}",
            listed
        );
        // Now remove it.
        assert!(unbind_key(&mut km, "emacs", "\\ez"));
        let listed = list_bindings(&km, "emacs");
        assert!(
            !listed.iter().any(|(k, _)| k == &seq),
            "unbound sequence still present: {:?}",
            listed
        );
    }
}
