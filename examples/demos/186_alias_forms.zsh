#!/usr/bin/env zshrs
# Alias forms — regular, global, suffix.
# Ported from Src/builtin.c bin_alias + Src/Modules/parameter.c aliasestab.

echo "── regular alias ──"
alias ll='ls -la'
alias gst='git status'
alias mkdir='mkdir -p'
alias | head -5

echo "── query a specific alias ──"
alias ll
alias gst

echo "── unalias ──"
alias temp='echo hello'
echo "before: $(alias temp)"
unalias temp
echo "after: $(alias temp 2>&1)"

echo "── global alias (-g) ──"
alias -g G='| grep'
alias -g L='| less'
alias -g | head -5

echo "── suffix alias (-s) ──"
alias -s zsh='cat'  # cat .zsh files when typed alone
alias -s | head -5

echo "── list all aliases ──"
echo "total aliases: $(alias | wc -l)"

echo "── alias expansion in [[ ]] ──"
alias mygreet='echo hello'
result=$(mygreet)
echo "via alias: $result"

echo "── alias that references variables ──"
target=world
alias greet_target='echo target=$target'
greet_target
target=universe
greet_target

echo "── alias chain (one calls another) ──"
alias first='echo first'
alias second='first; echo second'
second

echo "── disable an alias temporarily ──"
\mkdir /tmp/zshrs_alias_$$ 2>/dev/null
ls -d /tmp/zshrs_alias_$$ 2>/dev/null && rmdir /tmp/zshrs_alias_$$

# Cleanup
unalias ll gst mygreet greet_target first second 2>/dev/null
unalias -g G L 2>/dev/null
unalias -s zsh 2>/dev/null
unalias mkdir 2>/dev/null
echo "after cleanup: $(alias | wc -l) aliases remaining"
