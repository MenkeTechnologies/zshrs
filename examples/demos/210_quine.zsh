#!/usr/bin/env zshrs
# Self-printing program (quine) variants in zsh.

echo "── single-line quine attempt ──"
s='echo "s=${(qq)s}; eval \$s"'; eval $s

echo
echo "── read self and print ──"
read_self() {
    if [[ -f $0 ]]; then
        head -20 "$0"
    else
        echo "(no source available)"
    fi
}
echo "first 5 lines of this script:"
head -5 "$0" 2>/dev/null || echo "(non-file invocation)"

echo
echo "── functional quine (function that prints itself) ──"
typeset -f self_printing_fn
self_printing_fn() {
    typeset -f self_printing_fn
}
self_printing_fn

echo
echo "── reflective fn-via-functions[] ──"
demo_fn() { echo "hello"; }
echo "demo_fn body:"
echo "${functions[demo_fn]}"

echo
echo "── recursive self-reference safe pattern ──"
fact() {
    if (( $1 <= 1 )); then
        echo 1
    else
        echo $(( $1 * $(fact $(($1 - 1))) ))
    fi
}
echo "fact(5) = $(fact 5)"
echo "fact(10) = $(fact 10)"

echo
echo "── mutually recursive ──"
is_even_rec() {
    if (( $1 == 0 )); then return 0
    else
        is_odd_rec $(( $1 - 1 ))
    fi
}
is_odd_rec() {
    if (( $1 == 0 )); then return 1
    else
        is_even_rec $(( $1 - 1 ))
    fi
}
for n in 4 5 6 7; do
    if is_even_rec $n; then echo "$n is even"; else echo "$n is odd"; fi
done

echo
echo "── meta: print number of demos ──"
demo_count=$(ls examples/demos/*.zsh 2>/dev/null | wc -l)
echo "this repo has $((demo_count + 0)) demos"

# === ztest assertions ===
zassert_eq "$(fact 5)"  120     "fact(5)"
zassert_eq "$(fact 10)" 3628800 "fact(10)"
zassert_eq "$(fact 0)"  1       "fact(0) base"
is_even_rec 4 && zassert_ok 1 "4 is even" || zassert_ok 0 "4 should be even"
is_even_rec 7 && zassert_ok 0 "7 should be odd" || zassert_ok 1 "7 is odd"
zassert_ok "$demo_count" "demo_count populated"
zassert_gt "$demo_count" 0 "demo_count is positive"
zassert_contains "${functions[demo_fn]}" "hello" "functions[] reflection"
ztest_run
