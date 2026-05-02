# All major dispatchers in one file — exercises the full surface.
alias gst='git status -sb'
alias -g G='| grep'
alias -s txt=cat
export EDITOR=vim
PROJECT_ROOT="/tmp/proj"
hash -d zpwr=/tmp/zpwr
zstyle ':completion:*' menu select
bindkey '^R' fzf-history-widget
compdef _git git
zmodload zsh/datetime
setopt EXTENDED_GLOB
unsetopt BEEP
trap 'echo bye' EXIT
my_func() { echo hi; }
