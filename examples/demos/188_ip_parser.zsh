#!/usr/bin/env zshrs
# IP address parser + validator + subnet math.

is_valid_ipv4() {
    local ip=$1
    local -a parts=( ${(s/./)ip} )
    (( ${#parts[@]} == 4 )) || return 1
    local p
    for p in "${parts[@]}"; do
        [[ $p == [0-9]## ]] || return 1
        (( p >= 0 && p <= 255 )) || return 1
    done
    return 0
}

ipv4_to_int() {
    local ip=$1
    local -a parts=( ${(s/./)ip} )
    echo $(( (parts[1] << 24) | (parts[2] << 16) | (parts[3] << 8) | parts[4] ))
}

int_to_ipv4() {
    local n=$1
    printf "%d.%d.%d.%d\n" \
        $(( (n >> 24) & 0xff )) \
        $(( (n >> 16) & 0xff )) \
        $(( (n >> 8) & 0xff )) \
        $(( n & 0xff ))
}

ip_class() {
    local ip=$1
    local first=${ip%%.*}
    if (( first >= 1 && first <= 126 )); then echo A
    elif (( first >= 128 && first <= 191 )); then echo B
    elif (( first >= 192 && first <= 223 )); then echo C
    elif (( first >= 224 && first <= 239 )); then echo D
    elif (( first >= 240 && first <= 254 )); then echo E
    else echo "?"
    fi
}

is_private() {
    local ip=$1
    local first=${ip%%.*}
    local rest=${ip#*.}
    local second=${rest%%.*}
    if (( first == 10 )); then return 0; fi
    if (( first == 172 && second >= 16 && second <= 31 )); then return 0; fi
    if (( first == 192 && second == 168 )); then return 0; fi
    return 1
}

setopt extended_glob

echo "── validate ──"
for ip in 192.168.1.1 10.0.0.1 256.1.1.1 1.2.3 1.2.3.4.5 abc.def.ghi.jkl 0.0.0.0 255.255.255.255; do
    if is_valid_ipv4 "$ip"; then
        echo "  $ip: valid"
    else
        echo "  $ip: INVALID"
    fi
done

echo "── int conversion ──"
for ip in 192.168.1.1 10.0.0.1 8.8.8.8; do
    n=$(ipv4_to_int "$ip")
    back=$(int_to_ipv4 $n)
    echo "  $ip = $n → $back"
done

echo "── classify ──"
for ip in 10.0.0.1 172.16.0.1 192.168.1.1 8.8.8.8 224.0.0.1; do
    cls=$(ip_class "$ip")
    if is_private "$ip"; then
        echo "  $ip: class $cls (private)"
    else
        echo "  $ip: class $cls (public)"
    fi
done

echo "── subnet calc /24 ──"
for ip in 10.0.0.42 192.168.1.100; do
    n=$(ipv4_to_int "$ip")
    network=$(( n & 0xffffff00 ))
    broadcast=$(( n | 0xff ))
    echo "  $ip /24"
    echo "    network:   $(int_to_ipv4 $network)"
    echo "    broadcast: $(int_to_ipv4 $broadcast)"
done

echo "── iterate a small range ──"
start=$(ipv4_to_int "192.168.1.1")
for ((n=start; n<start+5; n++)); do
    echo "  $(int_to_ipv4 $n)"
done

# === ztest assertions ===
is_valid_ipv4 192.168.1.1 && r=1 || r=0; zassert_eq "$r" 1 "valid common ipv4"
is_valid_ipv4 256.1.1.1 && r=1 || r=0;   zassert_eq "$r" 0 "256 out of range"
is_valid_ipv4 1.2.3 && r=1 || r=0;       zassert_eq "$r" 0 "3-part rejected"
zassert_eq "$(ipv4_to_int 192.168.1.1)" 3232235777    "ipv4_to_int 192.168.1.1"
zassert_eq "$(ipv4_to_int 8.8.8.8)"     134744072     "ipv4_to_int 8.8.8.8"
zassert_eq "$(int_to_ipv4 3232235777)"  "192.168.1.1" "int_to_ipv4 roundtrip"
zassert_eq "$(ip_class 10.0.0.1)"   "A"   "class A"
zassert_eq "$(ip_class 172.16.0.1)" "B"   "class B"
zassert_eq "$(ip_class 192.168.1.1)" "C"  "class C"
zassert_eq "$(ip_class 224.0.0.1)"  "D"   "class D"
is_private 10.0.0.1     && r=1 || r=0; zassert_eq "$r" 1 "10/8 is private"
is_private 8.8.8.8      && r=1 || r=0; zassert_eq "$r" 0 "8.8.8.8 not private"
ztest_run
