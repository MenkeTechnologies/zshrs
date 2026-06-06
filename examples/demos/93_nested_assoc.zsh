#!/usr/bin/env zshrs
# 2D-table emulation via flat assoc with composite keys.
# Ported from Src/params.c (PM_HASHED + key access patterns).

# Encode a 2D table using "row|col" composite keys.

typeset -A grid

set_cell() {
    local row=$1 col=$2 val=$3
    grid[${row},${col}]=$val
}

get_cell() {
    local row=$1 col=$2
    echo "${grid[${row},${col}]:-0}"
}

# Fill a 5x5 multiplication table.
for ((r = 1; r <= 5; r++)); do
    for ((c = 1; c <= 5; c++)); do
        set_cell $r $c $(( r * c ))
    done
done

echo "── 5x5 multiplication grid via 2D-assoc ──"
# NOTE: build the row in a buffer rather than printf-in-a-loop because
# zshrs has a known bug where unterminated stdout of one command bleeds
# into the next $(...) capture (BUGS.md candidate).
for ((r = 1; r <= 5; r++)); do
    row=""
    for ((c = 1; c <= 5; c++)); do
        val=${grid[${r},${c}]:-0}
        row+=$(printf "%4d" $val)
    done
    echo "$row"
done

echo "── lookup specific cells ──"
for pair in "2,3" "5,5" "1,4"; do
    set -- ${(s/,/)pair}
    echo "grid[$1,$2] = $(get_cell $1 $2)"
done

echo "── delete a cell ──"
unset 'grid[3,3]'
echo "grid[3,3] after unset: $(get_cell 3 3) (was 9)"

echo "── iterate via key/value pairs ──"
echo "non-zero cells (first 5):"
n=0
for k v in "${(@kv)grid}"; do
    (( n++ ))
    if (( n > 5 )); then break; fi
    echo "  $k → $v"
done

echo "── row totals ──"
for ((r = 1; r <= 5; r++)); do
    sum=0
    for ((c = 1; c <= 5; c++)); do
        val=${grid[${r},${c}]:-0}
        (( sum += val ))
    done
    echo "row $r sum = $sum"
done

echo "── 3-key composite (cube emulation) ──"
typeset -A cube
for ((x = 1; x <= 3; x++)); do
    for ((y = 1; y <= 3; y++)); do
        for ((z = 1; z <= 3; z++)); do
            cube[${x}/${y}/${z}]=$(( x + y + z ))
        done
    done
done
echo "cube[1/1/1] = ${cube[1/1/1]}"
echo "cube[3/3/3] = ${cube[3/3/3]}"
echo "cube[2/3/1] = ${cube[2/3/1]}"
echo "total entries: ${#cube[@]}"

# === ztest assertions ===
zassert_eq "$(get_cell 2 3)"  "6"   "grid[2,3] = 2*3"
zassert_eq "$(get_cell 5 5)"  "25"  "grid[5,5] = 25"
zassert_eq "$(get_cell 1 4)"  "4"   "grid[1,4] = 4"
zassert_eq "$(get_cell 3 3)"  "0"   "grid[3,3] unset → 0 fallback"
# Row totals
sum=0
for ((c = 1; c <= 5; c++)); do
    val=${grid[1,${c}]:-0}
    (( sum += val ))
done
zassert_eq "$sum" 15 "row 1 sum = 15"
# 3-key cube
zassert_eq "${cube[1/1/1]}"   3   "cube[1/1/1] = 1+1+1"
zassert_eq "${cube[3/3/3]}"   9   "cube[3/3/3] = 9"
zassert_eq "${cube[2/3/1]}"   6   "cube[2/3/1] = 6"
zassert_eq "${#cube[@]}"      27  "cube has 27 entries"
ztest_run

