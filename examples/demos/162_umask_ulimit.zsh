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
