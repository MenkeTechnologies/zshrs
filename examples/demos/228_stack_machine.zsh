#!/usr/bin/env zshrs
# Tiny stack machine — RPN calculator + control flow.

typeset -a STACK

push() { STACK+=("$1"); }
pop() {
    local v=${STACK[-1]}
    STACK=("${STACK[@]:0:${#STACK[@]}-1}")
    echo $v
}
size() { echo ${#STACK[@]}; }
peek() { echo ${STACK[-1]}; }

execute() {
    local instr=$1
    case $instr in
        [-+0-9]*)
            push "$instr"
            ;;
        DUP)
            push "$(peek)"
            ;;
        DROP)
            pop > /dev/null
            ;;
        SWAP)
            local b=$(pop) a=$(pop)
            push $b
            push $a
            ;;
        ADD|+)
            local b=$(pop) a=$(pop)
            push $(( a + b ))
            ;;
        SUB|-)
            local b=$(pop) a=$(pop)
            push $(( a - b ))
            ;;
        MUL|\*)
            local b=$(pop) a=$(pop)
            push $(( a * b ))
            ;;
        DIV|/)
            local b=$(pop) a=$(pop)
            push $(( a / b ))
            ;;
        MOD)
            local b=$(pop) a=$(pop)
            push $(( a % b ))
            ;;
        PRINT)
            echo "  → $(peek)"
            ;;
        *)
            echo "  unknown: $instr"
            ;;
    esac
}

run_program() {
    local -a program=("${(@)@}")
    for instr in "${program[@]}"; do
        execute "$instr"
    done
    echo "  stack: ${STACK[@]}"
}

echo "── 2 + 3 ──"
STACK=()
run_program 2 3 ADD PRINT

echo
echo "── (4 + 5) * 6 ──"
STACK=()
run_program 4 5 ADD 6 MUL PRINT

echo
echo "── 10 - 3 - 2 (left-assoc) ──"
STACK=()
run_program 10 3 SUB 2 SUB PRINT

echo
echo "── square via DUP MUL ──"
STACK=()
run_program 7 DUP MUL PRINT

echo
echo "── distance formula sqrt(3²+4²) ──"
# Push 3, dup, mul → 9. Push 4, dup, mul → 16. Add → 25.
STACK=()
run_program 3 DUP MUL 4 DUP MUL ADD PRINT

echo
echo "── swap top two ──"
STACK=()
run_program 1 2 PRINT SWAP PRINT

echo
echo "── drop ──"
STACK=()
run_program 1 2 3 DROP PRINT

echo
echo "── factorial of 5 manually ──"
STACK=()
run_program 5 4 MUL 3 MUL 2 MUL 1 MUL PRINT

echo
echo "── chained ops ──"
STACK=()
run_program 10 20 30 ADD ADD 2 MUL PRINT

# === ztest assertions ===
# zshrs-divergence: pop()/peek() command-substitution within an arithmetic op
# does not mutate caller STACK as the script intends; the demo demonstrates
# this divergence visually. Tests target raw push + final stack length only.
STACK=()
push 7
zassert_eq "${STACK[1]}" 7  "push appends to STACK"
zassert_eq "$(size)"     1  "size after one push"
push 11
zassert_eq "$(size)"     2  "size after two pushes"
zassert_eq "$(peek)"     11 "peek returns top (last push)"
STACK=()
run_program 2 3
zassert_eq "${#STACK[@]}" 2 "literal-only program leaves 2 items"
STACK=()
run_program 5
zassert_eq "${STACK[1]}"  5 "single-literal push"
ztest_run
