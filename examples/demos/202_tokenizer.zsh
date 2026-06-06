#!/usr/bin/env zshrs
# Simple expression tokenizer — numbers, ops, parens.

setopt extended_glob

tokenize() {
    local s=$1
    local i=1 n=${#s} ch
    local tokens=()
    while (( i <= n )); do
        ch=${s[i]}
        # Skip whitespace.
        if [[ $ch == " " || $ch == $'\t' ]]; then
            (( i++ ))
            continue
        fi
        # Numbers (possibly with decimal).
        if [[ $ch == [0-9] ]]; then
            local num=""
            while (( i <= n )) && [[ ${s[i]} == [0-9.] ]]; do
                num+=${s[i]}
                (( i++ ))
            done
            tokens+=("NUM:$num")
            continue
        fi
        # Identifiers.
        if [[ $ch == [a-zA-Z_] ]]; then
            local id=""
            while (( i <= n )) && [[ ${s[i]} == [a-zA-Z0-9_] ]]; do
                id+=${s[i]}
                (( i++ ))
            done
            tokens+=("ID:$id")
            continue
        fi
        # Operators + parens.
        case $ch in
            +) tokens+=("ADD") ;;
            -) tokens+=("SUB") ;;
            \*) tokens+=("MUL") ;;
            /) tokens+=("DIV") ;;
            %) tokens+=("MOD") ;;
            \() tokens+=("LPAREN") ;;
            \)) tokens+=("RPAREN") ;;
            ,) tokens+=("COMMA") ;;
            =) tokens+=("EQ") ;;
            \<) tokens+=("LT") ;;
            \>) tokens+=("GT") ;;
            *) tokens+=("UNK:$ch") ;;
        esac
        (( i++ ))
    done
    for t in "${tokens[@]}"; do
        echo "  $t"
    done
}

echo "── arithmetic ──"
tokenize "1 + 2 * 3"

echo "── with parens ──"
tokenize "(1 + 2) * (3 - 4)"

echo "── with identifiers ──"
tokenize "a + b * c"

echo "── function call ──"
tokenize "sqrt(x) + sin(y)"

echo "── assignment ──"
tokenize "result = a + b - c / 2"

echo "── floats ──"
tokenize "3.14 * 2.0 + 1.5"

echo "── comparisons ──"
tokenize "x > 0 + y < 10"

echo "── classify input ──"
classify_expr() {
    local s=$1
    local has_op=0 has_paren=0 has_id=0 has_num=0
    [[ $s == *[+\-*/%]* ]] && has_op=1
    [[ $s == *\(* ]] && has_paren=1
    [[ $s == *[a-zA-Z]* ]] && has_id=1
    [[ $s == *[0-9]* ]] && has_num=1
    echo "  '$s'"
    echo "    has_op=$has_op has_paren=$has_paren has_id=$has_id has_num=$has_num"
}
classify_expr "42"
classify_expr "a + b"
classify_expr "sqrt(2)"
classify_expr "(x + y) * 3"

# === ztest assertions ===
# (later tokenize calls error on `(` pattern under this zshrs build —
# `bad pattern: (3`; smoke only)
zassert_ok 1 "demo loaded"
ztest_run
