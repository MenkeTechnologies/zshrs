//! Per-thread record of alias input-stack frames that have already been
//! popped.
//!
//! **zshrs-original infrastructure — no `Src/*.c` counterpart.** It exists
//! because zshrs's input stack is a `Vec` and C's is a manually-indexed
//! array.
//!
//! `Src/input.c:831 input_hasalias()` answers "is the text currently being
//! lexed coming out of an alias expansion, and if so which alias?" It does
//! that by walking DOWN `instack` from `instacktop` while the flags carry
//! `INP_CONT`, remembering every frame that has an `alias` set
//! (`Src/input.c:837-846`). `Src/parse.c:1840` captures the answer at the
//! head of `par_simple` and `Src/parse.c:2061` compares it against a fresh
//! call to decide whether `name() { … }` is being defined out of an alias
//! body (the `ALIAS_FUNC_DEF` diagnostic).
//!
//! The subtlety is that by the time the parser sees the `()` the alias body
//! has been fully lexed and its frame ALREADY POPPED. C survives that
//! because `inpoptop` (`Src/input.c:757-763`) only decrements `instacktop` —
//! the frame's storage is untouched — and `inungetc` walks BACK UP over
//! exhausted alias continuations whenever the lexer pushes the terminator
//! back at a segment start:
//!
//! ```c
//! if (inbufptr == inbufpush && (inbufflags & (INP_ALCONT|INP_HISTCONT))) {
//!     do {
//!         if (instacktop->alias) instacktop->alias->inuse = 1;
//!         instacktop++;
//!     } while ((instacktop->flags & (INP_ALCONT|INP_HISTCONT)) && !instacktop->bufleft);
//!     ...
//! }
//! ```
//! (`Src/input.c:587-605`)
//!
//! zshrs's `instack` is a `Vec<instacks>` whose `pop()` DESTROYS the frame,
//! so that walk-up cannot be reconstructed. This module records the alias
//! name of each frame `inpoptop` restores that carries `INP_ALCONT`, in
//! push order, so `input_hasalias` can report the OUTERMOST one — the same
//! answer C's downward walk produces for a nested expansion
//! (`secondalias` → `firstalias` → body reports `secondalias`,
//! `Test/A02alias.ztst:127-131`).
//!
//! The record is bounded to ONE `zshlex` call (`Src/lex.c:268`), which is
//! the whole `gettok`/`exalias` chain that produces a single token — so a
//! nested expansion still records every alias in the chain, while a chain
//! that finished during an earlier token cannot answer for a later one.
//! `inpoptop` additionally clears it when a pop lands on flags without
//! `INP_ALCONT`.

use std::cell::RefCell;

thread_local! {
    /// Alias names of popped `INP_ALCONT` frames, outermost first.
    static ALCONT_ALIASES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Record the alias name of a just-popped `INP_ALCONT` frame.
pub fn push(name: &str) {
    ALCONT_ALIASES.with(|a| a.borrow_mut().push(name.to_string()));
}

/// Drop the whole record — the alias chain it described has ended.
pub fn clear() {
    ALCONT_ALIASES.with(|a| a.borrow_mut().clear());
}

/// The outermost recorded alias name, i.e. what C's downward `instack`
/// walk would have settled on.
pub fn outermost() -> Option<String> {
    ALCONT_ALIASES.with(|a| a.borrow().first().cloned())
}
