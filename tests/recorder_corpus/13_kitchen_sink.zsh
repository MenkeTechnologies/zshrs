# All major dispatchers in one file — exercises the full surface.
alias gst='git status -sb'
alias -g G='| grep'
alias -s txt=cat
export EDITOR=vim
PROJECT_ROOT="/tmp/proj"
hash -d zpwr=/tmp/zpwr
zstyle ':completion:*' menu select
bindkey '^R' fzf-history-widget
# `compdef` is defined BY compinit — without it the call is
# command-not-found in zsh and in zshrs alike, and no event is emitted.
# `-u` skips the insecure-directory prompt, `-D` skips the dump file.
autoload -Uz compinit
compinit -u -D
compdef _git git
zmodload zsh/datetime
setopt EXTENDED_GLOB
unsetopt BEEP
trap 'echo bye' EXIT
my_func() { echo hi; }
