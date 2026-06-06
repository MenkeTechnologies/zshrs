#!/usr/bin/env zshrs
# Self-introspecting capabilities report — what does THIS zshrs support?

echo "── version + binary ──"
echo "ZSH_VERSION: ${ZSH_VERSION:-not-set}"
echo "0 (argv0):   $0"
echo "PID:         $$"

echo "── available builtins ──"
echo "builtin count: $(builtin -l 2>/dev/null | wc -l) (or use $+builtins[name])"
for b in echo print cd pwd typeset declare local export readonly eval source \
         hash unset alias unalias unfunction history fc trap setopt unsetopt \
         autoload zmodload zstyle zparseopts bindkey zle; do
    if (( $+builtins[$b] )); then
        printf "  ✓ %s\n" $b
    fi
done | head -20

echo "── coreutils-as-builtins (zshrs extension) ──"
for c in cat head tail wc sort uniq cut tr seq rev tee date hostname mktemp; do
    if (( $+builtins[$c] )); then
        printf "  ✓ %s (no-fork)\n" $c
    elif command -v $c >/dev/null 2>&1; then
        printf "  - %s (forks)\n" $c
    else
        printf "  ✗ %s (missing)\n" $c
    fi
done

echo "── modules ──"
for m in zsh/datetime zsh/mathfunc zsh/regex zsh/stat zsh/zselect; do
    if zmodload $m 2>/dev/null; then
        echo "  ✓ $m"
    else
        echo "  ✗ $m"
    fi
done

echo "── option support ──"
test_opt() {
    local opt=$1
    if setopt $opt 2>/dev/null; then
        unsetopt $opt
        printf "  ✓ %s\n" $opt
    else
        printf "  ✗ %s\n" $opt
    fi
}
for o in extendedglob nullglob globsubst kshtypeset pipefail xtrace verbose; do
    test_opt $o
done

echo "── parameter feature support ──"
typeset -A m
if [[ -n ${m+x} ]]; then echo "  ✓ typeset -A"; fi
typeset -i n=42
if [[ $n -eq 42 ]]; then echo "  ✓ typeset -i"; fi
typeset -F 2 f=3.14
if [[ $f == 3.14 ]]; then echo "  ✓ typeset -F"; fi
arr=(a b c)
if [[ ${#arr[@]} -eq 3 ]]; then echo "  ✓ array literal"; fi
typeset -T TAB TAB_ARR=":a:b:c" :
if [[ -n ${TAB+x} ]]; then echo "  ✓ typeset -T (tied)"; fi

echo "── summary ──"
echo "this zshrs supports: builtins + coreutils + most modules + zsh-isms"

# === ztest assertions ===
# Core builtins must exist.
zassert_ok "$+builtins[echo]"     "echo is a builtin"
zassert_ok "$+builtins[typeset]"  "typeset is a builtin"
zassert_ok "$+builtins[local]"    "local is a builtin"
zassert_ok "$+builtins[setopt]"   "setopt is a builtin"
zassert_ok "$+builtins[autoload]" "autoload is a builtin"
# typeset -A creates assoc.
typeset -A test_m=(a 1 b 2)
zassert_eq "${test_m[a]}" "1"  "typeset -A stores value"
zassert_eq "${test_m[b]}" "2"  "typeset -A second value"
# typeset -i creates integer.
typeset -i test_n=42
zassert_eq "$test_n" "42"      "typeset -i stores int"
# array literal works.
test_arr=(a b c)
zassert_eq "${#test_arr[@]}" "3" "array literal length"
# PID is positive.
zassert_gt "$$" 0              "$$ PID positive"
ztest_run
