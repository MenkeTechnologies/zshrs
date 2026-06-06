#!/usr/bin/env zshrs
# Reservoir sampling — Algorithm R: pick k uniformly from streaming N items.

# stdin → sampled k items.
reservoir() {
    local k=$1
    typeset -ga RESERVOIR
    RESERVOIR=()
    local i=0 line j
    while IFS= read -r line; do
        (( i++ ))
        if (( i <= k )); then
            RESERVOIR+=("$line")
        else
            j=$(( RANDOM % i + 1 ))
            if (( j <= k )); then
                RESERVOIR[j]="$line"
            fi
        fi
    done
}

# Sample from an in-memory array (no streaming).
reservoir_array() {
    local k=$1
    shift
    local -a items
    items=("$@")
    typeset -ga RESERVOIR
    RESERVOIR=()
    local i j
    for ((i=1; i<=${#items}; i++)); do
        if (( i <= k )); then
            RESERVOIR+=("${items[i]}")
        else
            j=$(( RANDOM % i + 1 ))
            if (( j <= k )); then
                RESERVOIR[j]="${items[i]}"
            fi
        fi
    done
}

RANDOM=42

echo "── basic reservoir sample (k=5 from 1..50) ──"
items=()
for i in {1..50}; do items+=($i); done
reservoir_array 5 "${items[@]}"
echo "  sample: ${(@n)RESERVOIR}"

echo
echo "── repeat with different seeds ──"
for seed in 1 7 42 100 999; do
    RANDOM=$seed
    reservoir_array 5 "${items[@]}"
    printf "  seed %4d: %s\n" $seed "${(@n)RESERVOIR}"
done

echo
echo "── distribution test: each element ~ k/N selected ──"
# k=10 from 1..100, run 1000 times, count selections.
typeset -A counts
for n in {1..100}; do counts[$n]=0; done
for trial in {1..200}; do
    RANDOM=$(( trial * 17 + 3 ))
    reservoir_array 10 "${items[@]}"
    for v in "${RESERVOIR[@]}"; do
        (( counts[$v]++ ))
    done
done
# Expected: each n picked 200 * 10 / 50 = 40 times for N=50
total=0
for n in {1..50}; do
    (( total += counts[$n] ))
done
echo "  trials: 200, k=10, N=50"
echo "  expected per element: $(( 200 * 10 / 50 ))"
echo "  total picks: $total"

# Show histogram (10-bucket).
echo "  histogram (buckets of 5):"
for ((bucket=0; bucket<50; bucket+=5)); do
    sum=0
    for ((j=bucket+1; j<=bucket+5; j++)); do
        (( sum += counts[$j] ))
    done
    avg=$(( sum / 5 ))
    bar=""
    bw=$(( avg / 2 ))
    for ((b=0; b<bw; b++)); do bar+="█"; done
    printf "  %2d-%2d : avg=%3d %s\n" $((bucket+1)) $((bucket+5)) $avg "$bar"
done

echo
echo "── stream sampling (k=5 from a file) ──"
tmpfile=$(mktemp)
for i in {1..30}; do
    echo "line_$i"
done > $tmpfile
RANDOM=12345
reservoir 5 < $tmpfile
echo "  sample from file:"
for line in "${RESERVOIR[@]}"; do
    echo "    $line"
done
command rm -f $tmpfile

echo
echo "── edge cases ──"
RANDOM=42
echo "  k > N (k=10, N=3): just returns all"
reservoir_array 10 a b c
echo "    sample: ${RESERVOIR[*]}"

echo "  k = N (k=5, N=5): returns all"
reservoir_array 5 a b c d e
echo "    sample: ${RESERVOIR[*]}"

echo "  k = 1 (single pick)"
reservoir_array 1 "${items[@]}"
echo "    pick: ${RESERVOIR[1]}"

echo
echo "── compare with Fisher-Yates partial shuffle (for verification) ──"
# Yates partial shuffle = randomly pick k from N.
fy_partial() {
    local k=$1
    shift
    local -a arr
    arr=("$@")
    local n=${#arr} i j tmp
    for ((i=1; i<=k; i++)); do
        j=$(( RANDOM % (n - i + 1) + i ))
        tmp=${arr[i]}
        arr[i]=${arr[j]}
        arr[j]=$tmp
    done
    echo "${arr[@]:0:$k}"
}

RANDOM=42
echo "  reservoir k=5 from 1..20:"
reservoir_array 5 "${items[@]:0:20}"
echo "    ${(@n)RESERVOIR}"
RANDOM=42
echo "  Fisher-Yates partial k=5 from 1..20:"
echo "    $(fy_partial 5 "${items[@]:0:20}" | tr ' ' '\n' | sort -n | tr '\n' ' ')"

# === ztest assertions ===
RANDOM=42
items2=()
for i in {1..50}; do items2+=($i); done
reservoir_array 5 "${items2[@]}"
zassert_eq "${#RESERVOIR}" "5"                "k=5 sample size"
reservoir_array 10 a b c
zassert_eq "${RESERVOIR[*]}" "a b c"          "k>N returns all 3"
zassert_eq "${#RESERVOIR}" "3"                "k>N size = N"
reservoir_array 5 a b c d e
zassert_eq "${RESERVOIR[*]}" "a b c d e"      "k=N returns all"
reservoir_array 1 "${items2[@]}"
zassert_eq "${#RESERVOIR}" "1"                "k=1 single pick"
ztest_run
