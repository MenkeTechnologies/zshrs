#!/usr/bin/env zshrs
# print -P prompt escape sequences.
# Ported from zsh's prompt.c (Src/prompt.c putpromptchar dispatch).

echo "── %n user ──"
print -P "%n"

echo "── %d / %~ cwd ──"
print -P "%d"          # literal cwd
print -P "%~"          # with ~ for $HOME

echo "── %M long hostname ──"
print -P "%M"

echo "── conditional %(?true.false) on \$? ──"
true
print -P "after true: %(?.OK.FAIL)"
false
print -P "after false: %(?.OK.FAIL)"

echo "── %F{color} foreground / %f reset ──"
print -P "%F{red}red%f %F{green}green%f %F{blue}blue%f"

echo "── %B bold / %b unbold ──"
print -P "%Bbold%b vs normal"

echo "── %U underline / %u ununderline ──"
print -P "%Uunderlined%u text"

echo "── %{...%} literal pass-through (skip width) ──"
print -P "before %{<lit>%} after"

echo "── %% literal percent ──"
print -P "literal: %%"
