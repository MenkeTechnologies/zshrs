#!/usr/bin/env zshrs
# IPv6 parser — validate, expand, compress.

# `[0-9A-Fa-f]##` (one-or-more) and `${g##0##}` below are EXTENDED_GLOB
# operators; without this setopt they are matched as literal `#` characters.
setopt extended_glob

is_hex_group() {
    local g=$1
    (( ${#g} > 4 )) && return 1
    [[ -z $g ]] && return 1
    [[ $g == [0-9A-Fa-f]## ]] && return 0
    return 1
}

# Expand "::" to full 8 groups of 4 hex.
expand_ipv6() {
    local addr=$1
    # Split on "::" first (max one occurrence).
    local left right
    if [[ $addr == *::* ]]; then
        left="${addr%%::*}"
        right="${addr#*::}"
        # Count groups in each side.
        local lcount=0 rcount=0
        if [[ -n $left ]]; then
            local IFS_save=$IFS
            IFS=':'
            set -- $=left
            lcount=$#
            IFS=$IFS_save
        fi
        if [[ -n $right ]]; then
            local IFS_save=$IFS
            IFS=':'
            set -- $=right
            rcount=$#
            IFS=$IFS_save
        fi
        local need=$(( 8 - lcount - rcount ))
        if (( need < 1 )); then echo "INVALID"; return 1; fi
        local zeros=""
        local i
        for ((i=0; i<need; i++)); do zeros+=":0000"; done
        # Build full address.
        local full
        if [[ -z $left && -z $right ]]; then
            # "::" on its own — the zero run IS the whole address.
            full="${zeros#:}"
        elif [[ -z $left ]]; then
            full="${zeros#:}:${right}"
        elif [[ -z $right ]]; then
            full="${left}${zeros}"
            full="${full#:}"
        else
            full="${left}${zeros}:${right}"
        fi
        # Pad each group to 4 chars.
        local IFS_save=$IFS
        IFS=':'
        set -- $=full
        IFS=$IFS_save
        local out=""
        for g in "$@"; do
            local padded=$(printf "%04s" "$g")
            padded=${padded// /0}
            out+="${padded}:"
        done
        echo "${out%:}"
    else
        # No "::". Should be 8 colon-sep groups.
        local IFS_save=$IFS
        IFS=':'
        set -- $=addr
        IFS=$IFS_save
        if (( $# != 8 )); then echo "INVALID"; return 1; fi
        local out=""
        for g in "$@"; do
            local padded=$(printf "%04s" "$g")
            padded=${padded// /0}
            out+="${padded}:"
        done
        echo "${out%:}"
    fi
}

# Compress by collapsing longest run of zero groups.
compress_ipv6() {
    local expanded=$1
    local IFS_save=$IFS
    IFS=':'
    set -- $=expanded
    IFS=$IFS_save
    # Find longest run of "0000" or "0".
    local i n=$#
    local best_start=-1 best_len=0
    local cur_start=-1 cur_len=0
    for ((i=1; i<=n; i++)); do
        local g="${(P)i}"
        # Strip leading zeros except last char.
        local stripped="${g##0##}"
        [[ -z $stripped ]] && stripped="0"
        if [[ $stripped == "0" ]]; then
            if (( cur_start < 0 )); then cur_start=$i; cur_len=1
            else (( cur_len++ )); fi
            if (( cur_len > best_len )); then
                best_len=$cur_len
                best_start=$cur_start
            fi
        else
            cur_start=-1
            cur_len=0
        fi
    done
    # Strip leading zeros from each group.
    local out=""
    local replaced=0
    for ((i=1; i<=n; i++)); do
        if (( best_len >= 2 && i >= best_start && i < best_start + best_len )); then
            if (( ! replaced )); then
                out+="::"
                replaced=1
            fi
            continue
        fi
        local g="${(P)i}"
        local stripped="${g##0##}"
        [[ -z $stripped ]] && stripped="0"
        out+="${stripped}:"
    done
    # Clean trailing ":".
    if [[ $out == *::: ]]; then
        out=${out%:}
    elif [[ $out == *: && $out != ::* ]]; then
        out=${out%:}
    fi
    # If didn't end with ::, strip the final ":" left by loop.
    if [[ $out == *: && $out != *:: ]]; then
        out=${out%:}
    fi
    echo "$out"
}

addrs=(
    "::1"
    "::"
    "fe80::1"
    "2001:db8::1"
    "2001:0db8:0000:0000:0000:0000:0000:0001"
    "2001:db8:0:0:1:0:0:1"
    "ff02::1:2"
    "1::"
    "1:2:3:4:5:6:7:8"
)

echo "── expand ──"
for a in "${addrs[@]}"; do
    printf "  %-40s → %s\n" "$a" "$(expand_ipv6 "$a")"
done

echo
echo "── round-trip (expand → compress) ──"
for a in "${addrs[@]}"; do
    expanded=$(expand_ipv6 "$a")
    if [[ $expanded == INVALID ]]; then
        printf "  %-40s → INVALID\n" "$a"
        continue
    fi
    compressed=$(compress_ipv6 "$expanded")
    printf "  %-40s → %s → %s\n" "$a" "$expanded" "$compressed"
done

echo
echo "── invalid cases ──"
bad=(
    "1:2:3"             # too few
    "1:2:3:4:5:6:7:8:9" # too many
    "gg::1"             # not hex
)
for a in "${bad[@]}"; do
    printf "  %-40s → %s\n" "$a" "$(expand_ipv6 "$a" 2>/dev/null)"
done

echo
echo "── group hex validation ──"
samples=(0 ff abcd 12345 g123 "" 0000)
for g in "${samples[@]}"; do
    if is_hex_group "$g"; then echo "  '$g' → valid"; else echo "  '$g' → invalid"; fi
done

# === ztest assertions ===
# expand_ipv6 zero-pads every group to 4 hex digits, so the expected strings
# below are the fully-expanded RFC 4291 forms.
zassert_eq "$(expand_ipv6 '::1')"        "0000:0000:0000:0000:0000:0000:0000:0001" "expand ::1"
zassert_eq "$(expand_ipv6 '::')"         "0000:0000:0000:0000:0000:0000:0000:0000" "expand ::"
zassert_eq "$(expand_ipv6 'fe80::1')"    "fe80:0000:0000:0000:0000:0000:0000:0001"    "expand fe80::1"
zassert_eq "$(expand_ipv6 '2001:db8::1')" "2001:0db8:0000:0000:0000:0000:0000:0001" "expand 2001:db8::1"
zassert_eq "$(expand_ipv6 '1:2:3:4:5:6:7:8')" "0001:0002:0003:0004:0005:0006:0007:0008" "full form passthrough"
zassert_eq "$(expand_ipv6 'ff02::1:2')"  "ff02:0000:0000:0000:0000:0000:0001:0002" "expand ff02::1:2"
zassert_eq "$(expand_ipv6 '1::')"        "0001:0000:0000:0000:0000:0000:0000:0000" "expand 1::"
zassert_eq "$(expand_ipv6 '1:2:3')"      "INVALID" "too few groups → INVALID"
zassert_eq "$(expand_ipv6 '1:2:3:4:5:6:7:8:9')" "INVALID" "too many groups → INVALID"
ztest_run
