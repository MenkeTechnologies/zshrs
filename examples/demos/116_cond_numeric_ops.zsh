#!/usr/bin/env zshrs
# Numeric + string conditional operators inside [[ … ]].
# Ported from Src/cond.c cond_str / cond_val + cond_match.

a=5
b=10
s1=hello
s2=world

echo "── arithmetic numeric ─ via -eq -lt etc ──"
[[ $a -eq 5 ]] && echo "a -eq 5"
[[ $a -lt $b ]] && echo "a -lt b"
[[ $a -le 5 ]] && echo "a -le 5"
[[ $b -gt $a ]] && echo "b -gt a"
[[ $b -ge 10 ]] && echo "b -ge 10"
[[ $a -ne $b ]] && echo "a -ne b"

echo "── direct arith inside (( )) ──"
(( a == 5 )) && echo "(( a == 5 ))"
(( a < b )) && echo "(( a < b ))"
(( b - a == 5 )) && echo "(( b - a == 5 ))"

echo "── string comparison < > (lexical) ──"
[[ "apple" < "banana" ]] && echo "apple < banana"
[[ "abc" < "abd" ]] && echo "abc < abd"
[[ "Z" < "a" ]] && echo "Z < a (ASCII)"

echo "── string == != ──"
[[ $s1 == hello ]] && echo "s1 == hello"
[[ $s1 != $s2 ]] && echo "s1 != s2"

echo "── string emptiness ──"
empty=
nonempty=value
[[ -z $empty ]] && echo "-z empty: yes"
[[ -n $nonempty ]] && echo "-n nonempty: yes"

echo "── string contains via glob ──"
[[ "hello world" == *world* ]] && echo "contains 'world'"
[[ "hello world" == hello* ]] && echo "starts with 'hello'"
[[ "hello world" == *world ]] && echo "ends with 'world'"

echo "── combined with && || ──"
[[ $a -gt 0 && $a -lt 100 ]] && echo "0 < a < 100"
[[ $a -gt 100 || $b -gt 5 ]] && echo "either branch"

echo "── negation ! ──"
[[ ! -z $nonempty ]] && echo "! -z nonempty (== -n)"
[[ ! $a -eq 99 ]] && echo "a != 99"

echo "── file tests ──"
tmp=/tmp/zshrs_cond_$$
touch "$tmp"
[[ -e $tmp ]] && echo "-e exists"
[[ -f $tmp ]] && echo "-f regular"
[[ -r $tmp ]] && echo "-r readable"
[[ ! -d $tmp ]] && echo "! -d not dir"
rm -f "$tmp"

echo "── chained dispatch ──"
classify() {
    local n=$1
    if (( n < 0 )); then echo "neg"
    elif (( n == 0 )); then echo "zero"
    elif (( n < 10 )); then echo "single digit"
    elif (( n < 100 )); then echo "double digit"
    else echo "100+"
    fi
}
for n in -5 0 7 42 100 1000; do
    printf "%5d → %s\n" $n "$(classify $n)"
done

# === ztest assertions ===
# Numeric -eq / -lt / -gt etc.
[[ 5 -eq 5 ]] && zassert_ok 1  "5 -eq 5" || zassert_ok 0 "5 -eq 5"
[[ 5 -lt 10 ]] && zassert_ok 1 "5 -lt 10" || zassert_ok 0 "5 -lt 10"
[[ 10 -gt 5 ]] && zassert_ok 1 "10 -gt 5" || zassert_ok 0 "10 -gt 5"
[[ 5 -ne 10 ]] && zassert_ok 1 "5 -ne 10" || zassert_ok 0 "5 -ne 10"
# String lexical
[[ "apple" < "banana" ]] && zassert_ok 1 "apple < banana lex" || zassert_ok 0 "apple < banana lex"
# Glob match
[[ "hello world" == *world* ]] && zassert_ok 1 "glob contains" || zassert_ok 0 "glob contains"
[[ "hello world" == hello* ]] && zassert_ok 1 "glob starts" || zassert_ok 0 "glob starts"
# classify dispatch
zassert_eq "$(classify -5)"   "neg"            "classify -5"
zassert_eq "$(classify 0)"    "zero"           "classify 0"
zassert_eq "$(classify 7)"    "single digit"   "classify 7"
zassert_eq "$(classify 42)"   "double digit"   "classify 42"
zassert_eq "$(classify 100)"  "100+"           "classify 100"
zassert_eq "$(classify 1000)" "100+"           "classify 1000"
ztest_run
