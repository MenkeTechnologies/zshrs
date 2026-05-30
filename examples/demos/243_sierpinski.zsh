#!/usr/bin/env zshrs
# Sierpinski triangle — fractal via Pascal's triangle modulo 2.

ROWS=16

# Build via Pascal's triangle binomial mod 2.
echo "── Sierpinski via Pascal mod 2 (16 rows) ──"
typeset -a row prev
prev=(1)
local r c
for ((r=1; r<=ROWS; r++)); do
    # Left-pad to center.
    local pad=$((ROWS - r))
    local line=""
    for ((c=0; c<pad; c++)); do line+=" "; done
    for v in "${prev[@]}"; do
        if (( v )); then
            line+="▲ "
        else
            line+="  "
        fi
    done
    echo "$line"
    # Compute next row of Pascal mod 2.
    row=(1)
    for ((c=1; c<${#prev[@]}; c++)); do
        row+=( $(( (prev[c] + prev[c+1]) % 2 )) )
    done
    row+=(1)
    prev=("${row[@]}")
done

echo
echo "── Sierpinski via bitwise (i & j == 0) ──"
# Classic: at row i col j, print if (i & j) == 0.
SZ=32
for ((i=0; i<SZ; i++)); do
    line=""
    for ((j=0; j<SZ; j++)); do
        if (( j > i )); then
            line+=" "
        elif (( (i & j) == 0 )); then
            line+="*"
        else
            line+=" "
        fi
    done
    echo "$line"
done

echo
echo "── Sierpinski carpet (3-state) ──"
# Cell (i,j) is hole if any (i/3^k % 3) and (j/3^k % 3) are both 1.
carpet() {
    local i=$1 j=$2 k
    for ((k=0; k<3; k++)); do
        if (( (i / (3**k)) % 3 == 1 && (j / (3**k)) % 3 == 1 )); then
            return 1
        fi
    done
    return 0
}

SZ=27
for ((i=0; i<SZ; i++)); do
    line=""
    for ((j=0; j<SZ; j++)); do
        if carpet $i $j; then line+="█"; else line+=" "; fi
    done
    echo "$line"
done
