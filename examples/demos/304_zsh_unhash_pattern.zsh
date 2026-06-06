#!/usr/bin/env zshrs
# unhash + unset + unalias + unfunction w/ -m pattern matching.
# Ports Src/builtin.c bin_unhash / bin_unset / bin_unalias / bin_unfunction.

echo "── setup: bunch of aliases ──"
alias myls_short='ls -1'
alias myls_long='ls -la'
alias myls_color='ls --color=auto'
alias mygit_log='git log --oneline'
alias mygit_diff='git diff'
alias mygrep='grep -E'

echo "  defined aliases:"
alias 2>/dev/null | grep -E "^my(ls|git|grep)" | sed 's/^/    /'

echo
echo "── unalias -m 'myls_*' removes all matching ──"
unalias -m 'myls_*' 2>/dev/null
echo "  after unalias -m 'myls_*':"
alias 2>/dev/null | grep -E "^my(ls|git|grep)" | sed 's/^/    /'

echo
echo "── unalias -m '*' clears all my* aliases ──"
unalias -m 'mygit*' 2>/dev/null
echo "  after unalias -m 'mygit*':"
alias 2>/dev/null | grep -E "^my(ls|git|grep)" | sed 's/^/    /' || echo "    (none)"

unalias mygrep 2>/dev/null

echo
echo "── setup: assoc parameter ──"
typeset -A myvars
myvars=( debug 1 verbose 0 cache 1 trace 0 stats 1 )
echo "  myvars keys: ${(@k)myvars}"

# unset with -m pattern (zsh extension).
echo
echo "── unset -m 'debug' ──"
unset -m 'debug' 2>/dev/null
echo "  Wait — unset -m removes top-level pname matches, not assoc keys"
echo "  use \${myvars[debug]} unset:"
unset 'myvars[debug]'
echo "  myvars now: ${(@k)myvars}"

echo
echo "── setup: functions ──"
my_fn1() { echo "fn1"; }
my_fn2() { echo "fn2"; }
my_helper() { echo "helper"; }
external_fn() { echo "external"; }

echo "  defined: my_fn1 my_fn2 my_helper external_fn"

echo
echo "── unfunction -m 'my_*' ──"
unfunction -m 'my_*' 2>/dev/null
echo "  remaining functions matching my_*:"
typeset -f 2>/dev/null | grep -E "^my_" | sed 's/^/    /' || echo "    (none — pattern unset worked)"
echo "  external_fn still defined?"
typeset -f external_fn 2>/dev/null | head -1

unfunction external_fn 2>/dev/null

echo
echo "── setup: command hash ──"
# Force lookups to cache.
hash -r 2>/dev/null
which ls > /dev/null
which cat > /dev/null
which echo > /dev/null

echo "  hashed:"
hash 2>/dev/null | head -5 | sed 's/^/    /'

echo
echo "── unhash -dm '*' clears directory hash ──"
hash -d zsh=/usr/local/share/zsh 2>/dev/null
hash -d home=$HOME 2>/dev/null
hash -d tmp=/tmp 2>/dev/null
echo "  named dirs:"
hash -d 2>/dev/null | sed 's/^/    /'

unhash -dm '*' 2>/dev/null
echo "  after unhash -dm '*':"
hash -d 2>/dev/null | sed 's/^/    /' || echo "    (cleared)"

echo
echo "── pattern matching in builtins (zsh extension) ──"
echo "  builtins supporting -m flag for pattern unset:"
echo "    unalias -m 'pat'  → remove matching aliases"
echo "    unhash  -m 'pat'  → remove matching command hash"
echo "    unhash -dm 'pat'  → remove matching dir hash"
echo "    unhash -fm 'pat'  → remove matching function hash"
echo "    unfunction -m 'pat' → remove matching functions"
echo "    unset -m 'pat'    → remove matching parameters"

echo
echo "── unset -m pattern on parameters ──"
typeset -g leak_var1=value1
typeset -g leak_var2=value2
typeset -g leak_var3=value3
typeset -g keep_var=keep
echo "  before: leak_var1=$leak_var1 leak_var2=$leak_var2 leak_var3=$leak_var3 keep_var=$keep_var"

unset -m 'leak_*' 2>/dev/null
echo "  after unset -m 'leak_*':"
echo "    leak_var1=[${leak_var1:-unset}]"
echo "    leak_var2=[${leak_var2:-unset}]"
echo "    leak_var3=[${leak_var3:-unset}]"
echo "    keep_var=$keep_var (preserved)"

unset keep_var

echo
echo "── verify reverse: unalias by exact name ──"
alias only_one='echo lonely'
echo "  before: $(alias only_one 2>/dev/null)"
unalias only_one
echo "  after unalias only_one: $(alias only_one 2>/dev/null || echo '(not defined)')"

# === ztest assertions ===
# unalias -m
alias xpat_a='echo a'
alias xpat_b='echo b'
alias xpat_c='echo c'
alias xkeep='echo keep'
unalias -m 'xpat_*' 2>/dev/null
zassert_eq "$(alias xpat_a 2>/dev/null)" ""    "unalias -m removed xpat_a"
zassert_eq "$(alias xpat_b 2>/dev/null)" ""    "unalias -m removed xpat_b"
zassert_contains "$(alias xkeep 2>/dev/null)" "xkeep"  "unalias -m left xkeep"
unalias xkeep 2>/dev/null
# unset -m on parameters
typeset -g zkill1=v1 zkill2=v2 zkeep=keep
unset -m 'zkill*' 2>/dev/null
zassert_eq "${zkill1:-unset}" "unset"          "unset -m removed zkill1"
zassert_eq "${zkill2:-unset}" "unset"          "unset -m removed zkill2"
zassert_eq "${zkeep:-unset}" "keep"            "unset -m left zkeep"
unset zkeep
# unfunction -m
zfn_a() { echo a; }
zfn_b() { echo b; }
zkeep_fn() { echo keep; }
unfunction -m 'zfn_*' 2>/dev/null
zfn_a_out=$(zfn_a 2>/dev/null || echo gone)
zassert_eq "$zfn_a_out" "gone"                 "unfunction -m removed zfn_a"
zkeep_out=$(zkeep_fn 2>/dev/null)
zassert_eq "$zkeep_out" "keep"                 "unfunction -m left zkeep_fn"
unfunction zkeep_fn 2>/dev/null
ztest_run
