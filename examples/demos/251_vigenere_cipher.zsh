#!/usr/bin/env zshrs
# Vigenère cipher — classic poly-alphabetic substitution.

# Convert char to 0-25 index (A=0..Z=25); -1 for non-letter.
char_idx() {
    local c=$1
    case $c in
        [A-Z]) printf "%d" $(( #c - 65 )) ;;
        [a-z]) printf "%d" $(( #c - 97 )) ;;
        *)     printf "%d" -1 ;;
    esac
}

idx_to_char() {
    local i=$1 case=$2  # case=u or l
    if [[ $case == u ]]; then
        printf "\\$(printf %03o $((i + 65)))"
    else
        printf "\\$(printf %03o $((i + 97)))"
    fi
}

encrypt() {
    local plain=$1 key=$2 out=""
    local i kp pi ki shifted is_upper
    local klen=${#key}
    local pk_idx=0
    for ((i=1; i<=${#plain}; i++)); do
        local c=${plain[i]}
        pi=$(char_idx "$c")
        if (( pi < 0 )); then
            out+="$c"  # passthrough non-letters
            continue
        fi
        kp=${key[ pk_idx % klen + 1 ]}
        ki=$(char_idx "$kp")
        shifted=$(( (pi + ki) % 26 ))
        case $c in
            [A-Z]) is_upper=u ;;
            *)     is_upper=l ;;
        esac
        out+=$(idx_to_char $shifted $is_upper)
        (( pk_idx++ ))
    done
    echo "$out"
}

decrypt() {
    local cipher=$1 key=$2 out=""
    local i kp ci ki shifted is_upper
    local klen=${#key}
    local pk_idx=0
    for ((i=1; i<=${#cipher}; i++)); do
        local c=${cipher[i]}
        ci=$(char_idx "$c")
        if (( ci < 0 )); then
            out+="$c"
            continue
        fi
        kp=${key[ pk_idx % klen + 1 ]}
        ki=$(char_idx "$kp")
        shifted=$(( (ci - ki + 26) % 26 ))
        case $c in
            [A-Z]) is_upper=u ;;
            *)     is_upper=l ;;
        esac
        out+=$(idx_to_char $shifted $is_upper)
        (( pk_idx++ ))
    done
    echo "$out"
}

echo "── encrypt/decrypt round-trips ──"
pairs=(
    "HELLO|KEY"
    "ATTACKATDAWN|LEMON"
    "The quick brown fox|cipher"
    "ABCDEFGHIJKLMNOPQRSTUVWXYZ|A"   # key A = identity
    "Cryptography is fun!|MENKE"
)
for p in "${pairs[@]}"; do
    plain="${p%|*}"
    key="${p#*|}"
    cipher=$(encrypt "$plain" "$key")
    back=$(decrypt "$cipher" "$key")
    printf "  plain:  %s\n  key:    %s\n  cipher: %s\n  decrypt: %s   %s\n\n" \
        "$plain" "$key" "$cipher" "$back" "$([[ $back == $plain ]] && echo ✓ || echo ✗)"
done

echo "── identity test (key=A) ──"
plain="ZSHRS RUNS HOT"
cipher=$(encrypt "$plain" "A")
echo "  '$plain' with key 'A' → '$cipher'"

echo
echo "── tableau preview (first 5 rows) ──"
for ((r=0; r<5; r++)); do
    row=""
    for ((c=0; c<26; c++)); do
        row+=$(idx_to_char $(((r + c) % 26)) u)
    done
    echo "  $row"
done

# === ztest assertions ===
zassert_eq "$(char_idx A)" "0"   "A → 0"
zassert_eq "$(char_idx Z)" "25"  "Z → 25"
zassert_eq "$(char_idx a)" "0"   "a → 0"
zassert_eq "$(char_idx z)" "25"  "z → 25"
zassert_eq "$(char_idx ' ')" "-1" "space → -1"
zassert_eq "$(idx_to_char 0 u)"  "A" "0 → A"
zassert_eq "$(idx_to_char 25 u)" "Z" "25 → Z"
zassert_eq "$(idx_to_char 0 l)"  "a" "0 → a"
zassert_eq "$(encrypt HELLO KEY)" "RIJVS"          "encrypt HELLO/KEY"
zassert_eq "$(encrypt ATTACKATDAWN LEMON)" "LXFOPVEFRNHR" "encrypt ATTACKATDAWN/LEMON"
zassert_eq "$(decrypt RIJVS KEY)" "HELLO"          "decrypt RIJVS/KEY"
zassert_eq "$(decrypt LXFOPVEFRNHR LEMON)" "ATTACKATDAWN" "decrypt round trip"
zassert_eq "$(encrypt ABCDEFGHIJKLMNOPQRSTUVWXYZ A)" "ABCDEFGHIJKLMNOPQRSTUVWXYZ" "key=A is identity"
enc_round=$(encrypt 'Cryptography is fun!' MENKE)
dec_round=$(decrypt "$enc_round" MENKE)
zassert_eq "$dec_round" "Cryptography is fun!" "punct passthrough round-trip"
ztest_run
