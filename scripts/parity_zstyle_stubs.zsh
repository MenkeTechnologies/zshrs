# Companion to parity_zstyle.zsh — definitions for the functions that
# fixture NAMES but that live outside zsh's own Completion tree.
#
# Why this file exists
# --------------------
# parity_zstyle.zsh is a capture of the author's live `zstyle -L`. Several
# styles are function-VALUED, and seven of the functions they name ship
# with zpwr/fasd rather than with zsh:
#
#   cache-policy  zpwrMonthlyCachingPolicy  zpwrDailyCachingPolicy
#   completer     _megacomplete  _fasd_zsh_word_complete{,_d,_f,_trigger}
#
# In a harness shell none of them exist, so compsys silently takes a
# different path than it does in the author's session: `_main_complete`
# skips an undefined completer, and `_retrieve_cache` treats a missing
# cache-policy as "always rebuild". Both shells behave the same way, so the
# parity verdict stays valid — but it is a verdict about a DIFFERENT
# completer chain than the one being modelled, which makes the fixture a
# weaker test than it looks.
#
# Sourced AFTER parity_zstyle.zsh. Kept separate because that file is
# regenerated wholesale by scripts/dump_live_zstyle.py, which would discard
# anything added to it.
#
# Contract, not behaviour
# -----------------------
# These reproduce each function's RETURN CONTRACT, which is what compsys
# branches on. Where the real body is self-contained it is reproduced
# exactly; where it depends on the author's environment (tmux panes, a fasd
# database) the stub returns the no-match status instead of faking data.
#
# Getting the status wrong is not cosmetic. A completer that returns 0
# without adding matches STOPS the chain: `_main_complete` treats it as
# success, and every later completer (`_approximate`, `_correct`, …) never
# runs. That exact bug — `_first` returning 0 from an empty body — silently
# reduced every multi-completer config to `_complete`-only until it was
# found. So a stub that cannot produce matches MUST return 1.

# --- cache-policy -----------------------------------------------------
# Reproduced verbatim from zpwr's autoload/common/zpwrBindZstyle. Portable
# as-is: a `(Nm+N)` glob qualifier and nothing else. Returns 0 (rebuild)
# when the cache file is older than N days, which is what `_cache_invalid`
# tests. Exercising the real body keeps the `(Nm+N)` qualifier itself in
# the parity surface.
zpwrMonthlyCachingPolicy () {
    # rebuild if cache is more than a month old
    local -a oldp
    oldp=( "$1"(Nm+31) )
    (( $#oldp ))
}

zpwrWeeklyCachingPolicy () {
    # rebuild if cache is more than a week old
    local -a oldp
    oldp=( "$1"(Nm+7) )
    (( $#oldp ))
}

zpwrDailyCachingPolicy () {
    # rebuild if cache is more than a day old
    local -a oldp
    oldp=( "$1"(Nm+1) )
    (( $#oldp ))
}

# --- completers -------------------------------------------------------
# The real `_megacomplete` runs `\_complete` and returns its status, then
# adds tmux-pane words and a few other sources when the environment offers
# them. The delegation is the part the chain depends on, and it is
# reproducible; the extra sources need a live tmux pane, so they are left
# out rather than faked.
_megacomplete () {
    local ret
    \_complete && ret=0 || ret=1
    return ret
}

# fasd completers need a populated fasd database. With none, the honest
# result is "no matches", i.e. status 1 so the chain continues to
# `_approximate` / `_correct` exactly as it would when fasd has nothing to
# offer for this word.
_fasd_zsh_word_complete ()         { return 1 }
_fasd_zsh_word_complete_d ()       { return 1 }
_fasd_zsh_word_complete_f ()       { return 1 }
_fasd_zsh_word_complete_trigger () { return 1 }
