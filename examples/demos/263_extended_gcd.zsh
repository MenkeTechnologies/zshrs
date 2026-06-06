#!/usr/bin/env zshrs
# Extended Euclidean — gcd(a,b) = a*x + b*y; modular inverse.

# Iterative ext-gcd: returns "gcd x y" in a global tuple.
typeset -gA EGCD
ext_gcd() {
    local a=$1 b=$2
    local old_r=$a r=$b
    local old_s=1 s=0
    local old_t=0 t=1
    local q tmp
    while (( r != 0 )); do
        q=$(( old_r / r ))
        tmp=$r;     r=$((     old_r - q * r ));     old_r=$tmp
        tmp=$s;     s=$((     old_s - q * s ));     old_s=$tmp
        tmp=$t;     t=$((     old_t - q * t ));     old_t=$tmp
    done
    EGCD[g]=$old_r
    EGCD[x]=$old_s
    EGCD[y]=$old_t
}

mod_inv() {
    local a=$1 m=$2
    ext_gcd $a $m
    if (( EGCD[g] != 1 )); then echo "no-inverse"; return 1; fi
    local x=${EGCD[x]}
    echo $(( (x % m + m) % m ))
}

echo "── extended GCD ──"
pairs=("48 18" "100 75" "270 192" "12345 67890" "17 13" "1 1" "0 5" "99 81")
for p in "${pairs[@]}"; do
    set -- ${=p}
    a=$1; b=$2
    ext_gcd $a $b
    g=${EGCD[g]}
    x=${EGCD[x]}
    y=${EGCD[y]}
    check=$(( a * x + b * y ))
    printf "  gcd(%5d, %5d) = %d ; %d×%d + %d×%d = %d %s\n" \
        $a $b $g $a $x $b $y $check "$([[ $check == $g ]] && echo ✓ || echo ✗)"
done

echo
echo "── modular inverse a⁻¹ mod m ──"
inv_tests=("3 11" "7 26" "10 17" "13 1000000007" "15 100" "6 9")
for t in "${inv_tests[@]}"; do
    set -- ${=t}
    a=$1; m=$2
    inv=$(mod_inv $a $m)
    if [[ $inv == no-inverse ]]; then
        printf "  %d⁻¹ mod %d : DNE (gcd ≠ 1)\n" $a $m
    else
        check=$(( a * inv % m ))
        printf "  %d⁻¹ mod %d = %d   ; check %d×%d mod %d = %d %s\n" \
            $a $m $inv $a $inv $m $check "$([[ $check == 1 ]] && echo ✓ || echo ✗)"
    fi
done

echo
echo "── coprimality table (mod 12) ──"
echo "  φ(12) = a∈[1..11] with gcd(a,12)=1"
out=""
for a in {1..11}; do
    ext_gcd $a 12
    if (( EGCD[g] == 1 )); then out+="$a "; fi
done
echo "  $out"

echo
echo "── RSA-toy demo ──"
# Two small primes.
p=11; q=13
n=$(( p * q ))
phi=$(( (p-1) * (q-1) ))
e=7
d=$(mod_inv $e $phi)
echo "  p=$p q=$q n=pq=$n φ(n)=$phi"
echo "  e=$e d=$d (e·d mod φ = $(( e * d % phi )))"
msg=42
c=$(modpow_stub() { local r=1 b=$1 ex=$2 m=$3; (( b %= m )); while (( ex > 0 )); do (( ex & 1 )) && (( r = r * b % m )); (( ex >>= 1 )); (( b = b * b % m )); done; echo $r; }; modpow_stub $msg $e $n)
back=$(modpow_stub() { local r=1 b=$1 ex=$2 m=$3; (( b %= m )); while (( ex > 0 )); do (( ex & 1 )) && (( r = r * b % m )); (( ex >>= 1 )); (( b = b * b % m )); done; echo $r; }; modpow_stub $c $d $n)
echo "  msg=$msg → enc=msg^e mod n = $c → dec=enc^d mod n = $back"

# === ztest assertions ===
ext_gcd 48 18
zassert_eq "${EGCD[g]}" "6"  "gcd(48,18) = 6"
ext_gcd 17 13
zassert_eq "${EGCD[g]}" "1"  "gcd(17,13) = 1 (coprime)"
ext_gcd 270 192
zassert_eq "${EGCD[g]}" "6"  "gcd(270,192) = 6"
# Bezout identity check: a·x + b·y = gcd
ext_gcd 100 75
g=${EGCD[g]}; x=${EGCD[x]}; y=${EGCD[y]}
zassert_eq "$g" "25"          "gcd(100,75) = 25"
zassert_eq "$((100 * x + 75 * y))" "25" "Bezout: 100x + 75y = 25"
zassert_eq "$(mod_inv 3 11)"  "4"  "3⁻¹ mod 11 = 4"
zassert_eq "$(mod_inv 7 26)"  "15" "7⁻¹ mod 26 = 15"
zassert_eq "$(mod_inv 10 17)" "12" "10⁻¹ mod 17 = 12"
zassert_eq "$(mod_inv 15 100)" "no-inverse" "gcd(15,100)=5 → no inverse"
zassert_eq "$(mod_inv 6 9)"    "no-inverse" "gcd(6,9)=3 → no inverse"
# φ(12) = 4 (a ∈ {1,5,7,11})
phi=0
for a in {1..11}; do
    ext_gcd $a 12
    (( EGCD[g] == 1 )) && (( phi++ ))
done
zassert_eq "$phi" "4" "φ(12) = 4"
# RSA-toy: msg → enc → dec
zassert_eq "$back" "42" "RSA round-trip msg=42"
ztest_run
