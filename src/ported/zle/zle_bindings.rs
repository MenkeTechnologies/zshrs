//! ZLE key bindings
//!
//! Direct port from zsh/Src/Zle/zle_bindings.c

use super::zle_thingy::Thingy;

/// Parse a bindkey-style key sequence string into raw bytes.
///
/// Bindkey-vocabulary subset of `getkeystring` (Src/utils.c) —
/// zsh uses the same parser for `bindkey 'seq' widget`, restricted
/// to the key-sequence vocabulary documented at `man zshzle`
/// BINDKEY:
///   - `^X` → control character (X & 0x1F)
///   - `\\e` → ESC (0x1B)
///   - `\\M-X` → ESC + X (zsh's meta encoding for the keymap-trie)
///   - `\\C-X` → control character
///   - everything else → literal byte
///
/// Port of `getkeystring(char *s, int *len, int how, int *misc)` from Src/utils.c:6915 — `bindkey` calls
/// it at line 4111 with `GETKEYS_BINDKEY` to convert the user-typed
/// key spec into the raw byte sequence the keymap trie indexes by.
/// The C version mutates a buffer in place + writes length via out
/// pointer; this Rust port returns a fresh `Vec<u8>`.
/// WARNING: param names don't match C — Rust=(s) vs C=(s, len, how, misc)

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
#[allow(unused_imports)]
use crate::ported::zle::zle_main::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_misc::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_hist::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_move::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_word::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_params::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_vi::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_utils::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_refresh::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_tricky::*;
#[allow(unused_imports)]
use crate::ported::zle::textobjects::*;
#[allow(unused_imports)]
use crate::ported::zle::deltochar::*;

pub fn getkeystring(s: &str) -> Vec<u8> {                                    // c:utils.c:6915
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

/// Format a raw key-sequence byte slice for human-readable display.
/// Port of `printbind(char *str, FILE *stream)` from Src/Zle/zle_utils.c:1283 — used by
/// `bindkey -L` and the `where-is` widget to show key bindings in the
/// same `^X` / `\\eX` form `getkeystring` accepts. C signature writes
/// to a `FILE*` stream; this Rust port returns the formatted `String`.
/// WARNING: param names don't match C — Rust=(seq) vs C=(str, stream)
pub fn printbind(seq: &[u8]) -> String {                                     // c:zle_utils.c:1283
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
pub fn bindkey(keymap: &str, seq: &str, widget: &str) -> bool { // c:zle_keymap.c:566
    let seq_bytes = getkeystring(seq);
    let mut tab = crate::ported::zle::zle_keymap::keymapnamtab().lock().unwrap();
    let node = match tab.get_mut(keymap) {
        Some(n) => n,
        None => return false,
    };
    let inner = std::sync::Arc::make_mut(&mut node.keymap);
    inner.bind_seq(&seq_bytes, Thingy::new(widget));
    true
}

/// Enumerate every (key-sequence, widget-name) pair in `keymap`.
/// Port of `bindkey -L` listing from Src/Zle/zle_keymap.c (the
/// listing branch of `bin_bindkey`). Both 1-byte fast-path entries
/// (`first[]`) and multi-byte trie entries (`multi`) are included.
pub fn bindlistout(keymap: &str) -> Vec<(String, String)> { // c:zle_keymap.c:1094
    let mut bindings = Vec::new();

    if let Some(map) = crate::ported::zle::zle_keymap::openkeymap(keymap) {
        // Single character bindings
        for (i, thingy) in map.first.iter().enumerate() {
            if let Some(t) = thingy {
                let seq = printbind(&[i as u8]);
                bindings.push((seq, t.nam.clone()));
            }
        }

        // Multi-character bindings
        for (seq, binding) in &map.multi {
            if let Some(t) = &binding.bind {
                let seq_str = printbind(seq);
                bindings.push((seq_str, t.nam.clone()));
            } else if let Some(s) = &binding.str {
                let seq_str = printbind(seq);
                bindings.push((seq_str, format!("send-string \"{}\"", s)));
            }
        }
    }

    bindings.sort_by(|a, b| a.0.cmp(&b.0));
    bindings
}

// =====================================================================
// Default key binding tables — direct port of
// `Src/Zle/zle_bindings.c:88-421`. Each table maps a (control-)byte to
// the canonical widget name. The C source uses generated `z_*` enum
// indices into `thingies[]`; the Rust port uses widget-name strings
// resolved via `Thingy::builtin(name)` (functionally equivalent — the
// C indices are a build-time optimisation).
// =====================================================================

/// `int emacsbind[32]` — `Src/Zle/zle_bindings.c:88-121`. Maps control
/// chars (`^@`..`^_`) for the emacs keymap.
pub static EMACSBIND: [&str; 32] = [
    /* ^@ */ "set-mark-command",
    /* ^A */ "beginning-of-line",
    /* ^B */ "backward-char",
    /* ^C */ "undefined-key",
    /* ^D */ "delete-char-or-list",
    /* ^E */ "end-of-line",
    /* ^F */ "forward-char",
    /* ^G */ "send-break",
    /* ^H */ "backward-delete-char",
    /* ^I */ "expand-or-complete",
    /* ^J */ "accept-line",
    /* ^K */ "kill-line",
    /* ^L */ "clear-screen",
    /* ^M */ "accept-line",
    /* ^N */ "down-line-or-history",
    /* ^O */ "accept-line-and-down-history",
    /* ^P */ "up-line-or-history",
    /* ^Q */ "push-line",
    /* ^R */ "history-incremental-search-backward",
    /* ^S */ "history-incremental-search-forward",
    /* ^T */ "transpose-chars",
    /* ^U */ "kill-whole-line",
    /* ^V */ "quoted-insert",
    /* ^W */ "backward-kill-word",
    /* ^X */ "undefined-key",
    /* ^Y */ "yank",
    /* ^Z */ "undefined-key",
    /* ^[ */ "undefined-key",
    /* ^\ */ "undefined-key",
    /* ^] */ "undefined-key",
    /* ^^ */ "undefined-key",
    /* ^_ */ "undo",
];

/// `int metabind[128]` — `Src/Zle/zle_bindings.c:123-253`. Maps the
/// post-ESC byte for emacs-mode meta sequences.
pub static METABIND: [&str; 128] = [
    /* M-^@ */ "undefined-key",
    /* M-^A */ "undefined-key",
    /* M-^B */ "undefined-key",
    /* M-^C */ "undefined-key",
    /* M-^D */ "list-choices",
    /* M-^E */ "undefined-key",
    /* M-^F */ "undefined-key",
    /* M-^G */ "send-break",
    /* M-^H */ "backward-kill-word",
    /* M-^I */ "self-insert-unmeta",
    /* M-^J */ "self-insert-unmeta",
    /* M-^K */ "undefined-key",
    /* M-^L */ "clear-screen",
    /* M-^M */ "self-insert-unmeta",
    /* M-^N */ "undefined-key",
    /* M-^O */ "undefined-key",
    /* M-^P */ "undefined-key",
    /* M-^Q */ "undefined-key",
    /* M-^R */ "undefined-key",
    /* M-^S */ "undefined-key",
    /* M-^T */ "undefined-key",
    /* M-^U */ "undefined-key",
    /* M-^V */ "undefined-key",
    /* M-^W */ "undefined-key",
    /* M-^X */ "undefined-key",
    /* M-^Y */ "undefined-key",
    /* M-^Z */ "undefined-key",
    /* M-^[ */ "undefined-key",
    /* M-^\ */ "undefined-key",
    /* M-^] */ "undefined-key",
    /* M-^^ */ "undefined-key",
    /* M-^_ */ "copy-prev-word",
    /* M-  */ "expand-history",
    /* M-! */ "expand-history",
    /* M-" */ "quote-region",
    /* M-# */ "undefined-key",
    /* M-$ */ "spell-word",
    /* M-% */ "undefined-key",
    /* M-& */ "undefined-key",
    /* M-' */ "quote-line",
    /* M-( */ "undefined-key",
    /* M-) */ "undefined-key",
    /* M-* */ "undefined-key",
    /* M-+ */ "undefined-key",
    /* M-, */ "undefined-key",
    /* M-- */ "neg-argument",
    /* M-. */ "insert-last-word",
    /* M-/ */ "undefined-key",
    /* M-0 */ "digit-argument",
    /* M-1 */ "digit-argument",
    /* M-2 */ "digit-argument",
    /* M-3 */ "digit-argument",
    /* M-4 */ "digit-argument",
    /* M-5 */ "digit-argument",
    /* M-6 */ "digit-argument",
    /* M-7 */ "digit-argument",
    /* M-8 */ "digit-argument",
    /* M-9 */ "digit-argument",
    /* M-: */ "undefined-key",
    /* M-; */ "undefined-key",
    /* M-< */ "beginning-of-buffer-or-history",
    /* M-= */ "undefined-key",
    /* M-> */ "end-of-buffer-or-history",
    /* M-? */ "which-command",
    /* M-@ */ "undefined-key",
    /* M-A */ "accept-and-hold",
    /* M-B */ "backward-word",
    /* M-C */ "capitalize-word",
    /* M-D */ "kill-word",
    /* M-E */ "undefined-key",
    /* M-F */ "forward-word",
    /* M-G */ "get-line",
    /* M-H */ "run-help",
    /* M-I */ "undefined-key",
    /* M-J */ "undefined-key",
    /* M-K */ "undefined-key",
    /* M-L */ "down-case-word",
    /* M-M */ "undefined-key",
    /* M-N */ "history-search-forward",
    /* M-O */ "undefined-key",
    /* M-P */ "history-search-backward",
    /* M-Q */ "push-line",
    /* M-R */ "undefined-key",
    /* M-S */ "spell-word",
    /* M-T */ "transpose-words",
    /* M-U */ "up-case-word",
    /* M-V */ "undefined-key",
    /* M-W */ "copy-region-as-kill",
    /* M-X */ "undefined-key",
    /* M-Y */ "undefined-key",
    /* M-Z */ "undefined-key",
    /* M-[ */ "undefined-key",
    /* M-\ */ "undefined-key",
    /* M-] */ "undefined-key",
    /* M-^ */ "undefined-key",
    /* M-_ */ "insert-last-word",
    /* M-` */ "undefined-key",
    /* M-a */ "accept-and-hold",
    /* M-b */ "backward-word",
    /* M-c */ "capitalize-word",
    /* M-d */ "kill-word",
    /* M-e */ "undefined-key",
    /* M-f */ "forward-word",
    /* M-g */ "get-line",
    /* M-h */ "run-help",
    /* M-i */ "undefined-key",
    /* M-j */ "undefined-key",
    /* M-k */ "undefined-key",
    /* M-l */ "down-case-word",
    /* M-m */ "undefined-key",
    /* M-n */ "history-search-forward",
    /* M-o */ "undefined-key",
    /* M-p */ "history-search-backward",
    /* M-q */ "push-line",
    /* M-r */ "undefined-key",
    /* M-s */ "spell-word",
    /* M-t */ "transpose-words",
    /* M-u */ "up-case-word",
    /* M-v */ "undefined-key",
    /* M-w */ "copy-region-as-kill",
    /* M-x */ "execute-named-cmd",
    /* M-y */ "yank-pop",
    /* M-z */ "execute-last-named-cmd",
    /* M-{ */ "undefined-key",
    /* M-| */ "vi-goto-column",
    /* M-} */ "undefined-key",
    /* M-~ */ "undefined-key",
    /* M-^? */ "backward-kill-word",
];

/// `int viinsbind[32]` — `Src/Zle/zle_bindings.c:256-289`. Maps
/// control chars for vi insert mode.
pub static VIINSBIND: [&str; 32] = [
    /* ^@ */ "undefined-key",
    /* ^A */ "self-insert",
    /* ^B */ "self-insert",
    /* ^C */ "self-insert",
    /* ^D */ "list-choices",
    /* ^E */ "self-insert",
    /* ^F */ "self-insert",
    /* ^G */ "list-expand",
    /* ^H */ "vi-backward-delete-char",
    /* ^I */ "expand-or-complete",
    /* ^J */ "accept-line",
    /* ^K */ "self-insert",
    /* ^L */ "clear-screen",
    /* ^M */ "accept-line",
    /* ^N */ "self-insert",
    /* ^O */ "self-insert",
    /* ^P */ "self-insert",
    /* ^Q */ "vi-quoted-insert",
    /* ^R */ "redisplay",
    /* ^S */ "self-insert",
    /* ^T */ "self-insert",
    /* ^U */ "vi-kill-line",
    /* ^V */ "vi-quoted-insert",
    /* ^W */ "vi-backward-kill-word",
    /* ^X */ "undefined-key",
    /* ^Y */ "self-insert",
    /* ^Z */ "self-insert",
    /* ^[ */ "vi-cmd-mode",
    /* ^\ */ "self-insert",
    /* ^] */ "self-insert",
    /* ^^ */ "self-insert",
    /* ^_ */ "self-insert",
];

/// `int vicmdbind[128]` — `Src/Zle/zle_bindings.c:292-421`. Maps all
/// chars 0-127 for vi command mode.
pub static VICMDBIND: [&str; 128] = [
    /* ^@ */ "undefined-key",
    /* ^A */ "undefined-key",
    /* ^B */ "undefined-key",
    /* ^C */ "undefined-key",
    /* ^D */ "list-choices",
    /* ^E */ "undefined-key",
    /* ^F */ "undefined-key",
    /* ^G */ "list-expand",
    /* ^H */ "vi-backward-char",
    /* ^I */ "undefined-key",
    /* ^J */ "accept-line",
    /* ^K */ "undefined-key",
    /* ^L */ "clear-screen",
    /* ^M */ "accept-line",
    /* ^N */ "down-history",
    /* ^O */ "undefined-key",
    /* ^P */ "up-history",
    /* ^Q */ "undefined-key",
    /* ^R */ "redo",
    /* ^S */ "undefined-key",
    /* ^T */ "undefined-key",
    /* ^U */ "undefined-key",
    /* ^V */ "undefined-key",
    /* ^W */ "undefined-key",
    /* ^X */ "undefined-key",
    /* ^Y */ "undefined-key",
    /* ^Z */ "undefined-key",
    /* ^[ */ "beep",
    /* ^\ */ "undefined-key",
    /* ^] */ "undefined-key",
    /* ^^ */ "undefined-key",
    /* ^_ */ "undefined-key",
    /*   */ "vi-forward-char",
    /* ! */ "undefined-key",
    /* " */ "vi-set-buffer",
    /* # */ "pound-insert",
    /* $ */ "vi-end-of-line",
    /* % */ "vi-match-bracket",
    /* & */ "undefined-key",
    /* ' */ "vi-goto-mark-line",
    /* ( */ "undefined-key",
    /* ) */ "undefined-key",
    /* * */ "undefined-key",
    /* + */ "vi-down-line-or-history",
    /* , */ "vi-rev-repeat-find",
    /* - */ "vi-up-line-or-history",
    /* . */ "vi-repeat-change",
    /* / */ "vi-history-search-backward",
    /* 0 */ "vi-digit-or-beginning-of-line",
    /* 1 */ "digit-argument",
    /* 2 */ "digit-argument",
    /* 3 */ "digit-argument",
    /* 4 */ "digit-argument",
    /* 5 */ "digit-argument",
    /* 6 */ "digit-argument",
    /* 7 */ "digit-argument",
    /* 8 */ "digit-argument",
    /* 9 */ "digit-argument",
    /* : */ "execute-named-cmd",
    /* ; */ "vi-repeat-find",
    /* < */ "vi-unindent",
    /* = */ "list-choices",
    /* > */ "vi-indent",
    /* ? */ "vi-history-search-forward",
    /* @ */ "undefined-key",
    /* A */ "vi-add-eol",
    /* B */ "vi-backward-blank-word",
    /* C */ "vi-change-eol",
    /* D */ "vi-kill-eol",
    /* E */ "vi-forward-blank-word-end",
    /* F */ "vi-find-prev-char",
    /* G */ "vi-fetch-history",
    /* H */ "undefined-key",
    /* I */ "vi-insert-bol",
    /* J */ "vi-join",
    /* K */ "undefined-key",
    /* L */ "undefined-key",
    /* M */ "undefined-key",
    /* N */ "vi-rev-repeat-search",
    /* O */ "vi-open-line-above",
    /* P */ "vi-put-before",
    /* Q */ "undefined-key",
    /* R */ "vi-replace",
    /* S */ "vi-change-whole-line",
    /* T */ "vi-find-prev-char-skip",
    /* U */ "undefined-key",
    /* V */ "visual-line-mode",
    /* W */ "vi-forward-blank-word",
    /* X */ "vi-backward-delete-char",
    /* Y */ "vi-yank-whole-line",
    /* Z */ "undefined-key",
    /* [ */ "undefined-key",
    /* \ */ "undefined-key",
    /* ] */ "undefined-key",
    /* ^ */ "vi-first-non-blank",
    /* _ */ "vi-first-non-blank",
    /* ` */ "vi-goto-mark",
    /* a */ "vi-add-next",
    /* b */ "vi-backward-word",
    /* c */ "vi-change",
    /* d */ "vi-delete",
    /* e */ "vi-forward-word-end",
    /* f */ "vi-find-next-char",
    /* g */ "undefined-key",
    /* h */ "vi-backward-char",
    /* i */ "vi-insert",
    /* j */ "down-line-or-history",
    /* k */ "up-line-or-history",
    /* l */ "vi-forward-char",
    /* m */ "vi-set-mark",
    /* n */ "vi-repeat-search",
    /* o */ "vi-open-line-below",
    /* p */ "vi-put-after",
    /* q */ "undefined-key",
    /* r */ "vi-replace-chars",
    /* s */ "vi-substitute",
    /* t */ "vi-find-next-char-skip",
    /* u */ "undo",
    /* v */ "visual-mode",
    /* w */ "vi-forward-word",
    /* x */ "vi-delete-char",
    /* y */ "vi-yank",
    /* z */ "undefined-key",
    /* { */ "undefined-key",
    /* | */ "vi-goto-column",
    /* } */ "undefined-key",
    /* ~ */ "vi-swap-case",
    /* ^? */ "vi-backward-char",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindkey_returns_false_for_unknown_keymap() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        crate::ported::zle::zle_keymap::createkeymapnamtab();
        crate::ported::zle::zle_keymap::default_bindings();
        assert!(!bindkey("no-such-keymap", "^A", "self-insert"));
    }

    #[test]
    fn bindkey_then_unbind_round_trips_through_emacs_keymap() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        crate::ported::zle::zle_keymap::createkeymapnamtab();
        crate::ported::zle::zle_keymap::default_bindings();
        // Pick a sequence unlikely to clash with the default emacs map.
        // \M-z = ESC z = bytes 0x1B 0x7A.
        assert!(bindkey("emacs", "\\ez", "self-insert"));
        // Verify the binding shows up in bindlistout.
        let listed = bindlistout("emacs");
        let seq = printbind(&[0x1b, 0x7a]);
        assert!(
            listed.iter().any(|(k, v)| k == &seq && v == "self-insert"),
            "bound sequence missing from list: {:?}",
            listed
        );
        // Now remove it (inline of the deleted unbindkey helper).
        let seq_bytes = getkeystring("\\ez");
        let mut tab = crate::ported::zle::zle_keymap::keymapnamtab().lock().unwrap();
        let node = tab.get_mut("emacs").unwrap();
        let inner = std::sync::Arc::make_mut(&mut node.keymap);
        inner.unbind_seq(&seq_bytes);
        drop(tab);
        let listed = bindlistout("emacs");
        assert!(
            !listed.iter().any(|(k, _)| k == &seq),
            "unbound sequence still present: {:?}",
            listed
        );
    }
}
