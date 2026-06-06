#!/usr/bin/env zshrs
# Transposition cipher — columnar transposition w/ keyword.

# Encrypt: place plaintext in rows of width=keylen, read columns by keyword order.
columnar_encrypt() {
    local plain=$1 key=$2
    plain="${plain// /}"   # strip spaces
    local klen=${#key}
    local plen=${#plain}
    # Pad to multiple of klen.
    local rows=$(( (plen + klen - 1) / klen ))
    local total=$(( rows * klen ))
    while (( ${#plain} < total )); do
        plain+="X"
    done

    # Build grid as array of rows.
    typeset -a grid
    local r
    for ((r=1; r<=rows; r++)); do
        local start=$(( (r-1)*klen + 1 ))
        local end=$(( r * klen ))
        grid[r]="${plain[$start,$end]}"
    done

    # Determine column order from key (alphabetical, ties by position).
    typeset -a tagged
    local i
    for ((i=1; i<=klen; i++)); do
        tagged+=("${key[i]} $i")
    done
    sorted=( "${(@n)tagged}" )

    # Build cipher by reading columns in order.
    local out=""
    for entry in "${sorted[@]}"; do
        local col=${entry##* }
        for ((r=1; r<=rows; r++)); do
            out+="${grid[r][col]}"
        done
    done
    echo "$out"
}

columnar_decrypt() {
    local cipher=$1 key=$2
    local klen=${#key}
    local clen=${#cipher}
    local rows=$(( clen / klen ))

    typeset -a tagged
    local i
    for ((i=1; i<=klen; i++)); do
        tagged+=("${key[i]} $i")
    done
    sorted=( "${(@n)tagged}" )

    # Reconstruct columns in sorted order.
    typeset -A column_data
    local ci=1
    for entry in "${sorted[@]}"; do
        local col=${entry##* }
        local col_end=$(( ci + rows - 1 ))
        column_data[$col]="${cipher[$ci,$col_end]}"
        ci=$(( ci + rows ))
    done

    # Rebuild row by row.
    local out=""
    for ((r=1; r<=rows; r++)); do
        for ((i=1; i<=klen; i++)); do
            out+="${column_data[$i][r]}"
        done
    done
    echo "$out"
}

# Rail fence cipher (zigzag).
rail_fence_encrypt() {
    local plain=$1 rails=$2
    local n=${#plain}
    typeset -a fence
    local i
    for ((i=1; i<=rails; i++)); do fence[i]=""; done
    local row=1 dir=1
    for ((i=1; i<=n; i++)); do
        fence[row]+="${plain[i]}"
        if (( row == rails )); then dir=-1; fi
        if (( row == 1 )); then dir=1; fi
        (( row += dir ))
    done
    local out=""
    for ((i=1; i<=rails; i++)); do
        out+="${fence[i]}"
    done
    echo "$out"
}

# Reverse rail fence.
rail_fence_decrypt() {
    local cipher=$1 rails=$2
    local n=${#cipher}
    # Compute rail-pattern offsets.
    typeset -a pattern row_count
    local row=1 dir=1 i
    for ((i=1; i<=n; i++)); do
        pattern[i]=$row
        (( row_count[row]++ ))
        if (( row == rails )); then dir=-1; fi
        if (( row == 1 )); then dir=1; fi
        (( row += dir ))
    done
    # Distribute cipher chars to rails.
    typeset -a rail
    for ((i=1; i<=rails; i++)); do rail[i]=""; done
    local ci=1
    for ((row=1; row<=rails; row++)); do
        local rc=${row_count[row]:-0}
        rail[row]="${cipher[$ci,$((ci+rc-1))]}"
        (( ci += rc ))
    done
    # Read back by zig-zag.
    typeset -a pointers
    for ((i=1; i<=rails; i++)); do pointers[i]=1; done
    local out="" pat_row pos
    for ((i=1; i<=n; i++)); do
        pat_row=${pattern[i]}
        pos=${pointers[pat_row]}
        out+="${rail[pat_row][pos]}"
        (( pointers[pat_row]++ ))
    done
    echo "$out"
}

echo "── columnar transposition ──"
tests=(
    "HELLOWORLD|KEY"
    "ATTACKATDAWN|LEMON"
    "THEQUICKBROWNFOX|CIPHER"
    "MEETMEMIDNIGHT|ZEBRA"
)
for t in "${tests[@]}"; do
    plain="${t%|*}"
    key="${t#*|}"
    enc=$(columnar_encrypt "$plain" "$key")
    dec=$(columnar_decrypt "$enc" "$key")
    # Strip trailing X pads.
    dec="${dec%%X##}"
    printf "  plain:  %s\n  key:    %s\n  enc:    %s\n  dec:    %s\n\n" \
        "$plain" "$key" "$enc" "$dec"
done

echo "── rail fence ──"
tests=(
    "WEAREDISCOVEREDFLEEATONCE|3"
    "ATTACKATDAWN|3"
    "HELLOWORLD|2"
    "HELLOWORLD|4"
    "WORLDOFCRYPTOGRAPHY|5"
)
for t in "${tests[@]}"; do
    plain="${t%|*}"
    rails="${t#*|}"
    enc=$(rail_fence_encrypt "$plain" $rails)
    dec=$(rail_fence_decrypt "$enc" $rails)
    mark="✓"
    [[ $dec != $plain ]] && mark="✗"
    printf "  plain (%2d rails): %s\n  enc:              %s\n  dec:              %s   %s\n\n" \
        "$rails" "$plain" "$enc" "$dec" "$mark"
done

echo "── key order analysis ──"
# Show which column gets read first for various keys.
keys=(KEY ZEBRA HELLO ABCDE)
for k in "${keys[@]}"; do
    typeset -a tagged
    tagged=()
    local i
    for ((i=1; i<=${#k}; i++)); do
        tagged+=("${k[i]} $i")
    done
    sorted=( "${(@n)tagged}" )
    local order=""
    for entry in "${sorted[@]}"; do
        order+="${entry##* } "
    done
    printf "  key '%-8s' → col order: %s\n" "$k" "$order"
done

echo
echo "── frequency-preserving property ──"
# Transposition preserves character histogram.
plain="THEQUICKBROWNFOXJUMPSOVERTHELAZYDOG"
echo "  plain: $plain"
enc=$(columnar_encrypt "$plain" "ZEBRA")
echo "  enc:   $enc"
typeset -A p_freq c_freq
for ((i=1; i<=${#plain}; i++)); do
    (( p_freq[${plain[i]}]++ ))
done
for ((i=1; i<=${#enc}; i++)); do
    [[ ${enc[i]} == X ]] && continue
    (( c_freq[${enc[i]}]++ ))
done

# Compare sums.
p_total=0
c_total=0
for k in "${(@k)p_freq}"; do (( p_total += p_freq[$k] )); done
for k in "${(@k)c_freq}"; do (( c_total += c_freq[$k] )); done
echo "  plaintext chars: $p_total"
echo "  ciphertext chars (excl X pad): $c_total"
echo "  → frequencies preserved (vulnerable to letter-freq analysis)"

echo
echo "── stats ──"
echo "  classical ciphers covered: columnar transposition, rail fence"
echo "  vulnerability: preserves letter frequency (defeated by Kasiski/Friedman)"
echo "  strength: hides patterns / digraphs"
echo "  WWI-era use: ADFGVX, double transposition"

# === ztest assertions ===
enc=$(columnar_encrypt "HELLOWORLD" "KEY")
zassert_eq "$enc" "EORXHLODLWLX"               "columnar HELLOWORLD/KEY"
enc=$(columnar_encrypt "ATTACKATDAWN" "LEMON")
zassert_eq "$enc" "TANAKWTTXCAXADX"            "columnar ATTACKATDAWN/LEMON"
enc=$(columnar_encrypt "MEETMEMIDNIGHT" "ZEBRA")
zassert_eq "$enc" "MNXEIHEMGTDTMEI"            "columnar MEETMEMIDNIGHT/ZEBRA"
enc=$(rail_fence_encrypt "WEAREDISCOVEREDFLEEATONCE" 3)
zassert_eq "$enc" "WECRLTEERDSOEEFEAOCAIVDEN"  "rail fence 3 rails"
enc=$(rail_fence_encrypt "ATTACKATDAWN" 3)
zassert_eq "$enc" "ACDTAKTANTAW"               "rail fence ATTACK 3 rails"
enc=$(rail_fence_encrypt "HELLOWORLD" 2)
zassert_eq "$enc" "HLOOLELWRD"                 "rail fence HELLOWORLD 2 rails"
ztest_run
