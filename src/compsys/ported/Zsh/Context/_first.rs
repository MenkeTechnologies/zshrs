//! Port of `_first` from `Completion/Zsh/Context/_first`.
//!
//! Full upstream body (47 lines verbatim):
//! ```text
//! sh: 1  #compdef -first-
//! sh: 2
//! sh: 3  # This function is called at the very beginning before any other
//! sh: 4  # function for a specific context.
//! sh: 5  #
//! sh: 6  # This just gives some examples of things you might want to do here.
//! sh: 7  #
//! sh: 8  #
//! sh: 9  # Other things you can do here is to complete different things if the
//! sh:10  # word on the line matches a certain pattern. This example allows
//! sh:11  # completion of words from the history by adding two commas at the end
//! sh:12  # and hitting TAB.
//! sh:13  #
//! sh:14  #     if [[ "$PREFIX" = *,, ]]; then
//! sh:15  #       local max i=1 expl opt
//! sh:16  #
//! sh:17  #       PREFIX="$PREFIX[1,-2]"
//! sh:18  #       # If a numeric prefix is given, we use it as the number of
//! sh:19  #       # lines (multiplied by ten below) in the history to search.
//! sh:20  #       if [[ ${NUMERIC:-1} -gt 1 ]]; then
//! sh:21  #         max=$NUMERIC
//! sh:22  #         unset NUMERIC
//! sh:23  #       else
//! sh:24  #         # The default is to search the last 100 lines.
//! sh:25  #         max=10
//! sh:26  #       fi
//! sh:27  #       # We first search in the last ten words, then in the last
//! sh:28  #       # twenty words, and so on...
//! sh:29  #       while [[ i -le max ]]; do
//! sh:30  #         if zstyle -t ":completion:${curcontext}:history-words" sort; then
//! sh:31  #           opt=-J
//! sh:32  #         else
//! sh:33  #           opt=-V
//! sh:34  #         fi
//! sh:35  #         if _wanted "$opt" history-words expl "history ($n)" \
//! sh:36  #                compadd -Q - \
//! sh:37  #                    "${(@)${(@)historywords:#[\$'\"]*}[1,i*10]}"; then
//! sh:38  #           # We have found at least one matching word, so we switch
//! sh:39  #           # on menu-completion and make sure that no other
//! sh:40  #           # completion function is called by setting _compskip.
//! sh:41  #           compstate[insert]=menu
//! sh:42  #           _compskip=all
//! sh:43  #           return 0
//! sh:44  #         fi
//! sh:45  #         (( i++ ))
//! sh:46  #       done
//! sh:47  #     fi
//! ```
//!
//! Upstream `_first` is a USER-OVERRIDE HOOK — the default body is
//! empty (only comments). End users redefine it to customize the
//! first-thing-that-runs behavior.
//!
//! Strict Rust port: returns false (no first-thing override active).
//! Calling code can replace the default by calling a registered
//! `_first` fn via [`crate::compsys::ported::_call_function`].

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
