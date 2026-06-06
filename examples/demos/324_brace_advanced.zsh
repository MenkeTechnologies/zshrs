#!/usr/bin/env zshrs
# Brace expansion — comprehensive zsh forms.
# Ports Src/glob.c::brace_expand + Src/lex.c (parse_string).

echo "── numeric ranges ──"
echo "  {1..5}        :" {1..5}
echo "  {0..10}       :" {0..10}
echo "  {-3..3}       :" {-3..3}
echo "  {1..10..2}    :" {1..10..2}
echo "  {10..1..2}    :" {10..1..-2}
echo "  {100..110}    :" {100..110}

echo
echo "── zero-padded ranges ──"
echo "  {01..05}      :" {01..05}
echo "  {001..010}    :" {001..010}
echo "  {1..100..10}  :" {1..100..10}

echo
echo "── alpha ranges ──"
echo "  {a..e}        :" {a..e}
echo "  {A..F}        :" {A..F}
echo "  {a..z..3}     :" {a..z..3}
echo "  {z..a}        :" {z..a}

echo
echo "── alternation lists ──"
echo "  {apple,banana,cherry} :" {apple,banana,cherry}
echo "  prefix-{x,y,z}        :" prefix-{x,y,z}
echo "  {1,one,uno}           :" {1,one,uno}

echo
echo "── nested braces ──"
echo "  {a,b{1,2,3}c,d}       :" {a,b{1,2,3}c,d}
echo "  {{1..3},{a..c}}       :" {{1..3},{a..c}}

echo
echo "── product ──"
echo "  {a,b,c}{1,2,3}        :" {a,b,c}{1,2,3}
echo "  {x..z}{a..c}          :" {x..z}{a..c}

echo
echo "── file extension patterns ──"
echo "  log.{out,err,debug}       :" log.{out,err,debug}
echo "  config.{json,yaml,toml}   :" config.{json,yaml,toml}
echo "  test_{a..d}.{1..3}.txt    :" test_{a..d}.{1..3}.txt

echo
echo "── practical: timestamp filenames ──"
echo "  backup_{2024..2026}_{q1,q2,q3,q4}.tar:" backup_{2024..2026}_{q1,q2,q3,q4}.tar

echo
echo "── empty alternation ──"
echo "  {,a,b}                : '|' marks empty"
arr=( {,a,b} )
for ((i=1; i<=${#arr}; i++)); do
    printf "  [%d] = '%s'\n" $i "${arr[i]}"
done

echo
echo "── side effects with arithmetic ──"
echo "  $((2+3))-{a..c}        :" $((2+3))-{a..c}

echo
echo "── arrays from braces ──"
typeset -a days
days=( {Mon,Tue,Wed,Thu,Fri,Sat,Sun} )
echo "  days array: ${days[@]} (size ${#days})"

typeset -a months
months=( {Jan,Feb,Mar,Apr,May,Jun,Jul,Aug,Sep,Oct,Nov,Dec} )
echo "  months array: ${months[@]} (size ${#months})"

typeset -a digits
digits=( {0..9} )
echo "  digits: ${digits[@]}"

typeset -a hex
hex=( {0..9} {a..f} )
echo "  hex digits: ${hex[@]}"

echo
echo "── loop over brace ──"
for i in {1..5}; do
    printf "  iter %d\n" $i
done

echo
echo "── nested loops via product ──"
for combo in {x,y}{1,2}; do
    printf "  %s\n" "$combo"
done

echo
echo "── command repetition ──"
echo {1..5}-{a..c} | tr ' ' '\n' | head -8 | sed 's/^/  /'

echo
echo "── count expansions ──"
echo "  {1..100}: $(echo {1..100} | wc -w | tr -d ' ') items"
echo "  {a..z}:   $(echo {a..z} | wc -w | tr -d ' ') items"
echo "  {x,y,z}{a,b}{1,2}: $(echo {x,y,z}{a,b}{1,2} | wc -w | tr -d ' ') items"

echo
echo "── padding behavior ──"
echo "  {001..010}: zero-padded if any literal starts with 0"
echo "  {1..10}:    no padding"

echo
echo "── reverse via step ──"
echo "  {10..1..-1}:" {10..1..-1}
echo "  {z..a..-3}:" {z..a..-3}

# === ztest assertions ===
nums=( {1..5} )
zassert_eq "${nums[*]}" "1 2 3 4 5" "numeric range 1..5"
neg=( {-3..3} )
zassert_eq "${neg[*]}"  "-3 -2 -1 0 1 2 3" "neg range"
zero=( {01..05} )
zassert_eq "${zero[*]}" "01 02 03 04 05" "zero-padded range"
letters=( {a..e} )
zassert_eq "${letters[*]}" "a b c d e" "alpha range"
prod=( {a,b,c}{1,2,3} )
zassert_eq "${#prod}"   9   "product cardinality"
zassert_eq "${prod[1]}" "a1" "product first"
zassert_eq "${prod[9]}" "c3" "product last"
zassert_eq "${#days}"   7   "days"
zassert_eq "${#months}" 12  "months"
zassert_eq "${#digits}" 10  "digits"
zassert_eq "${#hex}"    16  "hex digits"
ztest_run
