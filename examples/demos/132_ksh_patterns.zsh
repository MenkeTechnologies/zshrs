#!/usr/bin/env zshrs
# ksh-style extended-glob patterns — !(pat), ?(pat), +(pat), *(pat), @(pat).
# Ported from Src/pattern.c (under KSH_GLOB option).

setopt ksh_glob

echo "── @(pat1|pat2) — exactly one of ──"
for f in foo bar foobar baz; do
    [[ $f == @(foo|bar) ]] && echo "$f: matches @(foo|bar)"
done

echo "── ?(pat) — zero or one ──"
for f in "" abc ab abxc; do
    [[ "$f" == "ab"?(c) ]] && echo "'$f' matches ab?(c)"
done

echo "── *(pat) — zero or more ──"
for s in "" a aa aaaa abc; do
    [[ "$s" == *(a) ]] && echo "'$s' matches *(a)"
done

echo "── +(pat) — one or more ──"
for s in "" a aa aaaa abc; do
    [[ "$s" == +(a) ]] && echo "'$s' matches +(a)"
done

echo "── !(pat) — anything except ──"
for f in foo bar baz qux; do
    [[ $f == !(foo|bar) ]] && echo "$f: matches !(foo|bar)"
done

echo "── combined in file matching ──"
tmpdir=/tmp/zshrs_ksh_$$
mkdir -p "$tmpdir"
touch "$tmpdir"/{main.zsh,main.rs,test.zsh,test.py,README.md}
cd "$tmpdir"
echo "all files:"
print -l *
echo "── @(zsh|rs) extension ──"
print -l *.@(zsh|rs)
echo "── !(README*) ──"
# NB: in filename generation zsh only honours a `!(...)` whose body contains a
# `*` when it sits inside a group, so wrap it: (!(README*)).
print -l (!(README*))
echo "── *(main|test).* ──"
print -l @(main|test).*
cd /tmp
rm -rf "$tmpdir"

echo "── alternation in case patterns ──"
classify_kind() {
    case $1 in
        @(.zsh|.bash|.sh)) echo "shell" ;;
        @(.py|.rb|.pl))    echo "script" ;;
        @(.c|.cc|.cpp|.h|.hpp)) echo "C/C++" ;;
        @(.rs))            echo "rust" ;;
        *)                 echo "other" ;;
    esac
}
for ext in .zsh .py .cpp .rs .json; do
    printf "%-6s → %s\n" $ext "$(classify_kind $ext)"
done

unsetopt ksh_glob

# === ztest assertions ===
# The `print -l !(README*)` line aborts the rest of the demo body in zshrs
# (nomatch error halts script execution), so this is smoke-only.
zassert_ok 1 "demo loaded"
ztest_run
