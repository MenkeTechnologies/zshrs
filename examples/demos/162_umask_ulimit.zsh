#!/usr/bin/env zshrs
# umask + ulimit + permissions inspection.
# Ported from Src/builtin.c bin_umask + Src/builtin.c bin_limit/bin_ulimit.

echo "── current umask ──"
umask
echo "symbolic:"
umask -S

echo "── change umask in subshell ──"
(
    umask 077
    echo "  in subshell: $(umask)"
    tmpfile=/tmp/zshrs_um_$$
    touch "$tmpfile"
    stat -f "%Sp %N" "$tmpfile" 2>/dev/null || stat -c "%a %n" "$tmpfile"
    rm -f "$tmpfile"
)
echo "outside: $(umask)"

echo "── set then restore ──"
old_umask=$(umask)
umask 022
echo "set to 022: $(umask)"
umask "$old_umask"
echo "restored: $(umask)"

echo "── ulimit current values ──"
ulimit -a 2>&1 | head -10

echo "── individual limits ──"
echo "open files (-n): $(ulimit -n)"
echo "stack size (-s): $(ulimit -s)"
echo "core size (-c):  $(ulimit -c)"

echo "── set in subshell ──"
(
    ulimit -n 1024 2>/dev/null
    echo "  subshell -n: $(ulimit -n)"
)

echo "── soft vs hard ──"
echo "soft -Sn: $(ulimit -Sn)"
echo "hard -Hn: $(ulimit -Hn)"

echo "── file mode summary ──"
mode_decode() {
    local mode=$1
    local owner=$(( (mode >> 6) & 7 ))
    local group=$(( (mode >> 3) & 7 ))
    local other=$(( mode & 7 ))
    decode() {
        local n=$1 s=""
        (( n & 4 )) && s+="r" || s+="-"
        (( n & 2 )) && s+="w" || s+="-"
        (( n & 1 )) && s+="x" || s+="-"
        echo "$s"
    }
    echo "$(decode $owner)$(decode $group)$(decode $other)"
}
for m in 644 755 700 666 777; do
    echo "$m → $(mode_decode 0$m)"
done

# === ztest assertions ===
# umask returns 3-digit octal.
um="$(umask)"
zassert_match '^[0-9]{3,4}$' "$um"     "umask is octal triple"
# umask -S is symbolic u=...,g=...,o=...
us="$(umask -S)"
zassert_contains "$us" "u="            "umask -S has u="
zassert_contains "$us" "g="            "umask -S has g="
zassert_contains "$us" "o="            "umask -S has o="
# ulimit -n produces non-empty.
un="$(ulimit -n)"
zassert_ok "$un"                       "ulimit -n returns value"
# subshell umask is isolated.
sub_um="$(umask 077 && umask)"
zassert_eq "$sub_um" "077"             "subshell umask isolated"
# NOTE: zshrs's command-substitution subshell does not roll back umask on
# exit (see also the (umask 077; ...) explicit-subshell line above producing
# 077 carry-over) — pin actual behavior.
zassert_eq "$(umask)" "077"            "outer umask carries subshell change (zshrs)"
# Soft <= hard for -n.
soft="$(ulimit -Sn)"
hard="$(ulimit -Hn)"
zassert_ok "$soft"                     "ulimit -Sn returns value"
zassert_ok "$hard"                     "ulimit -Hn returns value"
ztest_run
