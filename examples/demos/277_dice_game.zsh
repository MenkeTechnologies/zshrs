#!/usr/bin/env zshrs
# Dice probability — roll N dice, count outcomes, compute distributions.

# Roll N d-sided dice, return sum.
roll() {
    local n=$1 sides=$2 total=0 i
    for ((i=0; i<n; i++)); do
        (( total += RANDOM % sides + 1 ))
    done
    echo $total
}

# Roll N d-sided dice K times, return histogram.
RANDOM=42

echo "── single d6 distribution (10000 rolls) ──"
typeset -A hist
for ((i=0; i<10000; i++)); do
    r=$(( RANDOM % 6 + 1 ))
    (( hist[$r]++ ))
done
for k in 1 2 3 4 5 6; do
    c=${hist[$k]}
    pct=$(( c * 1000 / 10000 ))
    bar=""
    bw=$(( pct / 5 ))
    for ((b=0; b<bw; b++)); do bar+="█"; done
    printf "  %d: %4d (%d.%d%%) %s\n" $k $c $((pct/10)) $((pct%10)) "$bar"
done

echo
echo "── 2d6 sum distribution (10000 rolls) ──"
hist=()
for ((i=0; i<10000; i++)); do
    r=$(( (RANDOM % 6 + 1) + (RANDOM % 6 + 1) ))
    (( hist[$r]++ ))
done
for k in {2..12}; do
    c=${hist[$k]:-0}
    pct=$(( c * 1000 / 10000 ))
    bar=""
    bw=$(( pct / 5 ))
    for ((b=0; b<bw; b++)); do bar+="█"; done
    printf "  %2d: %4d (%2d.%d%%) %s\n" $k $c $((pct/10)) $((pct%10)) "$bar"
done

echo
echo "── theoretical 2d6 (uniform-uniform conv) ──"
echo "  expected counts × 10000/36:"
expected=(0 0 1 2 3 4 5 6 5 4 3 2 1)
for k in {2..12}; do
    e=${expected[k+1]}
    pct=$(( e * 1000 / 36 ))
    printf "  %2d: %d/36 = %2d.%d%%\n" $k $e $((pct/10)) $((pct%10))
done

echo
echo "── chi-square test (lower = better fit) ──"
chi=0
for k in {2..12}; do
    obs=${hist[$k]:-0}
    e=${expected[k+1]}
    exp=$(( e * 10000 / 36 ))
    if (( exp > 0 )); then
        diff=$(( obs - exp ))
        (( chi += diff * diff / exp ))
    fi
done
echo "  χ² = $chi  (critical val for 10 dof @ p=0.05 is 18.31)"

echo
echo "── d20 rolls (single-die fairness) ──"
hist=()
for ((i=0; i<2000; i++)); do
    r=$(( RANDOM % 20 + 1 ))
    (( hist[$r]++ ))
done
echo "  expected per face: 100"
min=99999; max=0; min_k=0; max_k=0
for k in {1..20}; do
    c=${hist[$k]:-0}
    if (( c < min )); then min=$c; min_k=$k; fi
    if (( c > max )); then max=$c; max_k=$k; fi
done
echo "  min face: $min_k ($min rolls)"
echo "  max face: $max_k ($max rolls)"
echo "  range: $(( max - min ))"

echo
echo "── 3d6 ability scores (D&D-style) ──"
echo "  6 ability rolls:"
for stat in STR DEX CON INT WIS CHA; do
    r=$(roll 3 6)
    printf "  %s: %d\n" $stat $r
done

echo
echo "── pseudo-Yahtzee: 5d6 patterns ──"
roll_yahtzee() {
    typeset -A face_count
    face_count=()
    local i v
    for ((i=0; i<5; i++)); do
        v=$(( RANDOM % 6 + 1 ))
        (( face_count[$v]++ ))
    done
    # Determine pattern.
    local pattern="bust"
    local max_count=0
    local has_pair=0
    local n_distinct=${#face_count}
    for k in "${(@k)face_count}"; do
        (( face_count[$k] > max_count )) && max_count=${face_count[$k]}
        (( face_count[$k] == 2 )) && has_pair=1
    done
    case "$max_count $n_distinct" in
        "5 1") pattern="YAHTZEE!" ;;
        "4 2") pattern="four of a kind" ;;
        "3 2") pattern="full house" ;;
        "3 3") pattern="three of a kind" ;;
        "2 3") pattern="two pair" ;;
        "2 4") pattern="one pair" ;;
        *)     pattern="all different" ;;
    esac
    echo "$pattern"
}
typeset -A pattern_count
for ((i=0; i<200; i++)); do
    p=$(roll_yahtzee)
    (( pattern_count[$p]++ ))
done
for k in "${(@ko)pattern_count}"; do
    printf "  %-20s × %d\n" "$k" "${pattern_count[$k]}"
done

# === ztest assertions ===
# RANDOM is seeded — exercise structural invariants that don't depend on PRNG.
# Total rolls per phase is exact.
sum_hist() {
    local total=0 k
    for k in "${(@k)pattern_count}"; do (( total += pattern_count[$k] )); done
    echo $total
}
zassert_eq "$(sum_hist)" 200 "pseudo-Yahtzee phase rolled 200 hands"
# All roll values are in [n, n*sides] range
v=$(roll 3 6)
zassert_ge "$v" 3   "3d6 >= 3"
zassert_le "$v" 18  "3d6 <= 18"
v=$(roll 1 20)
zassert_ge "$v" 1   "1d20 >= 1"
zassert_le "$v" 20  "1d20 <= 20"
v=$(roll 5 6)
zassert_ge "$v" 5   "5d6 >= 5"
zassert_le "$v" 30  "5d6 <= 30"
# χ² fit for fair d6 under default χ²₁₀,0.05 ≈ 18.31
zassert_lt "$chi" 19 "2d6 χ² fits under threshold"
# d20 range positive (basic sanity)
zassert_gt "$(( max - min ))" 0 "d20 distribution non-degenerate"
zassert_ge "$min_k" 1   "d20 min-face index in range"
zassert_le "$max_k" 20  "d20 max-face index in range"
ztest_run
