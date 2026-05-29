#!/usr/bin/env zshrs
# Naive JSON pretty-printer in pure zsh — state machine over chars.

local LB="{" RB="}" LK="[" RK="]" Q='"' COMMA="," COLON=":"

emit_indent() {
    local n=$1 k
    for ((k=0; k<n; k++)); do printf "  "; done
}

json_pretty() {
    local s=$1
    local -i indent=0
    local -i in_string=0
    local i ch
    for ((i = 1; i <= ${#s}; i++)); do
        ch=${s[i]}
        if (( in_string )); then
            printf "%s" "$ch"
            if [[ $ch == $Q ]]; then in_string=0; fi
            continue
        fi
        if [[ $ch == $LB ]]; then
            printf "${LB}\n"
            (( indent++ ))
            emit_indent $indent
        elif [[ $ch == $LK ]]; then
            printf "${LK}\n"
            (( indent++ ))
            emit_indent $indent
        elif [[ $ch == $RB ]]; then
            printf "\n"
            (( indent-- ))
            emit_indent $indent
            printf "%s" "$RB"
        elif [[ $ch == $RK ]]; then
            printf "\n"
            (( indent-- ))
            emit_indent $indent
            printf "%s" "$RK"
        elif [[ $ch == $COMMA ]]; then
            printf ",\n"
            emit_indent $indent
        elif [[ $ch == $COLON ]]; then
            printf ": "
        elif [[ $ch == $Q ]]; then
            printf "%s" "$Q"
            in_string=1
        elif [[ $ch == " " ]]; then
            : # skip
        else
            printf "%s" "$ch"
        fi
    done
    printf "\n"
}

echo "── small object ──"
json_pretty '{"name":"Alice","age":30,"role":"admin"}'

echo "── nested object ──"
json_pretty '{"user":{"id":1,"name":"Bob"},"active":true}'

echo "── array ──"
json_pretty '[1,2,3,"four",{"k":"v"}]'
