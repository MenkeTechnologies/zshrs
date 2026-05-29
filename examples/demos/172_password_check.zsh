#!/usr/bin/env zshrs
# Password strength checker.

check_password() {
    local pw=$1
    local len=${#pw}
    local -i score=0
    local -i has_lower=0 has_upper=0 has_digit=0 has_special=0
    local i ch

    for ((i=1; i<=len; i++)); do
        ch=${pw[i]}
        case $ch in
            [a-z]) has_lower=1 ;;
            [A-Z]) has_upper=1 ;;
            [0-9]) has_digit=1 ;;
            *)     has_special=1 ;;
        esac
    done

    # Score components.
    (( len >= 8 )) && (( score += 1 ))
    (( len >= 12 )) && (( score += 1 ))
    (( len >= 16 )) && (( score += 1 ))
    (( has_lower )) && (( score += 1 ))
    (( has_upper )) && (( score += 1 ))
    (( has_digit )) && (( score += 1 ))
    (( has_special )) && (( score += 2 ))

    # Common-pattern penalties.
    if [[ $pw == *password* || $pw == *123456* || $pw == *qwerty* ]]; then
        (( score -= 3 ))
    fi
    if [[ $pw == [a-z]## || $pw == [A-Z]## || $pw == [0-9]## ]]; then
        # All-one-class is weak.
        (( score -= 1 ))
    fi

    (( score < 0 )) && score=0
    (( score > 10 )) && score=10

    # Classify.
    local strength
    if (( score >= 8 )); then strength="strong"
    elif (( score >= 5 )); then strength="medium"
    elif (( score >= 3 )); then strength="weak"
    else strength="very weak"
    fi

    local features=""
    (( has_lower )) && features+="a-z "
    (( has_upper )) && features+="A-Z "
    (( has_digit )) && features+="0-9 "
    (( has_special )) && features+="!@#$ "

    printf "  pw=%-20s len=%2d score=%2d strength=%s features=[%s]\n" \
        "'$pw'" $len $score "$strength" "$features"
}

setopt extended_glob

echo "── test passwords ──"
check_password "12345"
check_password "password"
check_password "abc"
check_password "Password"
check_password "P@ssw0rd"
check_password "Tr0ub4dor&3"
check_password "correct horse battery staple"
check_password "ZshRsIsBest2026!"
check_password "aaaaaaaaaaaaaaaa"
check_password "qwerty123"
check_password "MyStrongP@ss123!"
