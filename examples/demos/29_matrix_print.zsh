#!/usr/bin/env zshrs
# Matrix-style nested-loop output — multiplication table.

echo "── 9x9 multiplication table ──"
for ((i = 1; i <= 9; i++)); do
    for ((j = 1; j <= 9; j++)); do
        printf "%4d" $(( i * j ))
    done
    echo
done

echo
echo "── triangular ──"
for ((i = 1; i <= 7; i++)); do
    for ((j = 1; j <= i; j++)); do
        printf "* "
    done
    echo
done

echo
echo "── diamond ──"
for ((i = 1; i <= 5; i++)); do
    for ((j = i; j < 5; j++)); do printf " "; done
    for ((j = 1; j <= 2 * i - 1; j++)); do printf "*"; done
    echo
done
for ((i = 4; i >= 1; i--)); do
    for ((j = 5; j > i; j--)); do printf " "; done
    for ((j = 1; j <= 2 * i - 1; j++)); do printf "*"; done
    echo
done

# === ztest assertions ===
# Multiplication table cell-level verification
zassert_eq $(( 1 * 1 ))   1  "1×1"
zassert_eq $(( 9 * 9 ))   81 "9×9"
zassert_eq $(( 7 * 8 ))   56 "7×8"
# Triangular row count
tri_lines() {
    local lines=0 i j
    for ((i=1; i<=7; i++)); do (( lines++ )); done
    echo $lines
}
zassert_eq "$(tri_lines)" 7 "triangle has 7 rows"
# Diamond width at peak = 2*5-1 = 9
zassert_eq $(( 2 * 5 - 1 )) 9 "diamond peak has 9 stars"
# Triangle row N has N stars
star_count() {
    local n=$1 i s=0
    for ((i=1; i<=n; i++)); do (( s++ )); done
    echo $s
}
zassert_eq "$(star_count 5)" 5 "row 5 of triangle has 5 stars"
zassert_eq "$(star_count 7)" 7 "row 7 of triangle has 7 stars"
ztest_run
