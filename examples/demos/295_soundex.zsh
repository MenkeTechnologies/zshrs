#!/usr/bin/env zshrs
# Soundex — phonetic hash for English surname matching.

soundex_digit() {
    local c=$1
    case $c in
        b|f|p|v|B|F|P|V) echo 1 ;;
        c|g|j|k|q|s|x|z|C|G|J|K|Q|S|X|Z) echo 2 ;;
        d|t|D|T) echo 3 ;;
        l|L) echo 4 ;;
        m|n|M|N) echo 5 ;;
        r|R) echo 6 ;;
        *) echo "" ;;
    esac
}

soundex() {
    local s=$1
    [[ -z $s ]] && { echo ""; return; }
    s=${s:u}    # uppercase
    local first=${s[1]}
    local out="$first"
    local prev_digit=$(soundex_digit "$first")
    local i
    for ((i=2; i<=${#s}; i++)); do
        local ch=${s[i]}
        local d=$(soundex_digit "$ch")
        if [[ -n $d && $d != $prev_digit ]]; then
            out+="$d"
        fi
        if [[ -n $d ]]; then
            prev_digit=$d
        else
            # H and W don't reset, all others do.
            if [[ $ch != H && $ch != W ]]; then
                prev_digit=""
            fi
        fi
        if (( ${#out} >= 4 )); then break; fi
    done
    # Pad to 4 chars.
    while (( ${#out} < 4 )); do out+="0"; done
    echo "$out"
}

echo "── classic Soundex tests ──"
classics=(
    Robert R163
    Rupert R163
    Rubin  R150
    Ashcraft A261
    Tymczak T520
    Pfister P236
    Honeyman H555
    Smith  S530
    Smyth  S530
    Lee    L000
    Gutierrez G362
)
for ((i=1; i<=${#classics}; i+=2)); do
    name=${classics[i]}
    expected=${classics[i+1]}
    got=$(soundex "$name")
    mark="✓"
    [[ $got != $expected ]] && mark="✗"
    printf "  %-12s : %s (expected %s) %s\n" "$name" "$got" "$expected" "$mark"
done

echo
echo "── group names by soundex ──"
names=(
    Adams Adamson Edmonds
    Smith Smyth Schmidt
    Brown Browne Braun
    Johnson Jensen Janssen
    Robinson Roberson Robertson
    Lee Lea Leigh
    Wilson Wilcox
    Davis Davies
    Miller Mueller Molnar
    Garcia Gracia
    Anderson Henderson
)
typeset -A groups
for name in "${names[@]}"; do
    code=$(soundex "$name")
    if [[ -n ${groups[$code]} ]]; then
        groups[$code]="${groups[$code]} $name"
    else
        groups[$code]="$name"
    fi
done

for code in "${(@ko)groups}"; do
    members=( ${=groups[$code]} )
    printf "  %s : %s\n" "$code" "${members[*]}"
done

echo
echo "── fuzzy match via soundex ──"
fuzzy_match() {
    local query=$1
    shift
    local target_code=$(soundex "$query")
    local -a matches
    for name in "$@"; do
        if [[ $(soundex "$name") == $target_code ]]; then
            matches+=("$name")
        fi
    done
    echo "${matches[@]}"
}

dictionary=(
    Smith Smyth Schmidt Smithson
    Brown Browne Braun Bron
    Johnson Jansen Jensen
    Robinson Robertson Robson
    Davis Davies Davison
    Williams Willams
)
queries=(Smith Brown Davis Williams)
for q in "${queries[@]}"; do
    matches=$(fuzzy_match "$q" "${dictionary[@]}")
    printf "  '%s' → %s\n" "$q" "$matches"
done

echo
echo "── digit mapping table ──"
echo "  b,f,p,v        → 1"
echo "  c,g,j,k,q,s,x,z → 2"
echo "  d,t            → 3"
echo "  l              → 4"
echo "  m,n            → 5"
echo "  r              → 6"
echo "  a,e,h,i,o,u,w,y → 0 (vowels + h/w)"

echo
echo "── verify expected codes ──"
expected_pairs=(
    "Lee L000"
    "Lloyd L300"
    "O'Brien O165"
    "X X000"
)
for p in "${expected_pairs[@]}"; do
    set -- ${=p}
    name=$1; expected=$2
    got=$(soundex "$name")
    printf "  %-12s → %s (expected %s) %s\n" "$name" "$got" "$expected" "$([[ $got == $expected ]] && echo ✓ || echo ✗)"
done

# === ztest assertions ===
zassert_eq "$(soundex Robert)"    "R163"  "soundex Robert"
zassert_eq "$(soundex Rupert)"    "R163"  "soundex Rupert (same as Robert)"
zassert_eq "$(soundex Rubin)"     "R150"  "soundex Rubin"
zassert_eq "$(soundex Smith)"     "S530"  "soundex Smith"
zassert_eq "$(soundex Smyth)"     "S530"  "soundex Smyth (homophone)"
zassert_eq "$(soundex Lee)"       "L000"  "soundex Lee"
zassert_eq "$(soundex Lloyd)"     "L300"  "soundex Lloyd"
zassert_eq "$(soundex X)"         "X000"  "soundex single letter padded"
zassert_eq "$(soundex_digit b)"   "1"     "digit b"
zassert_eq "$(soundex_digit r)"   "6"     "digit r"
ztest_run
