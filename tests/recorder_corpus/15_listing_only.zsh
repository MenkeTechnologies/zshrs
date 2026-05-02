# Listing-only invocations are NOT state mutations. None of these lines
# should produce records. The single trailing alias definition is the
# only event the harness expects.
alias
alias -L
setopt
unsetopt
trap
trap -l
hash -L
zstyle
zmodload -l
bindkey
alias only_real_one='captured'
