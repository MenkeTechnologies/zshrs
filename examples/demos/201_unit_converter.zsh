#!/usr/bin/env zshrs
# Unit conversion — length, weight, temperature, time.

# Length: base unit = mm
typeset -A LEN_TO_MM
LEN_TO_MM[mm]=1
LEN_TO_MM[cm]=10
LEN_TO_MM[m]=1000
LEN_TO_MM[km]=1000000
LEN_TO_MM[in]=254       # 25.4 * 10
LEN_TO_MM[ft]=3048      # 304.8 * 10
LEN_TO_MM[yd]=9144      # 914.4 * 10
LEN_TO_MM[mi]=16093440  # 1609344 * 10

# Convert: from FROM to TO units. Uses integer math with implicit /10 to handle .254 mm precision.
convert_length() {
    local val=$1 from=$2 to=$3
    local from_mm=${LEN_TO_MM[$from]}
    local to_mm=${LEN_TO_MM[$to]}
    # result = val * from_mm / to_mm
    local result=$(( val * from_mm * 100 / to_mm ))
    printf "%d.%02d" $(( result / 100 )) $(( result % 100 ))
}

echo "── length ──"
echo "1 m  → $(convert_length 1 m cm) cm"
echo "1 km → $(convert_length 1 km m) m"
echo "1 mi → $(convert_length 1 mi ft) ft"
echo "1 mi → $(convert_length 1 mi m) m"
echo "100 ft → $(convert_length 100 ft m) m"
echo "26.2 mi (marathon) ≈ $(convert_length 26 mi km) km"

# Weight: base = mg
typeset -A WT_TO_MG
WT_TO_MG[mg]=1
WT_TO_MG[g]=1000
WT_TO_MG[kg]=1000000
WT_TO_MG[oz]=28349   # 28.349 g * 1000
WT_TO_MG[lb]=453592  # 453.592 g * 1000

convert_weight() {
    local val=$1 from=$2 to=$3
    local from_mg=${WT_TO_MG[$from]}
    local to_mg=${WT_TO_MG[$to]}
    local result=$(( val * from_mg * 100 / to_mg ))
    printf "%d.%02d" $(( result / 100 )) $(( result % 100 ))
}

echo
echo "── weight ──"
echo "1 kg → $(convert_weight 1 kg g) g"
echo "1 lb → $(convert_weight 1 lb kg) kg"
echo "1 oz → $(convert_weight 1 oz g) g"
echo "150 lb (≈human) → $(convert_weight 150 lb kg) kg"

# Temperature
to_celsius() {
    local val=$1 unit=$2
    case $unit in
        C) echo $val ;;
        F) echo $(( (val - 32) * 5 / 9 )) ;;
        K) echo $(( val - 273 )) ;;
    esac
}

from_celsius() {
    local val=$1 unit=$2
    case $unit in
        C) echo $val ;;
        F) echo $(( val * 9 / 5 + 32 )) ;;
        K) echo $(( val + 273 )) ;;
    esac
}

convert_temp() {
    local val=$1 from=$2 to=$3
    local c=$(to_celsius $val $from)
    from_celsius $c $to
}

echo
echo "── temperature ──"
echo "0 C → $(convert_temp 0 C F) F"
echo "100 C → $(convert_temp 100 C F) F"
echo "32 F → $(convert_temp 32 F C) C"
echo "212 F → $(convert_temp 212 F C) C"
echo "0 C → $(convert_temp 0 C K) K"
echo "300 K → $(convert_temp 300 K C) C"

# Time
typeset -A TIME_TO_SEC
TIME_TO_SEC[s]=1
TIME_TO_SEC[min]=60
TIME_TO_SEC[hr]=3600
TIME_TO_SEC[day]=86400
TIME_TO_SEC[week]=604800
TIME_TO_SEC[year]=31536000

convert_time() {
    local val=$1 from=$2 to=$3
    local from_s=${TIME_TO_SEC[$from]}
    local to_s=${TIME_TO_SEC[$to]}
    echo $(( val * from_s / to_s ))
}

echo
echo "── time ──"
echo "1 hr → $(convert_time 1 hr s) s"
echo "1 day → $(convert_time 1 day hr) hr"
echo "1 year → $(convert_time 1 year day) day"
echo "3600 s → $(convert_time 3600 s hr) hr"
echo "1 week → $(convert_time 1 week min) min"

# === ztest assertions ===
zassert_eq "$(convert_length 1 m cm)"  "100.00"  "1 m = 100 cm"
zassert_eq "$(convert_length 1 km m)"  "1000.00" "1 km = 1000 m"
zassert_eq "$(convert_length 1 mi ft)" "5280.00" "1 mi = 5280 ft"
zassert_eq "$(convert_weight 1 kg g)"  "1000.00" "1 kg = 1000 g"
zassert_eq "$(convert_weight 1 lb kg)" "0.45"    "1 lb ≈ 0.45 kg (integer trunc)"
zassert_eq "$(convert_temp 0 C F)"     32        "0 C = 32 F"
zassert_eq "$(convert_temp 100 C F)"   212       "100 C = 212 F"
zassert_eq "$(convert_temp 32 F C)"    0         "32 F = 0 C"
zassert_eq "$(convert_temp 0 C K)"     273       "0 C = 273 K"
zassert_eq "$(convert_time 1 hr s)"    3600      "1 hr = 3600 s"
zassert_eq "$(convert_time 1 day hr)"  24        "1 day = 24 hr"
zassert_eq "$(convert_time 1 year day)" 365      "1 year = 365 day"
ztest_run
