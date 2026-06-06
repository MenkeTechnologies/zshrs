#!/usr/bin/env zshrs
# UTF-8 string handling — length, slice, iteration.
# Ported from Src/params.c MB_METACHARLEN + multibyte option.

s="héllo wörld"

echo "── basic length ──"
echo "byte length: ${#s}"
# zsh counts as bytes unless MULTIBYTE is set.

echo "── set multibyte ──"
setopt multibyte
echo "char length: ${#s}"

echo "── string with emoji ──"
emoji="🚀 🌟 ✨"
echo "byte length: $(echo -n "$emoji" | wc -c)"
echo "char length: ${#emoji}"

echo "── slicing UTF-8 ──"
greek="αβγδεζ"
echo "first 3: ${greek[1,3]}"
echo "last 3:  ${greek[-3,-1]}"

echo "── iterate chars ──"
for c in $(echo $greek | fold -w1); do
    echo "  $c"
done

echo "── via (s::) split ──"
print -l ${(s::)greek}

echo "── mixed ASCII + multibyte ──"
mix="A→B→C→D"
echo "len: ${#mix}"
echo "chars:"
for ((i = 1; i <= ${#mix}; i++)); do
    echo "  [$i] = ${mix[i]}"
done

echo "── ROT-N on ASCII portion ──"
ascii_part="Hello"
rotted=$(echo "$ascii_part" | tr 'A-Za-z' 'D-ZA-Cd-za-c')
echo "$ascii_part → $rotted"

echo "── case mapping with multibyte (partial — works on ASCII) ──"
echo "${mix:u}"
echo "${mix:l}"

echo "── line wrap helper ──"
wrap() {
    local txt=$1 width=$2 cur="" word
    for word in ${(s/ /)txt}; do
        if (( ${#cur} + ${#word} + 1 > width )); then
            echo "$cur"
            cur=$word
        else
            cur="${cur:+$cur }$word"
        fi
    done
    [[ -n $cur ]] && echo "$cur"
}
wrap "the quick brown fox jumps over the lazy dog and the cat too" 20

echo "── unicode normalize via printf (basic) ──"
printf 'U+%04X\n' 65 97 233 945 9731

# === ztest assertions ===
setopt multibyte
# 'héllo wörld' has 11 chars (5+1+5).
zassert_eq "${#s}"  "11"  "${#s} multibyte len"
# Greek slice
gr="αβγδεζ"
zassert_eq "${gr[1,3]}"   "αβγ"  "greek first 3 chars"
zassert_eq "${gr[-3,-1]}" "δεζ"  "greek last 3 chars"
zassert_eq "${#gr}"        "6"   "6 greek chars"
# Mixed counts 7 "chars" (4 ASCII + 3 arrows)
zassert_eq "${#mix}"  "7"   "mixed length 7"
zassert_eq "${mix[1]}" "A"  "mixed [1]=A"
zassert_eq "${mix[2]}" "→"  "mixed [2]=arrow"
# Case mapping on mixed (ASCII portion only)
zassert_eq "${mix:l}"  "a→b→c→d"  "lower mixed"
zassert_eq "${mix:u}"  "A→B→C→D"  "upper mixed"
# wrap helper produces 3 lines
wrapped=$(wrap "the quick brown fox jumps over the lazy dog and the cat too" 20 | wc -l | tr -d ' ')
zassert_eq "$wrapped" "3"  "wrap to 3 lines"
ztest_run
