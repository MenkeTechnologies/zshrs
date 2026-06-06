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

# === ztest assertions ===
# Many %-escapes are environment-dependent (%n=user, %M=hostname, %d=cwd) —
# assert on the deterministic ones: conditionals, literal %%, %{...%} passthrough.
true
ok_out=$(print -P "after true: %(?.OK.FAIL)")
zassert_eq "$ok_out"    "after true: OK"     "%(?.) picks .true on success"
false
fail_out=$(print -P "after false: %(?.OK.FAIL)")
zassert_eq "$fail_out"  "after false: FAIL"  "%(?.) picks .false on error"
pct=$(print -P "literal: %%")
zassert_eq "$pct"       "literal: %"         "%% emits literal %"
litpass=$(print -P "before %{<lit>%} after")
zassert_eq "$litpass"   "before <lit> after" "%{...%} literal pass-through"
red_raw=$(print -P "%F{red}red%f")
zassert_contains "$red_raw" "red"            "%F{red} contains the text"
user=$(print -P "%n")
zassert_ok "$user" "%n produces a username"
ztest_run

