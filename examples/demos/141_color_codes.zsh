#!/usr/bin/env zshrs
# ANSI color codes — manual + print -P + autoload colors.
# Ported from Src/prompt.c (color escape handling) + Src/Modules/terminfo.c.

echo "── manual ANSI escapes ──"
printf '\033[31m%s\033[0m\n' "red"
printf '\033[32m%s\033[0m\n' "green"
printf '\033[33m%s\033[0m\n' "yellow"
printf '\033[34m%s\033[0m\n' "blue"
printf '\033[35m%s\033[0m\n' "magenta"
printf '\033[36m%s\033[0m\n' "cyan"
printf '\033[1m%s\033[0m\n' "bold"

echo "── via print -P ──"
print -P "%F{red}red%f %F{green}green%f %F{blue}blue%f"
print -P "%B bold %b normal"
print -P "%U underline %u normal"

echo "── color via autoload ──"
autoload -U colors 2>/dev/null && colors 2>/dev/null
if [[ -n $color ]]; then
    echo "${color[red]}red${color[reset]} ${color[green]}green${color[reset]}"
elif [[ -n $fg[red] ]]; then
    echo "via fg/bg: $fg[red]red$reset_color"
fi

echo "── color helper fn ──"
red() { print -P "%F{red}$*%f"; }
green() { print -P "%F{green}$*%f"; }
yellow() { print -P "%F{yellow}$*%f"; }
bold() { print -P "%B$*%b"; }

red "this is red"
green "this is green"
yellow "this is yellow"
bold "this is bold"

echo "── 256-color palette ──"
for c in 196 202 226 46 33 92 201; do
    print -P "%F{$c}color $c%f"
done

echo "── status indicators ──"
ok()    { print -P "%F{green}✓%f $*"; }
fail()  { print -P "%F{red}✗%f $*"; }
warn()  { print -P "%F{yellow}⚠%f $*"; }
info()  { print -P "%F{blue}ℹ%f $*"; }

ok    "tests passed"
fail  "deploy failed"
warn  "deprecated API"
info  "build started"

echo "── 24-color colorbar via printf ──"
for c in {16..51}; do
    printf '\033[38;5;%dm█' $c
done
printf '\033[0m\n'

# === ztest assertions ===
# print -P emits raw ANSI; assert on the encoded escape bytes.
zassert_contains "$(red foo)"    $'\e[31m'    "red helper emits SGR 31"
zassert_contains "$(green bar)"  $'\e[32m'    "green helper emits SGR 32"
zassert_contains "$(yellow baz)" $'\e[33m'    "yellow helper emits SGR 33"
zassert_contains "$(bold zoo)"   $'\e[1m'     "bold helper emits SGR 1"
zassert_contains "$(red foo)"    "foo"        "red helper preserves content"
# 256-color codes have %F{N} pattern -> 38;5;N
zassert_contains "$(print -P '%F{196}x%f')" "38;5;196" "256-color SGR encoded"
# Status indicators
zassert_contains "$(ok done)"   "✓"  "ok shows checkmark"
zassert_contains "$(fail err)"  "✗"  "fail shows cross"
zassert_contains "$(warn wd)"   "⚠"  "warn shows triangle"
zassert_contains "$(info i)"    "ℹ"  "info shows i-glyph"
ztest_run
