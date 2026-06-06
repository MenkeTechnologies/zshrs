#!/usr/bin/env zshrs
# History-style filename modifiers — :h :t :r :e :a :A and :s/old/new/.
# Ported from zsh's hist.c remtpath/remtext/rembutext/remlpaths and
# Src/utils.c xsymlinks (for :A). Same modifiers also fire on parameter
# expansion (`${var:h}`) and history-word references.

p=/usr/local/share/zsh/site-functions/_completion.gz

echo "── path modifiers (zsh hist.c) ──"
echo "path : $p"
echo ":h   : ${p:h}        # dirname / head"
echo ":t   : ${p:t}        # basename / tail"
echo ":r   : ${p:r}        # root (strip last .ext)"
echo ":e   : ${p:e}        # extension"
echo ":h:h : ${p:h:h}      # chained — parent of dirname"
echo ":r:e : ${p:r:e}      # last ext after first ext stripped"

# :s/old/new/ — substitution modifier (single replacement).
echo "── substitution :s ──"
echo "$p:s/share/SHARE/      → ${p:s/share/SHARE/}"
echo "$p:gs/zsh/ZSH/        → ${p:gs/zsh/ZSH/}    # g = global"

# :a is absolute path normalization (resolves . and ..); :A also
# resolves symlinks (calls realpath internally per Src/utils.c:xsymlinks).
echo "── absolute / real ──"
rel="./foo/../bar/./baz"
echo "rel  : $rel"
echo ":a   : ${rel:a}     # collapsed-absolute"

echo "── chained on arrays ──"
files=(/etc/passwd /usr/bin/zsh /tmp/test.log)
print -l ${files:t}     # all tail
echo "---"
print -l ${files:h}     # all dirname

# === ztest assertions ===
zassert_eq "${p:h}" "/usr/local/share/zsh/site-functions"            ":h dirname"
zassert_eq "${p:t}" "_completion.gz"                                  ":t basename"
zassert_eq "${p:r}" "/usr/local/share/zsh/site-functions/_completion" ":r root"
zassert_eq "${p:e}" "gz"                                              ":e extension"
zassert_eq "${p:h:h}" "/usr/local/share/zsh"                          ":h:h chained"
zassert_eq "${p:s/share/SHARE/}" "/usr/local/SHARE/zsh/site-functions/_completion.gz" ":s substitution"
zassert_eq "${files[1]:t}" "passwd"   "array element :t"
zassert_eq "${files[2]:t}" "zsh"      "array element :t #2"
zassert_eq "${files[3]:h}" "/tmp"     "array element :h"
ztest_run
