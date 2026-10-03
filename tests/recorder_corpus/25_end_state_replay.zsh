# End-state round trip: `recorder_end_state_replays_into_a_shell` records
# this file into a scratch $ZSHRS_HOME, replays the shard in `zshrs -i -c`,
# and checks each table below came back as the file left it. Every line
# targets a shape the event fold lost.

# A body whose quoting the `$functions` deparse drops.
f_quote() { local mark='_.!~*'\''()-'; print -r -- "$mark" }

# Association keys, including one with a space.
typeset -gA H=(k1 v1 'k 2' 'v 2')

# `typeset -U` and a `typeset -T` tie with a non-default separator.
typeset -gU uarr=(a b a)
typeset -gT TIED_S tied_a ';'
tied_a=(x y)

# Integer and hidden-value attributes.
integer -g NUM=42
typeset -gH HIDDEN=secret

# Local-only names must not leak into the replay.
() { local GONE_LOCAL=1; typeset -g KEPT_GLOBAL=1 }

# Removed from the environment the shell started with.
unset REPLAY_GONE

# A value with spaces, and an `-e` style.
zstyle ':t:x' fmt '%d (errors: %e)'
zstyle -e ':t:e' ev 'reply=(x)'

# A vicmd text-object binding must stay in vicmd; a send-string decodes.
bindkey -M vicmd 'a-' vi-add-next
bindkey -M emacs -s '^Xq' 'hi'

# Widget, math function, module feature, directory autoload.
zle -N my-widget f_quote
f_math() { (( $1 * 2 )) }
functions -M mf 1 1 f_math
zmodload -F zsh/files b:zf_rm
autoload -Uz ${0:A:h}/replay_fns/replay_dir_fn

# A value built from the terminal is the replaying shell's, not the
# recorder's; a named directory abbreviates the prompt path.
typeset -g MY_TTY="tty=$TTY"
hash -d RTD=${0:A:h}

# `$$` in a value is the replaying shell's; a file that takes a descriptor
# runs again, so the descriptor is open and this shell's.
typeset -g MY_PID_FILE=/tmp/rt.$$
source ${0:A:h}/replay_fns/fd_owner.zsh

alias ll='ls -l'
alias -g G='| head'
setopt extendedglob
