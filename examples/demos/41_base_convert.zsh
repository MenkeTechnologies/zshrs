#!/usr/bin/env zshrs
# Base conversion — decimal ↔ binary/octal/hex via printf + arith.

to_bin() {
    local n=$1 bits=""
    if (( n == 0 )); then echo 0; return; fi
    while (( n > 0 )); do
        bits="$((n % 2))$bits"
        (( n /= 2 ))
    done
    echo "$bits"
}

to_hex() {
    printf "%x\n" $1
}

to_oct() {
    printf "%o\n" $1
}

from_bin() {
    echo $(( 2#$1 ))
}

from_hex() {
    echo $(( 16#$1 ))
}

from_oct() {
    echo $(( 8#$1 ))
}

echo "── dec → bin/oct/hex ──"
for n in 0 1 2 7 8 15 16 31 32 255 256 1024; do
    printf "%4d  bin=%-12s oct=%-5s hex=%s\n" \
        $n "$(to_bin $n)" "$(to_oct $n)" "$(to_hex $n)"
done

echo "── back to dec ──"
echo "0b1010 = $(from_bin 1010)"
echo "0x1F   = $(from_hex 1F)"
echo "0o755  = $(from_oct 755)"
echo "0b11111111 = $(from_bin 11111111)"

# === ztest assertions ===
zassert_eq "$(to_bin 0)"      "0"          "0 = 0"
zassert_eq "$(to_bin 10)"     "1010"       "10 = 1010"
zassert_eq "$(to_bin 255)"    "11111111"   "255 = 11111111"
zassert_eq "$(to_bin 256)"    "100000000"  "256 = 1<<8"
zassert_eq "$(to_hex 255)"    "ff"         "255 hex"
zassert_eq "$(to_hex 256)"    "100"        "256 hex"
zassert_eq "$(to_oct 8)"      "10"         "8 oct"
zassert_eq "$(to_oct 64)"     "100"        "64 oct"
zassert_eq "$(from_bin 1010)" "10"         "from_bin 1010"
zassert_eq "$(from_hex 1F)"   "31"         "from_hex 1F"
zassert_eq "$(from_oct 755)"  "493"        "from_oct 755"
# Round trip
zassert_eq "$(from_bin $(to_bin 42))" "42" "round trip 42"
ztest_run
