#!/usr/bin/env zshrs
# `hash` builtin — command cache + named directories.
# Ported from Src/builtin.c bin_hash + Src/hashnameddir.c (named dirs).

# Hash is used for both command path cache AND user-defined named dirs.

echo "── hash a command path explicitly ──"
hash mybash=/bin/sh
hash | grep -E '^mybash' || echo "(no mybash entry visible via plain hash)"

# Hash with `-d` shows the cmd-path cache.
echo "── hash table size (cmd cache) ──"
hash | wc -l

echo "── hash -r (reset/clear) ──"
hash -r
echo "after -r:"
hash | wc -l

echo "── re-populate by running things ──"
echo "(running ls, cat, head to fill cache)"
ls /dev/null >/dev/null 2>&1
cat /dev/null >/dev/null
echo "after exec:"
hash | wc -l

echo "── named directory via hash -d ──"
hash -d work=/tmp
# Named directories live in $nameddirs, not $hash ($hash is the command
# hash table and its subscripts are command names).
echo "named dir 'work' → ${nameddirs[work]:-via_param}"
# In zsh, ~work is a tilde-expansion that resolves to /tmp.
echo "expanded ~work: $(eval echo ~work)"

echo "── multiple named dirs ──"
hash -d zsh_src=/tmp
hash -d zsh_tests=/tmp
# List named dirs (zsh-specific).
echo "── all named dirs:"
hash -d | head -10

echo "── clear named dirs ──"
hash -d -r 2>&1 || true
# Or: unhash -d name
unhash -d work 2>/dev/null || true
unhash -d zsh_src 2>/dev/null || true
unhash -d zsh_tests 2>/dev/null || true
echo "after clear:"
hash -d | wc -l

# === ztest assertions ===
hash -r   # clear cmd-cache for a clean slate
hash mybash=/bin/sh
zassert_contains "$(hash)" "mybash"   "hash sets entry"
hash -r
# After -r, cmd-cache should be empty.
zassert_eq "$(hash | wc -l | tr -d ' ')" "0"  "hash -r empties cmd-cache"
# Named dirs
hash -d work=/tmp
expanded=$(eval echo "~work")
zassert_eq "$expanded" "/tmp" "named dir ~work → /tmp"
unhash -d work 2>/dev/null || true
hash -d proj1=/tmp
hash -d proj2=/tmp
zassert_contains "$(hash -d)" "proj1" "hash -d lists named dir"
zassert_contains "$(hash -d)" "proj2" "hash -d lists multiple"
unhash -d proj1 2>/dev/null || true
unhash -d proj2 2>/dev/null || true
ztest_run

