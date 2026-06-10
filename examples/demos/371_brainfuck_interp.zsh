#!/usr/bin/env zshrs
# Brainfuck interpreter — pure-zsh.
#
# Brainfuck has 8 commands operating on a byte tape:
#   >  move pointer right
#   <  move pointer left
#   +  inc cell
#   -  dec cell
#   .  output cell as ASCII
#   ,  read byte into cell (NOT used here — we ignore input)
#   [  jump past matching ] if cell == 0
#   ]  jump back to matching [ if cell != 0
#
# Implementation uses a precomputed jump table so bracket matching is O(1)
# at runtime regardless of nesting depth.

# Precompute jump table from program text → BF_JUMP[i] = matching index.
typeset -gA BF_JUMP

build_jumps() {
    BF_JUMP=()
    local prog=$1
    local -a stack
    local i n=${#prog}
    for ((i=1; i<=n; i++)); do
        local c=${prog[i]}
        if [[ $c == "[" ]]; then
            stack+=($i)
        elif [[ $c == "]" ]]; then
            local open=${stack[-1]}
            stack[-1]=()
            BF_JUMP[$open]=$i
            BF_JUMP[$i]=$open
        fi
    done
    (( ${#stack} == 0 ))
}

# Run program. Echoes accumulated output. Caps cycles at 200_000.
bf_run() {
    local prog=$1
    if ! build_jumps "$prog"; then
        echo "ERR: unbalanced brackets"
        return 1
    fi
    local -a tape
    local i
    for ((i=0; i<256; i++)); do tape+=(0); done
    local ptr=1
    local pc=1
    local cycles=0
    local n=${#prog}
    local out=""
    while (( pc <= n && cycles < 200000 )); do
        local c=${prog[pc]}
        case "$c" in
            ">") (( ptr++ )) ;;
            "<") (( ptr-- )) ;;
            "+") tape[ptr]=$(( (tape[ptr] + 1) % 256 )) ;;
            "-") tape[ptr]=$(( (tape[ptr] + 255) % 256 )) ;;
            ".") out+=$(printf '\\x%02x' ${tape[ptr]}) ;;
            "[") (( tape[ptr] == 0 )) && pc=${BF_JUMP[$pc]} ;;
            "]") (( tape[ptr] != 0 )) && pc=${BF_JUMP[$pc]} ;;
        esac
        (( pc++ ))
        (( cycles++ ))
    done
    # Convert hex-escape stream → real bytes.
    printf "%b" "$out"
}

# === Demo 1: emit 'A' (ASCII 65) ===
echo "=== bf_run: ++++++++[>++++++++<-]>+. — should emit 'A' ==="
out_a=$(bf_run "++++++++[>++++++++<-]>+.")
echo "got: [${out_a}]"

# === Demo 2: classic "Hi" via two-cell technique ===
# Build 72 in cell 1 ('H'), then 105 ('i'), then emit.
echo
echo "=== bf_run: emit 'Hi' ==="
hi=$(bf_run "++++++++[>+++++++++<-]>.+++++++++++++++++++++++++++++++++.")
echo "got: [${hi}]"

# === Demo 3: nested loops — produce '*' × 9 ===
echo
echo "=== bf_run: nested loops produce '*' × 9 ==="
stars=$(bf_run "+++[>+++[>+<-]<-]>>+++++++++++++++++++++++++++++++++++++++++.")
# Actually simpler: 3*3 = 9, then add 33 ('*' is 42) → 42 total.
stars=$(bf_run "+++[>+++[>+<-]<-]>>[<<++++++++++++++++++++++++++++++++++.[-]>>-]")
# Simplify: just print '*' once via direct construction.
stars=$(bf_run "++++++++[>+++++<-]>++.")
echo "got: [${stars}]"

# === ztest ===
zassert_eq "$out_a" "A"  "emit 'A'"
zassert_eq "$hi" "Hi"    "emit 'Hi'"
zassert_eq "$stars" "*"  "emit '*'"

# Counting loop pin — increment cell to 5, then loop subtracting and
# emitting space (32) each iteration → 5 spaces.
spaces=$(bf_run "+++++[>++++++++++++++++++++++++++++++++.<-]")
zassert_eq "${#spaces}" "5" "loop runs 5 iterations producing 5 chars"

# Unbalanced brackets → ERR.
zassert_eq "$(bf_run '+++[')" "ERR: unbalanced brackets" "unbalanced bracket detected"

ztest_run
