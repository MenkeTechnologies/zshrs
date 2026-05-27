//! ZLE key bindings
//!
//! Direct port from zsh/Src/Zle/zle_bindings.c

use super::zle_keymap::KM_IMMUTABLE;
use super::zle_thingy::Thingy;

#[allow(unused_imports)]
use crate::ported::zle::{
    deltochar::*, textobjects::*, zle_hist::*, zle_main::*, zle_misc::*, zle_move::*,
    zle_params::*, zle_refresh::*, zle_tricky::*, zle_utils::*, zle_vi::*, zle_word::*,
};
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

pub fn getkeystring(s: &str) -> Vec<u8> {
    // c:utils.c:6915
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
                // Escape sequence — must mirror C's
                // `Src/utils.c:6993-7017` switch:
                //   \a → 0x07 (BEL)         c:6995
                //   \b → 0x08 (BS)          c:7003
                //   \e / \E → 0x1b (ESC)    c:7019/7026
                //   \f → 0x0c (FF)          c:7013
                //   \n → 0x0a (LF)          c:7000
                //   \r → 0x0d (CR)          c:7016
                //   \t → 0x09 (TAB)         c:7006
                //   \v → 0x0b (VT)          c:7009
                // The previous Rust port only handled e/E/n/t/r,
                // silently dropping \a/\b/\f/\v through the
                // generic Some(&c) arm — `getkeystring("\\b")`
                // returned ['b'] (0x62) instead of [0x08]. Fixed
                // 2026-05 to match the C switch verbatim.
                match chars.peek() {
                    Some(&'a') => {
                        chars.next();
                        result.push(0x07); // BEL — c:6995
                    }
                    Some(&'b') => {
                        chars.next();
                        result.push(0x08); // BS — c:7003
                    }
                    Some(&'e') | Some(&'E') => {
                        chars.next();
                        result.push(0x1b); // ESC
                    }
                    Some(&'f') => {
                        chars.next();
                        result.push(0x0c); // FF — c:7013
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
                    Some(&'v') => {
                        chars.next();
                        result.push(0x0b); // VT — c:7009
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

// `printbind` lives in zle_utils.rs (matching C: `Src/Zle/
// zle_utils.c:1283`). The duplicate that used to live here was a
// fake — its callers (zle_main.rs) already routed through
// `super::zle_utils::printbind`. Removed to drop the stale
// allowlist entry too.

/// Port of `int bindkey(Keymap km, const char *seq, Thingy bind,
/// char *str)` from `Src/Zle/zle_keymap.c:566`. Bind a key sequence in
/// a named keymap. C semantics: return 1 if the keymap is
/// `KM_IMMUTABLE`, 2 if `seq` is empty, 0 on success. If `bind` is
/// `t_undefinedkey` or `ztrlen(seq) > 1`, the binding lives in the
/// multi-char trie (`km->multi`); else it goes in the single-byte
/// `km->first[f]` array. The single-byte path handles the
/// prefix-due-to-send-string case by removing the trie entry when
/// `k->prefixct == 0`. The Rust port delegates the trie walk to
/// `bind_seq` and the multi-char `domulti` branch; the KM_IMMUTABLE
/// and empty-seq early returns are checked here.
/// WARNING: param names don't match C — Rust=(keymap, seq, widget) vs C=(km, seq, bind, str)
pub fn bindkey(keymap: &str, seq: &str, widget: &str) -> bool {
    // c:566
    let seq_bytes = getkeystring(seq); // c:569 seq[0]
    let mut tab = crate::ported::zle::zle_keymap::keymapnamtab()
        .lock()
        .unwrap();
    let node = match tab.get_mut(keymap) {
        // c:566 Keymap km
        Some(n) => n,
        None => return false, // C: caller resolves Keymap
    };
    // c:572 — KM_IMMUTABLE check
    if (node.keymap.flags & KM_IMMUTABLE) != 0 {
        // c:572
        return false; // c:573 return 1
    }
    // c:574 — !*seq check
    if seq_bytes.is_empty() {
        // c:574
        return false; // c:575 return 2
    }
    let inner = std::sync::Arc::make_mut(&mut node.keymap);
    // c:576 — single-vs-multi byte dispatch delegates to
    // `bind_seq`, which handles the prefix promotion (c:577-586),
    // trie insert (c:631-641), and `km->first[f]` single-byte
    // fast-path (c:600).
    inner.bind_seq(&seq_bytes, Thingy::new(widget));
    true // c:650 return 0
}

/// Enumerate every (key-sequence, widget-name) pair in `keymap`.
/// Port of `bindkey -L` listing from Src/Zle/zle_keymap.c (the
/// listing branch of `bin_bindkey`). Both 1-byte fast-path entries
/// (`first[]`) and multi-byte trie entries (`multi`) are included.
pub fn bindlistout(keymap: &str) -> Vec<(String, String)> {
    // c:zle_keymap.c:1094
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

/// All known internal widget names — the Rust analog of C's
/// `Src/Zle/thingies.list` (391 entries) restricted to the names
/// that have a Rust port in `iwidget_lookup` below. Walked at
/// startup by `init_thingies()` to populate THINGYTAB so `zle -l`
/// can enumerate widgets without each name having to be invoked
/// first via the keymap.
pub const IWIDGET_NAMES: &[&str] = &[
    "accept-and-hold",
    "accept-line",
    "accept-line-and-down-history",
    "backward-char",
    "backward-delete-char",
    "backward-kill-word",
    "backward-word",
    "beep",
    "beginning-of-buffer-or-history",
    "beginning-of-line",
    "capitalize-word",
    "clear-screen",
    "copy-prev-word",
    "copy-region-as-kill",
    "delete-char-or-list",
    "digit-argument",
    "down-case-word",
    "down-history",
    "down-line-or-history",
    "end-of-buffer-or-history",
    "end-of-line",
    "execute-last-named-cmd",
    "execute-named-cmd",
    "expand-history",
    "expand-or-complete",
    "forward-char",
    "forward-word",
    "get-line",
    "history-incremental-search-backward",
    "history-incremental-search-forward",
    "history-search-backward",
    "history-search-forward",
    "insert-last-word",
    "kill-line",
    "kill-whole-line",
    "kill-word",
    "list-choices",
    "list-expand",
    "neg-argument",
    "pound-insert",
    "push-line",
    "quote-line",
    "quote-region",
    "quoted-insert",
    "redisplay",
    "redo",
    "run-help",
    "self-insert",
    "self-insert-unmeta",
    "send-break",
    "set-mark-command",
    "spell-word",
    "transpose-chars",
    "transpose-words",
    "undefined-key",
    "undo",
    "up-case-word",
    "up-history",
    "up-line-or-history",
    "vi-add-eol",
    "vi-add-next",
    "vi-backward-blank-word",
    "vi-backward-char",
    "vi-backward-delete-char",
    "vi-backward-kill-word",
    "vi-backward-word",
    "vi-change",
    "vi-change-eol",
    "vi-change-whole-line",
    "vi-cmd-mode",
    "vi-delete",
    "vi-delete-char",
    "vi-digit-or-beginning-of-line",
    "vi-down-line-or-history",
    "vi-end-of-line",
    "vi-fetch-history",
    "vi-find-next-char",
    "vi-find-next-char-skip",
    "vi-find-prev-char",
    "vi-find-prev-char-skip",
    "vi-first-non-blank",
    "vi-forward-blank-word",
    "vi-forward-blank-word-end",
    "vi-forward-char",
    "vi-forward-word",
    "vi-forward-word-end",
    "vi-goto-column",
    "vi-goto-mark",
    "vi-goto-mark-line",
    "vi-history-search-backward",
    "vi-history-search-forward",
    "vi-indent",
    "vi-insert",
    "vi-insert-bol",
    "vi-join",
    "vi-kill-eol",
    "vi-kill-line",
    "vi-match-bracket",
    "vi-open-line-above",
    "vi-open-line-below",
    "vi-put-after",
    "vi-put-before",
    "vi-quoted-insert",
    "vi-repeat-change",
    "vi-repeat-find",
    "vi-repeat-search",
    "vi-replace",
    "vi-replace-chars",
    "vi-rev-repeat-find",
    "vi-rev-repeat-search",
    "vi-set-buffer",
    "vi-set-mark",
    "vi-substitute",
    "vi-swap-case",
    "vi-unindent",
    "vi-up-line-or-history",
    "vi-yank",
    "vi-yank-whole-line",
    "visual-line-mode",
    "visual-mode",
    "which-command",
    "yank",
    "yank-pop",
];

/// Lookup the canonical fn pointer for a built-in widget name.
/// Direct port of the dispatch achieved by C's
/// `Src/Zle/zle_bindings.c:55-60 widgets[]` table generated from
/// `iwidgets.list`. Each name → `ZleIntFunc` (the C signature is
/// `int (*)(char **)`). Returns `None` for widget names not in
/// the table; callers receive a no-op fn pointer matching what
/// `t_undefinedkey` resolves to in C.
pub fn iwidget_lookup(name: &str) -> Option<super::zle_h::ZleIntFunc> {
    match name {
        // "beep" → handlefeep at zle_utils.c:1405 (C source uses
        // handlefeep as the bound fn; "beep" is the canonical widget
        // name per iwidgets.list).
        "beep" => Some(|_| handlefeep()),
        "accept-and-hold" => Some(|_| acceptandhold()),
        "accept-line-and-down-history" => Some(|_| acceptlineanddownhistory()),
        "accept-line" => Some(|_| acceptline()),
        "backward-char" => Some(|_| backwardchar()),
        "backward-delete-char" => Some(|_| backwarddeletechar()),
        "backward-kill-word" => Some(backwardkillword),
        "backward-word" => Some(backwardword),
        "beginning-of-buffer-or-history" => Some(|_| beginningofbufferorhistory()),
        "beginning-of-line" => Some(|_| beginningofline()),
        "capitalize-word" => Some(capitalizeword),
        "copy-prev-word" => Some(|_| copyprevword()),
        "copy-region-as-kill" => Some(copyregionaskill),
        "delete-char-or-list" => Some(|_| deletecharorlist()),
        "digit-argument" => Some(|_| digitargument()),
        "down-case-word" => Some(downcaseword),
        "down-history" => Some(|_| downhistory()),
        "down-line-or-history" => Some(|_| downlineorhistory()),
        "end-of-buffer-or-history" => Some(|_| endofbufferorhistory()),
        "end-of-line" => Some(|_| endofline()),
        "expand-history" => Some(|_| expandhistory()),
        "expand-or-complete" => Some(|_| expandorcomplete()),
        "forward-char" => Some(|_| forwardchar()),
        "forward-word" => Some(forwardword),
        "history-incremental-search-backward" => Some(|_| historyincrementalsearchbackward()),
        "history-incremental-search-forward" => Some(|_| historyincrementalsearchforward()),
        "history-search-backward" => Some(|_| historysearchbackward()),
        "history-search-forward" => Some(|_| historysearchforward()),
        "insert-last-word" => Some(|_| insertlastword()),
        "kill-line" => Some(|_| killline()),
        "kill-whole-line" => Some(|_| killwholeline()),
        "kill-word" => Some(killword),
        "list-choices" => Some(|_| listchoices()),
        "list-expand" => Some(|_| listexpand()),
        "neg-argument" => Some(|_| negargument()),
        "pound-insert" => Some(|_| poundinsert()),
        "push-line" => Some(|_| pushline()),
        "quote-line" => Some(|_| quoteline()),
        "quote-region" => Some(|_| quoteregion()),
        "quoted-insert" => Some(|_| quotedinsert()),
        "redo" => Some(|_| redo()),
        "self-insert-unmeta" => Some(|_| selfinsertunmeta()),
        "self-insert" => Some(|_| selfinsert()),
        "send-break" => Some(|_| sendbreak()),
        "set-mark-command" => Some(|_| setmarkcommand()),
        "spell-word" => Some(|_| spellword()),
        "transpose-chars" => Some(|_| transposechars()),
        "transpose-words" => Some(transposewords),
        "undefined-key" => Some(|_| undefinedkey()),
        "undo" => Some(undo),
        "up-case-word" => Some(upcaseword),
        "up-history" => Some(|_| uphistory()),
        "up-line-or-history" => Some(|_| uplineorhistory()),
        "vi-add-eol" => Some(|_| viaddeol()),
        "vi-add-next" => Some(|_| viaddnext()),
        "vi-backward-blank-word" => Some(vibackwardblankword),
        "vi-backward-char" => Some(|_| vibackwardchar()),
        "vi-backward-delete-char" => Some(|_| vibackwarddeletechar()),
        "vi-backward-kill-word" => Some(vibackwardkillword),
        "vi-backward-word" => Some(vibackwardword),
        "vi-change-eol" => Some(|_| vichangeeol()),
        "vi-change-whole-line" => Some(|_| vichangewholeline()),
        "vi-change" => Some(|_| vichange()),
        "vi-cmd-mode" => Some(|_| vicmdmode()),
        "vi-delete-char" => Some(|_| videletechar()),
        "vi-delete" => Some(|_| videlete()),
        "vi-digit-or-beginning-of-line" => Some(|_| vidigitorbeginningofline()),
        "vi-down-line-or-history" => Some(|_| vidownlineorhistory()),
        "vi-end-of-line" => Some(|_| viendofline()),
        "vi-fetch-history" => Some(|_| vifetchhistory()),
        "vi-find-next-char-skip" => Some(|_| vifindnextcharskip()),
        "vi-find-next-char" => Some(|_| vifindnextchar()),
        "vi-find-prev-char-skip" => Some(|_| vifindprevcharskip()),
        "vi-find-prev-char" => Some(|_| vifindprevchar()),
        "vi-first-non-blank" => Some(|_| vifirstnonblank()),
        "vi-forward-blank-word-end" => Some(viforwardblankwordend),
        "vi-forward-blank-word" => Some(viforwardblankword),
        "vi-forward-char" => Some(|_| viforwardchar()),
        "vi-forward-word-end" => Some(viforwardwordend),
        "vi-forward-word" => Some(viforwardword),
        "vi-goto-column" => Some(|_| vigotocolumn()),
        // vi-goto-mark / vi-goto-mark-line / vi-set-mark read a
        // second key char before dispatching (C body c:887/c:929/c:872).
        // The keymap-level dispatch supplies the char via
        // `getrestchar_keybuf`; this fn-ptr wrapper passes NUL since
        // the dispatch is from a static table — the body re-reads
        // the next key itself.
        "vi-goto-mark-line" => Some(|_| vigotomarkline('\0')),
        "vi-goto-mark" => Some(|_| vigotomark('\0')),
        "vi-history-search-backward" => Some(|_| vihistorysearchbackward()),
        "vi-history-search-forward" => Some(|_| vihistorysearchforward()),
        "vi-indent" => Some(|_| viindent()),
        "vi-insert-bol" => Some(|_| viinsertbol()),
        "vi-insert" => Some(|_| viinsert()),
        "vi-join" => Some(|_| vijoin()),
        "vi-kill-eol" => Some(|_| vikilleol()),
        "vi-kill-line" => Some(|_| vikillline()),
        "vi-match-bracket" => Some(|_| vimatchbracket()),
        "vi-open-line-above" => Some(|_| viopenlineabove()),
        "vi-open-line-below" => Some(|_| viopenlinebelow()),
        "vi-put-after" => Some(|_| viputafter()),
        "vi-put-before" => Some(|_| viputbefore()),
        "vi-quoted-insert" => Some(|_| viquotedinsert()),
        "vi-repeat-change" => Some(|_| virepeatchange()),
        "vi-repeat-find" => Some(|_| virepeatfind()),
        "vi-repeat-search" => Some(|_| virepeatsearch()),
        "vi-replace-chars" => Some(|_| vireplacechars()),
        "vi-replace" => Some(|_| vireplace()),
        "vi-rev-repeat-find" => Some(|_| virevrepeatfind()),
        "vi-rev-repeat-search" => Some(|_| virevrepeatsearch()),
        "vi-set-buffer" => Some(|_| visetbuffer()),
        "vi-set-mark" => Some(|_| visetmark('\0')),
        "vi-substitute" => Some(|_| visubstitute()),
        "vi-swap-case" => Some(|_| viswapcase()),
        "vi-unindent" => Some(|_| viunindent()),
        "vi-up-line-or-history" => Some(|_| viuplineorhistory()),
        "vi-yank-whole-line" => Some(|_| viyankwholeline()),
        "visual-line-mode" => Some(|_| visuallinemode()),
        "visual-mode" => Some(|_| visualmode()),
        "yank-pop" => Some(|_| yankpop()),
        // Below: widget names that map to C ported with non-1:1 names
        // (per iwidgets.list). C uses the same fn for multiple
        // widget names — dispatch by bindk->nam inside the body.
        // clear-screen / redisplay / yank — existing `pub fn`s
        // live inside inner scopes in zle_refresh.rs / zle_misc.rs
        // (legacy nested impl/mod blocks). Inline minimal bodies
        // here matching the C source (zle_refresh.c:2366/2377,
        // zle_misc.c:533). Will redirect to canonical ported once
        // the inner-scope wrapping is unwound.
        "clear-screen" => Some(|_| {
            // Port of `clearscreen(char **args)` from
            // `Src/Zle/zle_refresh.c:2366`. C: `tcout(TCHOMEDOWN);
            // tcout(TCCLEAREOD); resetneeded = 1;`. The two termcap
            // sequences are H (home cursor) + J (clear to end). Write
            // to SHTTY (stdout fallback) instead of stdout.
            use std::sync::atomic::Ordering;
            let fd = crate::ported::init::SHTTY.load(Ordering::Relaxed);
            let out = if fd >= 0 { fd } else { 1 };
            let _ = crate::ported::utils::write_loop(out, b"\x1b[H\x1b[2J");
            ZLE_RESET_NEEDED.store(1, Ordering::SeqCst);
            0
        }),
        "redisplay" => Some(|_| {
            ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
            0
        }),
        "yank" => Some(|_| {
            let ring = KILLRING.lock().unwrap();
            let text = match ring.front() {
                Some(t) => t.clone(),
                None => return 1,
            };
            drop(ring);
            let cs = ZLECS.load(std::sync::atomic::Ordering::SeqCst);
            let mut line = ZLELINE.lock().unwrap();
            for (i, c) in text.iter().enumerate() {
                line.insert(cs + i, *c);
            }
            let new_ll = line.len();
            drop(line);
            ZLELL.store(new_ll, std::sync::atomic::Ordering::SeqCst);
            ZLECS.store(cs + text.len(), std::sync::atomic::Ordering::SeqCst);
            0
        }),
        "vi-yank" => Some(viyank),
        "which-command" => Some(super::zle_misc::processcmd),
        "run-help" => Some(super::zle_misc::processcmd),
        "get-line" => Some(|_| super::zle_hist::zgetline()),
        // execute-named-cmd / execute-last-named-cmd have NULL fn
        // in C iwidgets.list — handled inline at C bind dispatch
        // via `bindk->nam` check. The Rust dispatch path doesn't
        // need a fn pointer for them; mapped here so the keymap
        // lookup yields a valid `Thingy` rather than no-op.
        "execute-named-cmd" => Some(|_| 0),
        "execute-last-named-cmd" => Some(|_| 0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindkey_returns_false_for_unknown_keymap() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        crate::ported::zle::zle_keymap::createkeymapnamtab();
        crate::ported::zle::zle_keymap::default_bindings();
        assert!(!bindkey("no-such-keymap", "^A", "self-insert"));
    }

    #[test]
    fn bindkey_then_unbind_round_trips_through_emacs_keymap() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
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
        let mut tab = crate::ported::zle::zle_keymap::keymapnamtab()
            .lock()
            .unwrap();
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

    /// c:utils.c:6915 — `getkeystring` decodes `\e` / `\t` / `\n` /
    /// `\C-x` / `\M-x` escape sequences into raw bytes for keymap
    /// binding. Verify the common shorthand pairs.
    #[test]
    fn getkeystring_decodes_canonical_escapes() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getkeystring("\\e"), vec![0x1b]); // ESC
        assert_eq!(getkeystring("\\t"), vec![0x09]); // TAB
        assert_eq!(getkeystring("\\n"), vec![0x0a]); // LF
        assert_eq!(getkeystring("\\r"), vec![0x0d]); // CR
    }

    /// `\C-x` shorthand maps to control-byte `x & 0x1f` (the ASCII
    /// ctrl-char) per the C decoder.
    #[test]
    fn getkeystring_decodes_control_prefix() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getkeystring("\\C-a"), vec![0x01]); // ctrl-a
        assert_eq!(getkeystring("\\C-c"), vec![0x03]); // ctrl-c
    }

    /// `\M-x` shorthand (Meta-x) decodes to ESC + the byte (or to the
    /// 0x80-bit-set byte depending on Meta-mode). Either way: non-empty.
    #[test]
    fn getkeystring_decodes_meta_prefix() {
        let _g = crate::test_util::global_state_lock();
        let b = getkeystring("\\M-a");
        assert!(!b.is_empty(), "\\M-a must decode to at least 1 byte");
    }

    /// Plain ASCII passes through verbatim.
    #[test]
    fn getkeystring_passes_plain_ascii_through() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getkeystring("abc"), b"abc".to_vec());
    }

    /// `iwidget_lookup` resolves canonical zle widget names → fn ptr.
    /// `self-insert` is the must-have widget (every printable key is
    /// bound to it by default).
    #[test]
    fn iwidget_lookup_resolves_self_insert() {
        let _g = crate::test_util::global_state_lock();
        assert!(iwidget_lookup("self-insert").is_some());
    }

    /// `iwidget_lookup` for unknown widget names returns None.
    #[test]
    fn iwidget_lookup_returns_none_for_unknown_name() {
        let _g = crate::test_util::global_state_lock();
        assert!(iwidget_lookup("definitely-not-a-real-widget-zshrs").is_none());
    }

    /// `bindlistout` for an unknown keymap returns an empty vec —
    /// safer than panicking when bindkey is queried before init.
    #[test]
    fn bindlistout_empty_for_unknown_keymap() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert!(bindlistout("does-not-exist").is_empty());
    }

    /// c:utils.c:6915 — `getkeystring("")` returns empty. Defensive
    /// edge case so a regression panicking on empty doesn't crash
    /// every `bindkey ""` invocation.
    #[test]
    fn getkeystring_empty_string_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        assert!(getkeystring("").is_empty());
    }

    /// c:utils.c:6915 — `\\b` decodes to backspace (0x08). Pin the
    /// canonical backspace shortcut so a regen that maps it to DEL
    /// (0x7f) gets caught.
    #[test]
    fn getkeystring_decodes_backspace() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getkeystring("\\b"), vec![0x08]);
    }

    /// c:utils.c:6915 — `\\C-A` is equivalent to `\\C-a` (control
    /// shortcuts are case-INsensitive per zsh convention).
    #[test]
    fn getkeystring_control_prefix_is_case_insensitive() {
        let _g = crate::test_util::global_state_lock();
        let lower = getkeystring("\\C-a");
        let upper = getkeystring("\\C-A");
        assert_eq!(lower, upper, r#"\\C-a and \\C-A must decode identically"#);
        assert_eq!(lower, vec![0x01]);
    }

    /// `iwidget_lookup` for `accept-line` resolves — the most-used
    /// widget (Enter key). Smoke test for the internal-widget table
    /// every zsh user depends on.
    #[test]
    fn iwidget_lookup_resolves_accept_line() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            iwidget_lookup("accept-line").is_some(),
            "accept-line is the canonical Enter-key widget; must resolve"
        );
    }

    /// `iwidget_lookup` for empty string returns None. Defensive
    /// boundary.
    #[test]
    fn iwidget_lookup_empty_name_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(iwidget_lookup("").is_none());
    }

    /// c:566 — `bindkey` does NOT validate widget existence at
    /// bind time (C resolves the widget via Thingy at trigger
    /// time). Pin that an unknown-widget bind SUCCEEDS — a regen
    /// that tightens this to reject unknowns would break user
    /// scripts that bind to widgets defined later in startup.
    #[test]
    fn bindkey_unknown_widget_binds_anyway_matching_c() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let ok = bindkey("main", "\\C-x", "user-widget-not-yet-defined");
        assert!(
            ok,
            "C source resolves widgets at trigger time, so bind-time \
             unknowns must SUCCEED"
        );
    }

    // ─── zsh-corpus pins for getkeystring escapes ──────────────────

    /// `getkeystring("^A")` → [0x01] (control-char shorthand).
    #[test]
    fn zle_bindings_corpus_getkeystring_caret_A_is_ctrl_a() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getkeystring("^A"), vec![0x01]);
    }

    /// `getkeystring("^?")` → [0x7f] DEL.
    #[test]
    fn zle_bindings_corpus_getkeystring_caret_question_is_del() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getkeystring("^?"), vec![0x7f]);
    }

    /// `getkeystring("^[")` → [0x1b] ESC.
    #[test]
    fn zle_bindings_corpus_getkeystring_caret_bracket_is_esc() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getkeystring("^["), vec![0x1b]);
    }

    /// `getkeystring(r"\e")` → [0x1b].
    #[test]
    fn zle_bindings_corpus_getkeystring_backslash_e_is_esc() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getkeystring(r"\e"), vec![0x1b]);
    }

    /// `getkeystring(r"\t")` → [0x09] TAB.
    #[test]
    fn zle_bindings_corpus_getkeystring_backslash_t_is_tab() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getkeystring(r"\t"), vec![0x09]);
    }

    /// `getkeystring(r"\n")` → [0x0a] LF.
    #[test]
    fn zle_bindings_corpus_getkeystring_backslash_n_is_lf() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getkeystring(r"\n"), vec![0x0a]);
    }

    /// `getkeystring(r"\r")` → [0x0d] CR.
    #[test]
    fn zle_bindings_corpus_getkeystring_backslash_r_is_cr() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getkeystring(r"\r"), vec![0x0d]);
    }

    /// `getkeystring("")` is empty.
    #[test]
    fn zle_bindings_corpus_getkeystring_empty_is_empty() {
        let _g = crate::test_util::global_state_lock();
        assert!(getkeystring("").is_empty());
    }

    /// `getkeystring("abc")` (no escapes) is byte-identical.
    #[test]
    fn zle_bindings_corpus_getkeystring_plain_ascii_passthrough() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getkeystring("abc"), vec![b'a', b'b', b'c']);
    }
}
