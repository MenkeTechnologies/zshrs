#!/usr/bin/env zshrs
# Continued fractions — Euclidean expansion, convergents, sqrt approximation.

# Continued fraction expansion of p/q.
cf_expand() {
    local p=$1 q=$2 cnt=0
    typeset -ga CF
    CF=()
    while (( q != 0 && cnt < 50 )); do
        local a=$(( p / q ))
        CF+=($a)
        local t=$q
        q=$(( p - a * q ))
        p=$t
        (( cnt++ ))
    done
}

# Convergents from a CF expansion. Stores in NUMS, DENS.
cf_convergents() {
    local -a a
    a=("$@")
    typeset -ga NUMS DENS
    NUMS=()
    DENS=()
    local h_prev2=0 h_prev=1
    local k_prev2=1 k_prev=0
    local i
    for ((i=1; i<=${#a}; i++)); do
        local ai=${a[i]}
        local h=$(( ai * h_prev + h_prev2 ))
        local k=$(( ai * k_prev + k_prev2 ))
        NUMS+=($h)
        DENS+=($k)
        h_prev2=$h_prev
        h_prev=$h
        k_prev2=$k_prev
        k_prev=$k
    done
}

# Approximate sqrt(n) using CF expansion.
sqrt_cf() {
    local n=$1 iters=$2
    # Initial: a0 = floor(sqrt(n))
    local a0=1
    while (( (a0+1)*(a0+1) <= n )); do (( a0++ )); done
    typeset -ga CF
    CF=($a0)
    # If perfect square, return.
    if (( a0 * a0 == n )); then return; fi
    # Iterative: m_(k+1) = d_k * a_k - m_k
    #            d_(k+1) = (n - m^2) / d
    #            a_(k+1) = (a0 + m) / d
    local m=0 d=1 a=$a0
    local i
    for ((i=0; i<iters; i++)); do
        m=$(( d * a - m ))
        d=$(( (n - m*m) / d ))
        a=$(( (a0 + m) / d ))
        CF+=($a)
    done
}

echo "── continued fraction expansion of p/q ──"
fractions=(
    "13 4"     # = 3 + 1/4
    "415 93"   # famous: pi-like CF
    "22 7"     # pi approximation
    "355 113"  # better pi
    "21 5"     # = 4 + 1/5
    "1 1"      # = 1
    "100 17"
)
for f in "${fractions[@]}"; do
    set -- ${=f}
    cf_expand $1 $2
    printf "  %5d/%-5d = [%s]\n" $1 $2 "${CF[*]}"
done

echo
echo "── convergents (best rational approximations) ──"
# CF for pi: 3, 7, 15, 1, 292, 1, 1, ...
cf_pi=(3 7 15 1 292 1 1 1 2 1 3 1 14)
cf_convergents "${cf_pi[@]}"

echo "  pi convergents (CF: ${cf_pi[*]})"
local i
for ((i=1; i<=${#NUMS}; i++)); do
    local num=${NUMS[i]}
    local den=${DENS[i]}
    # Compute decimal (×10000 for precision).
    local approx=$(( num * 10000 / den ))
    printf "    %2d: %5d/%-5d ≈ %d.%04d\n" $i $num $den $((approx/10000)) $((approx%10000))
done

echo
echo "── sqrt approximations via CF ──"
for n in 2 3 5 7 13; do
    sqrt_cf $n 10
    echo "  √$n: CF = [${CF[*]}]"
    cf_convergents "${CF[@]}"
    local last=${#NUMS}
    local final_num=${NUMS[last]}
    local final_den=${DENS[last]}
    # Approximate value.
    local approx=$(( final_num * 100000 / final_den ))
    printf "    best convergent: %d/%d ≈ %d.%05d\n" \
        $final_num $final_den $((approx/100000)) $((approx%100000))
done

echo
echo "── golden ratio φ = (1+√5)/2 ──"
echo "  CF: [1; 1, 1, 1, 1, ...]  (all 1's)"
golden_cf=(1 1 1 1 1 1 1 1 1 1 1)
cf_convergents "${golden_cf[@]}"
echo "  convergents (= consecutive Fibonacci ratios):"
for ((i=1; i<=${#NUMS}; i++)); do
    num=${NUMS[i]}
    den=${DENS[i]}
    approx=$(( num * 100000 / den ))
    printf "    %2d: %3d/%-3d ≈ %d.%05d\n" $i $num $den $((approx/100000)) $((approx%100000))
done
echo "  → converges to φ ≈ 1.61803..."

echo
echo "── periodic CF for sqrt(2) ──"
echo "  √2 = [1; 2, 2, 2, 2, ...]   (period 1)"
echo "  √3 = [1; 1, 2, 1, 2, ...]   (period 2)"
echo "  √5 = [2; 4, 4, 4, 4, ...]   (period 1)"
echo "  √7 = [2; 1, 1, 1, 4, 1, 1, 1, 4, ...] (period 4)"

echo
echo "── Lagrange theorem ──"
echo "  Periodic CF iff CF expansion of a quadratic surd"
echo "  Example: √n is periodic iff n is not a perfect square"

echo
echo "── applications ──"
echo "  best rational approximations under denominator bound"
echo "  Diophantine equations (Pell's equation)"
echo "  cryptography (Wiener's attack on RSA)"
echo "  music theory (equal temperament approximations)"
echo "  calendar systems (year length)"

# === ztest assertions ===
cf_expand 13 4
zassert_eq "${CF[*]}" "3 4"           "cf 13/4 = [3;4]"
cf_expand 22 7
zassert_eq "${CF[*]}" "3 7"           "cf 22/7 = [3;7]"
cf_expand 21 5
zassert_eq "${CF[*]}" "4 5"           "cf 21/5 = [4;5]"
cf_expand 1 1
zassert_eq "${CF[*]}" "1"             "cf 1/1 = [1]"
cf_convergents 3 7 15 1
zassert_eq "${NUMS[*]}" "3 22 333 355" "pi convergent numerators"
zassert_eq "${DENS[*]}" "1 7 106 113"  "pi convergent denominators"
sqrt_cf 4 5
zassert_eq "${CF[*]}" "2"             "sqrt 4 perfect square returns [2]"
sqrt_cf 2 4
zassert_eq "${CF[*]}" "1 2 2 2 2"     "sqrt 2 CF = [1;2,2,2,2]"
ztest_run
