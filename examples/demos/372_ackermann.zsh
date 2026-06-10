#!/usr/bin/env zshrs
# Ackermann function — the canonical total-but-not-primitive-recursive
# function (Ackermann 1928, simplified by Péter).
#
#   A(0,n)   = n + 1
#   A(m,0)   = A(m-1, 1)            for m > 0
#   A(m,n)   = A(m-1, A(m, n-1))    for m,n > 0
#
# Growth is so fast that A(4,2) already has 19,729 decimal digits. We
# also can't afford to compute too many cells in shell: A(3,5)=253
# costs ~5s of stack-mutating iteration in a shell-level loop, and
# A(4,*) goes off the rails immediately (A(4,1)=65533). The demo
# table is therefore bounded at (m ≤ 3, n ≤ 4), well inside CI's 30s
# wall-clock cap.

ackermann_rec() {
    local m=$1 n=$2
    if (( m == 0 )); then
        echo $(( n + 1 ))
        return
    fi
    if (( n == 0 )); then
        ackermann_rec $(( m - 1 )) 1
        return
    fi
    ackermann_rec $(( m - 1 )) "$(ackermann_rec $m $(( n - 1 )))"
}

# Iterative Ackermann via explicit pending-m stack.
# Invariant: the value being computed is
#   A(stack[-1], A(stack[-2], A(... A(stack[1], n)...)))
# Pop reduces depth; the loop exits when the stack drains and n holds
# the answer.
ackermann_iter() {
    local m=$1 n=$2
    local -a stack
    stack+=($m)
    while (( ${#stack} > 0 )); do
        m=${stack[-1]}
        stack[-1]=()
        if (( m == 0 )); then
            (( n++ ))
        elif (( n == 0 )); then
            n=1
            stack+=($(( m - 1 )))
        else
            stack+=($(( m - 1 )))
            stack+=($m)
            (( n-- ))
        fi
    done
    echo $n
}

echo "=== Ackermann table (iterative — stack machine) ==="
printf "      "
for n in 0 1 2 3 4; do printf "%6d" $n; done
echo
for m in 0 1 2 3; do
    printf "m=%d  " $m
    for n in 0 1 2 3 4; do
        printf "%6s" "$(ackermann_iter $m $n)"
    done
    echo
done

# === ztest — known values via iter ===
zassert_eq "$(ackermann_iter 0 0)" "1"   "A(0,0)=1"
zassert_eq "$(ackermann_iter 0 5)" "6"   "A(0,5)=6"
zassert_eq "$(ackermann_iter 1 0)" "2"   "A(1,0)=2"
zassert_eq "$(ackermann_iter 1 5)" "7"   "A(1,5)=7"
zassert_eq "$(ackermann_iter 2 0)" "3"   "A(2,0)=3"
zassert_eq "$(ackermann_iter 2 3)" "9"   "A(2,3)=9"
zassert_eq "$(ackermann_iter 2 6)" "15"  "A(2,6)=15"
zassert_eq "$(ackermann_iter 3 0)" "5"   "A(3,0)=5"
zassert_eq "$(ackermann_iter 3 1)" "13"  "A(3,1)=13"
zassert_eq "$(ackermann_iter 3 2)" "29"  "A(3,2)=29"
zassert_eq "$(ackermann_iter 3 3)" "61"  "A(3,3)=61"

# Iterative ≡ recursive — pin only for the cheap (m ≤ 1) range. The
# recursive impl is depth-safe at m=2 but exploded $(  ) cost makes
# it impractical to spot-check beyond a single point.
for m in 0 1; do
    for n in 0 1 2 3 4; do
        r=$(ackermann_rec $m $n)
        i=$(ackermann_iter $m $n)
        zassert_eq "$i" "$r" "iter == rec for A($m,$n)"
    done
done

# Single m=2 spot-check to prove the rec impl is otherwise correct.
zassert_eq "$(ackermann_rec 2 2)" "$(ackermann_iter 2 2)" "iter == rec for A(2,2)"

ztest_run
