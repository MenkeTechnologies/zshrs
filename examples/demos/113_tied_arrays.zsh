#!/usr/bin/env zshrs
# Tied colon-arrays via typeset -T.
# Ported from Src/params.c PM_TIED + Src/Modules/parameter.c arrfixenv.

echo "── tie PATHARR scalar to PATHARR_PATHS array ──"
# Synthesize a fresh tied pair.
typeset -T PATHARR PATHARR_PATHS=/usr/bin:/usr/local/bin:/opt/bin
echo "PATHARR (scalar):    $PATHARR"
echo "PATHARR_PATHS (arr):"
print -l $PATHARR_PATHS

echo "── update via array changes the scalar ──"
PATHARR_PATHS+=/new/dir
echo "scalar after append: $PATHARR"
print -l $PATHARR_PATHS

echo "── update via scalar changes the array ──"
PATHARR=a:b:c:d
echo "array after scalar reassign:"
print -l $PATHARR_PATHS

echo "── tied custom separator ──"
typeset -T LINES LINES_ARR=$'one\ntwo\nthree' $'\n'
echo "scalar with newline-sep tie:"
echo "$LINES"
echo "array form:"
for x in $LINES_ARR; do echo "  - $x"; done

echo "── cleanup ──"
unset PATHARR PATHARR_PATHS LINES LINES_ARR 2>/dev/null

echo "── compare with manual sync (untied) ──"
manual_str=alpha:beta:gamma
manual_arr=( ${(s/:/)manual_str} )
echo "manual_str=$manual_str"
echo "manual_arr=${manual_arr[@]}"
# Changing one doesn't change the other:
manual_arr+=(delta)
echo "after manual_arr+=: manual_str still=$manual_str"

echo "── PATH itself is the classic tied pair ──"
echo "first 3 path[]:"
print -l $path[1] $path[2] $path[3]
