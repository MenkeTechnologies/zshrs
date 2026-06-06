#!/usr/bin/env zshrs
# IPv4 subnet calculator — CIDR parse, network/broadcast/range, contains.

# Parse "a.b.c.d" → 32-bit int.
ip_to_int() {
    local ip=$1
    local a b c d
    a=${ip%%.*}
    local rest=${ip#*.}
    b=${rest%%.*}
    rest=${rest#*.}
    c=${rest%%.*}
    d=${rest#*.}
    echo $(( (a << 24) | (b << 16) | (c << 8) | d ))
}

# 32-bit int → "a.b.c.d".
int_to_ip() {
    local n=$1
    local a=$(( (n >> 24) & 255 ))
    local b=$(( (n >> 16) & 255 ))
    local c=$(( (n >> 8) & 255 ))
    local d=$(( n & 255 ))
    echo "${a}.${b}.${c}.${d}"
}

# CIDR /N → mask as 32-bit int.
cidr_mask() {
    local n=$1
    (( n == 0 )) && { echo 0; return; }
    echo $(( (0xFFFFFFFF << (32 - n)) & 0xFFFFFFFF ))
}

# Network address = ip & mask.
network() {
    local ip=$1 mask=$2
    echo $(( $(ip_to_int "$ip") & $(cidr_mask $mask) ))
}

# Broadcast = network | ~mask.
broadcast() {
    local ip=$1 mask=$2
    local net=$(network "$ip" $mask)
    local invmask=$(( ~$(cidr_mask $mask) & 0xFFFFFFFF ))
    echo $(( net | invmask ))
}

# Host count = 2^(32-N) - 2 (usable, excl. network + broadcast).
host_count() {
    local n=$1
    if (( n >= 31 )); then echo $(( 1 << (32 - n) )); return; fi
    echo $(( (1 << (32 - n)) - 2 ))
}

# Class of address.
ip_class() {
    local ip=$1
    local a=${ip%%.*}
    if (( a < 128 )); then echo "A"
    elif (( a < 192 )); then echo "B"
    elif (( a < 224 )); then echo "C"
    elif (( a < 240 )); then echo "D (multicast)"
    else echo "E (reserved)"
    fi
}

# Private?
is_private() {
    local ip=$1
    local a=${ip%%.*}
    local rest=${ip#*.}
    local b=${rest%%.*}
    if (( a == 10 )); then return 0; fi
    if (( a == 172 && b >= 16 && b <= 31 )); then return 0; fi
    if (( a == 192 && b == 168 )); then return 0; fi
    if (( a == 127 )); then return 0; fi   # loopback
    return 1
}

# Check ip in network.
ip_in_network() {
    local ip=$1 net_ip=$2 mask=$3
    local ip_int=$(ip_to_int "$ip")
    local net=$(( $(ip_to_int "$net_ip") & $(cidr_mask $mask) ))
    local masked=$(( ip_int & $(cidr_mask $mask) ))
    (( masked == net ))
}

echo "── CIDR analysis ──"
networks=(
    "192.168.1.10/24"
    "10.0.0.1/8"
    "172.16.0.0/12"
    "192.168.100.50/30"
    "8.8.8.8/32"
    "0.0.0.0/0"
    "127.0.0.1/8"
)
for cidr in "${networks[@]}"; do
    ip=${cidr%/*}
    mask=${cidr#*/}
    net=$(network "$ip" $mask)
    bc=$(broadcast "$ip" $mask)
    hc=$(host_count $mask)
    cls=$(ip_class "$ip")
    priv=""
    if is_private "$ip"; then priv=" (private)"; fi
    echo
    echo "  $cidr$priv"
    echo "    IP class:       $cls"
    echo "    mask:           $(int_to_ip $(cidr_mask $mask))"
    echo "    network:        $(int_to_ip $net)"
    echo "    broadcast:      $(int_to_ip $bc)"
    echo "    first usable:   $(int_to_ip $((net + 1)))"
    echo "    last usable:    $(int_to_ip $((bc - 1)))"
    echo "    host count:     $hc"
done

echo
echo "── address conversion ──"
ips=("0.0.0.0" "127.0.0.1" "192.168.1.1" "8.8.8.8" "255.255.255.255")
for ip in "${ips[@]}"; do
    n=$(ip_to_int "$ip")
    back=$(int_to_ip $n)
    printf "  %-18s → %10s → %-18s %s\n" "$ip" "$n" "$back" "$([[ $ip == $back ]] && echo ✓ || echo ✗)"
done

echo
echo "── contains check ──"
queries=(
    "192.168.1.50    192.168.1.0/24"
    "192.168.2.50    192.168.1.0/24"
    "10.20.30.40     10.0.0.0/8"
    "172.20.30.40    172.16.0.0/12"
    "8.8.4.4         8.0.0.0/8"
    "1.2.3.4         8.0.0.0/8"
)
for q in "${queries[@]}"; do
    set -- ${=q}
    ip=$1
    cidr=$2
    net_ip=${cidr%/*}
    mask=${cidr#*/}
    if ip_in_network "$ip" "$net_ip" $mask; then
        printf "  %-18s in %-20s : ✓\n" "$ip" "$cidr"
    else
        printf "  %-18s in %-20s : ✗\n" "$ip" "$cidr"
    fi
done

echo
echo "── private ranges (RFC 1918) ──"
echo "  10.0.0.0/8      (16,777,216 addresses)"
echo "  172.16.0.0/12   (1,048,576 addresses)"
echo "  192.168.0.0/16  (65,536 addresses)"
echo "  127.0.0.0/8     (loopback)"

echo
echo "── special ranges ──"
specials=(
    "0.0.0.0          default route"
    "127.0.0.1        localhost"
    "169.254.0.0/16   link-local"
    "224.0.0.0/4      multicast"
    "255.255.255.255  broadcast"
)
for s in "${specials[@]}"; do
    echo "  $s"
done

# === ztest assertions ===
zassert_eq "$(ip_to_int 0.0.0.0)" "0"                   "0.0.0.0 = 0"
zassert_eq "$(ip_to_int 255.255.255.255)" "4294967295"  "255.255.255.255 = 2^32-1"
zassert_eq "$(ip_to_int 127.0.0.1)" "2130706433"        "127.0.0.1 = 0x7F000001"
zassert_eq "$(int_to_ip 0)" "0.0.0.0"                   "0 → 0.0.0.0"
zassert_eq "$(int_to_ip 2130706433)" "127.0.0.1"        "2130706433 → 127.0.0.1"
zassert_eq "$(cidr_mask 24)" "4294967040"               "/24 mask = 0xFFFFFF00"
zassert_eq "$(cidr_mask 0)" "0"                         "/0 mask = 0"
zassert_eq "$(int_to_ip $(network 192.168.1.10 24))" "192.168.1.0"  "network 192.168.1.10/24"
zassert_eq "$(int_to_ip $(broadcast 192.168.1.10 24))" "192.168.1.255"  "broadcast 192.168.1.10/24"
zassert_eq "$(host_count 24)" "254"                     "/24 → 254 usable"
zassert_eq "$(host_count 30)" "2"                       "/30 → 2 usable"
zassert_eq "$(ip_class 10.0.0.1)"   "A"                 "10.x → class A"
zassert_eq "$(ip_class 192.168.1.1)" "C"                "192.x → class C"
is_private 10.0.0.1 && r=1 || r=0; zassert_eq "$r" "1"  "10.x is private"
is_private 8.8.8.8 && r=1 || r=0; zassert_eq "$r" "0"   "8.8.8.8 not private"
ip_in_network 192.168.1.50 192.168.1.0 24 && r=1 || r=0; zassert_eq "$r" "1"  "ip_in_network match"
ip_in_network 192.168.2.50 192.168.1.0 24 && r=1 || r=0; zassert_eq "$r" "0"  "ip_in_network miss"
ztest_run
