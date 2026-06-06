#!/usr/bin/env zshrs
# Recipe unit converter — scale, unit-convert, format.

# Conversion factors to base unit (ml for volume, g for mass).
typeset -A VOL MASS

VOL=(
    "tsp" 5
    "tbsp" 15
    "cup" 240
    "pint" 473
    "quart" 946
    "gallon" 3785
    "fl_oz" 30
    "ml" 1
    "l" 1000
)

MASS=(
    "oz" 28
    "lb" 454
    "g" 1
    "kg" 1000
    "stick" 113   # 1 stick butter ≈ 113g
)

# Scale a quantity (int or "1/2") by integer multiplier.
parse_qty() {
    local q=$1
    # Handle fraction "a/b" or "n a/b".
    if [[ $q == */* ]]; then
        local num=${q%/*}
        local den=${q#*/}
        # 1000x scaled.
        echo $(( num * 1000 / den ))
        return
    fi
    # Mixed "n a/b".
    if [[ $q == *' '*/* ]]; then
        local int_part=${q%% *}
        local frac=${q##* }
        local num=${frac%/*}
        local den=${frac#*/}
        echo $(( int_part * 1000 + num * 1000 / den ))
        return
    fi
    # Plain int or decimal.
    if [[ $q == *.* ]]; then
        local d=${q%.*} f=${q#*.}
        f="${f}000"
        f=${f[1,3]}
        echo $(( d * 1000 + f ))
    else
        echo $(( q * 1000 ))
    fi
}

format_qty() {
    local q1000=$1
    local i=$(( q1000 / 1000 ))
    local f=$(( q1000 % 1000 ))
    # Common fractions.
    case $f in
        0)   printf "%d" $i ;;
        250) printf "%d 1/4" $i ;;
        333) printf "%d 1/3" $i ;;
        500) printf "%d 1/2" $i ;;
        667) printf "%d 2/3" $i ;;
        750) printf "%d 3/4" $i ;;
        *)   printf "%d.%03d" $i $f ;;
    esac
}

# scale "qty unit ingredient" by N.
scale_line() {
    local qty=$1 unit=$2 ing=$3 mult=$4
    local q1000=$(parse_qty "$qty")
    local scaled=$(( q1000 * mult ))
    printf "  %s %s %s\n" "$(format_qty $scaled)" "$unit" "$ing"
}

# Convert vol unit to ml.
to_ml() {
    local q=$1 u=$2
    local q1000=$(parse_qty "$q")
    local factor=${VOL[$u]:-0}
    (( factor == 0 )) && { echo "?"; return; }
    echo $(( q1000 * factor / 1000 ))  # result in ml
}

# Convert vol unit A to unit B.
convert_vol() {
    local q=$1 from=$2 to=$3
    local ml=$(to_ml "$q" "$from")
    local target_factor=${VOL[$to]:-0}
    (( target_factor == 0 )) && { echo "?"; return; }
    # ml / target_factor, with 1000x precision.
    echo $(( ml * 1000 / target_factor ))
}

# Recipe (qty, unit, ingredient).
recipe=(
    "2|cup|flour"
    "1|cup|sugar"
    "1/2|tsp|salt"
    "1|tsp|vanilla"
    "3|tbsp|cocoa"
    "1/2|cup|butter"
    "2||eggs"
    "1/4|cup|milk"
)

echo "── original recipe ──"
for r in "${recipe[@]}"; do
    set -- ${(s:|:)r}
    printf "  %s %s %s\n" "$1" "$2" "$3"
done

echo
echo "── scaled x2 ──"
for r in "${recipe[@]}"; do
    set -- ${(s:|:)r}
    scale_line "$1" "$2" "$3" 2
done

echo
echo "── scaled x3 ──"
for r in "${recipe[@]}"; do
    set -- ${(s:|:)r}
    scale_line "$1" "$2" "$3" 3
done

echo
echo "── volume conversions ──"
echo "  1 cup → $(convert_vol 1 cup ml) ml (×1000)"
echo "  1 cup → $(convert_vol 1 cup fl_oz) fl_oz (×1000)"
echo "  1 cup → $(convert_vol 1 cup tbsp) tbsp (×1000)"
echo "  1 cup → $(convert_vol 1 cup tsp) tsp (×1000)"
echo "  1 l   → $(convert_vol 1 l cup) cup (×1000)"
echo "  1 gallon → $(convert_vol 1 gallon l) l (×1000)"

echo
echo "── mass to grams ──"
for u in oz lb stick kg; do
    factor=${MASS[$u]}
    echo "  1 $u = $factor g"
done

# === ztest assertions ===
zassert_eq "${VOL[cup]}"    "240"  "1 cup = 240 ml"
zassert_eq "${VOL[tsp]}"    "5"    "1 tsp = 5 ml"
zassert_eq "${VOL[tbsp]}"   "15"   "1 tbsp = 15 ml"
zassert_eq "${VOL[l]}"      "1000" "1 l = 1000 ml"
zassert_eq "${VOL[gallon]}" "3785" "1 gallon ≈ 3785 ml"
zassert_eq "${MASS[oz]}"    "28"   "1 oz ≈ 28 g"
zassert_eq "${MASS[lb]}"    "454"  "1 lb ≈ 454 g"
zassert_eq "${MASS[stick]}" "113"  "1 stick butter ≈ 113 g"
zassert_eq "$(parse_qty 2)"   "2000"  "parse 2 → 2000"
zassert_eq "$(parse_qty '1/2')" "500"   "parse 1/2 → 500"
zassert_eq "$(parse_qty '1/4')" "250"   "parse 1/4 → 250"
zassert_eq "$(format_qty 500)"  "0 1/2" "format 500 → 0 1/2"
zassert_eq "$(format_qty 1000)" "1"     "format 1000 → 1"
zassert_eq "$(format_qty 2500)" "2 1/2" "format 2500 → 2 1/2"
ztest_run
