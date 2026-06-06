#!/usr/bin/env zshrs
# ASCII bar charts — horizontal + vertical from data.

zmodload zsh/datetime 2>/dev/null

# Horizontal bars: label + value + bar.
hbar() {
    local label=$1 val=$2 max=$3
    local width=40
    local bar_len=$(( val * width / max ))
    local bar=""
    local i
    for ((i=0; i<bar_len; i++)); do bar+="█"; done
    printf "%-10s | %s (%d)\n" "$label" "$bar" $val
}

echo "── horizontal bars ──"
typeset -A SALES
SALES[Jan]=50
SALES[Feb]=80
SALES[Mar]=120
SALES[Apr]=95
SALES[May]=160
SALES[Jun]=110
SALES[Jul]=200
SALES[Aug]=190
SALES[Sep]=150
SALES[Oct]=130
SALES[Nov]=170
SALES[Dec]=220

months=(Jan Feb Mar Apr May Jun Jul Aug Sep Oct Nov Dec)
max=0
for m in $months; do
    (( SALES[$m] > max )) && max=${SALES[$m]}
done

for m in $months; do
    hbar $m ${SALES[$m]} $max
done

# Vertical chart: print height per column from values.
vbar() {
    local label=$1 val=$2 max=$3
    local i
    # Print "█" stack from top to bottom in terminal: we just emit
    # height value and let outer loop arrange.
    for ((i=max; i>0; i--)); do
        if (( val >= i )); then
            printf "%s" "█"
        else
            printf "%s" " "
        fi
    done
}

echo
echo "── vertical chart (top→bottom) ──"
height=10
for ((row=height; row>=1; row--)); do
    for m in $months; do
        v=${SALES[$m]}
        scaled=$(( v * height / max ))
        if (( scaled >= row )); then
            printf "%-4s" "█"
        else
            printf "%-4s" " "
        fi
    done
    echo
done
# Bottom labels.
for m in $months; do printf "%-4s" "$m"; done
echo

echo
echo "── histogram from raw data ──"
data=(5 1 2 4 2 5 4 5 2 1 2 5 4 4 5 2)
typeset -A bucket
for v in "${data[@]}"; do
    (( bucket[$v]++ ))
done
echo "value | count | bar"
for k in ${(ko)bucket}; do
    count=${bucket[$k]}
    bar=""
    for ((i=0; i<count; i++)); do bar+="*"; done
    printf "  %3d | %4d | %s\n" $k $count "$bar"
done

# === ztest assertions ===
# Max from SALES is Dec=220.
zassert_eq "$max" 220                        "max month value is 220 (Dec)"
zassert_eq "${SALES[Jul]}" 200               "Jul = 200"
zassert_eq "${SALES[Jan]}" 50                "Jan = 50"
zassert_eq "${#months[@]}" 12                "12 months"
# Histogram counts from data=(5 1 2 4 2 5 4 5 2 1 2 5 4 4 5 2).
# Manually: 1→2 2→5 4→4 5→5
zassert_eq "${bucket[1]}" 2                  "value 1 appears 2 times"
zassert_eq "${bucket[2]}" 5                  "value 2 appears 5 times"
zassert_eq "${bucket[4]}" 4                  "value 4 appears 4 times"
zassert_eq "${bucket[5]}" 5                  "value 5 appears 5 times"
# Total samples
total_samples=0
for k in ${(k)bucket}; do (( total_samples += bucket[$k] )); done
zassert_eq "$total_samples" 16               "16 total samples"
ztest_run
