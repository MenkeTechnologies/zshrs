#!/usr/bin/env zshrs
# Progress-bar / spinner patterns.

draw_bar() {
    local pct=$1 width=${2:-40}
    local filled=$(( pct * width / 100 ))
    local empty=$(( width - filled ))
    local bar="" i
    for ((i=0; i<filled; i++)); do bar+="█"; done
    for ((i=0; i<empty; i++)); do bar+="░"; done
    printf "[%s] %3d%%\n" "$bar" $pct
}

echo "── progress steps ──"
for pct in 0 10 25 40 55 70 80 90 95 100; do
    draw_bar $pct
done

echo
echo "── multi-stage ──"
stages=(init download verify install configure)
n=${#stages[@]}
for ((i=0; i<n; i++)); do
    pct=$(( (i+1) * 100 / n ))
    draw_bar $pct
    echo "  → stage: ${stages[i+1]}"
done

echo
echo "── spinner frames ──"
frames=('|' '/' '-' '\\')
for f in {0..11}; do
    idx=$(( f % 4 + 1 ))
    printf "  %s frame=%d\n" "${frames[idx]}" $f
done

echo
echo "── braille spinner ──"
braille=('⠋' '⠙' '⠹' '⠸' '⠼' '⠴' '⠦' '⠧')
for f in {0..15}; do
    idx=$(( f % 8 + 1 ))
    printf "  %s\n" "${braille[idx]}"
done

# === ztest assertions ===
zassert_eq "$(draw_bar 0 10)"   "[░░░░░░░░░░]   0%"  "0% bar width=10"
zassert_eq "$(draw_bar 50 10)"  "[█████░░░░░]  50%"  "50% bar width=10"
zassert_eq "$(draw_bar 100 10)" "[██████████] 100%"  "100% bar width=10"
zassert_eq "${#stages[@]}" 5     "5 stages defined"
zassert_eq "${stages[1]}"  "init"      "first stage"
zassert_eq "${stages[5]}"  "configure" "last stage"
zassert_eq "${#frames[@]}" 4     "4 ASCII spinner frames"
zassert_eq "${#braille[@]}" 8    "8 braille frames"
zassert_eq "${frames[1]}"  "|"   "first ASCII frame"
zassert_eq "${braille[1]}" "⠋"   "first braille frame"
ztest_run
