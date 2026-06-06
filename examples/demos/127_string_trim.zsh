#!/usr/bin/env zshrs
# String trim/squeeze/padding utilities.
# Pattern uses Src/subst.c (parameter expansion + extended_glob).

setopt extended_glob

trim_left() {
    local s=$1
    echo "${s##[[:space:]]##}"
}

trim_right() {
    local s=$1
    echo "${s%%[[:space:]]##}"
}

trim() {
    local s=$1
    s=${s##[[:space:]]##}
    s=${s%%[[:space:]]##}
    echo "$s"
}

squeeze_ws() {
    local s=$1
    # Replace runs of whitespace with single space.
    s=${s//[[:space:]]##/ }
    echo "$s"
}

repeat_char() {
    local ch=$1 n=$2 out=""
    local i
    for ((i = 0; i < n; i++)); do out+=$ch; done
    echo "$out"
}

pad_left() {
    local s=$1 width=$2 fill=${3:- }
    local n=${#s}
    if (( n >= width )); then echo "$s"; return; fi
    local pad
    pad=$(repeat_char "$fill" $((width - n)))
    echo "${pad}${s}"
}

pad_right() {
    local s=$1 width=$2 fill=${3:- }
    local n=${#s}
    if (( n >= width )); then echo "$s"; return; fi
    local pad
    pad=$(repeat_char "$fill" $((width - n)))
    echo "${s}${pad}"
}

center() {
    local s=$1 width=$2 fill=${3:- }
    local n=${#s}
    if (( n >= width )); then echo "$s"; return; fi
    local total=$((width - n))
    local left=$((total / 2))
    local right=$((total - left))
    echo "$(repeat_char "$fill" $left)${s}$(repeat_char "$fill" $right)"
}

echo "── trim ──"
input="    hello world   "
echo "before: '$input' len=${#input}"
echo "trim:   '$(trim "$input")'"
echo "left:   '$(trim_left "$input")'"
echo "right:  '$(trim_right "$input")'"

echo "── squeeze whitespace ──"
echo "before: '   too    many     spaces   '"
echo "after:  '$(squeeze_ws "   too    many     spaces   ")'"

echo "── pad ──"
echo "[$(pad_left "42" 10 "0")]"
echo "[$(pad_left "hello" 12 ".")]"
echo "[$(pad_right "hi" 10 "-")]"
echo "[$(center "Title" 20 "*")]"
echo "[$(center "X" 11 "-")]"

echo "── width-aware table ──"
hdr_l=$(pad_right "Name" 12)
hdr_r=$(pad_left "Score" 6)
echo "${hdr_l}${hdr_r}"
echo "$(pad_right "Alice" 12)$(pad_left "30" 6)"
echo "$(pad_right "Bob" 12)$(pad_left "75" 6)"
echo "$(pad_right "Carol" 12)$(pad_left "92" 6)"

echo "── repeat string N times ──"
echo "$(repeat_char "-" 30)"
echo "$(repeat_char "=" 30)"
echo "$(repeat_char "abc" 5)"   # repeats string, not just char

# === ztest assertions ===
# trim_left/right + trim
zassert_eq "$(trim_left "   hi")"    "hi"   "trim_left"
zassert_eq "$(trim_right "hi   ")"   "hi"   "trim_right"
zassert_eq "$(trim "   hi   ")"      "hi"   "trim both"
# squeeze
zassert_eq "$(squeeze_ws "a    b   c")"  "a b c"  "squeeze ws"
# repeat_char
zassert_eq "$(repeat_char '-' 5)"  "-----"          "repeat 5 dashes"
zassert_eq "$(repeat_char 'ab' 3)" "ababab"         "repeat string"
zassert_eq "${#$(repeat_char '-' 30)}"  "30"        "repeat 30 dashes length"
# pad_left / pad_right / center
zassert_eq "$(pad_left '42' 5 '0')"   "00042"      "pad_left 0"
zassert_eq "$(pad_right 'hi' 5 '-')"  "hi---"      "pad_right -"
zassert_eq "$(center 'X' 5 '-')"      "--X--"      "center -"
zassert_eq "$(center 'X' 6 '-')"      "--X---"     "center even width"
# pad larger than width = passthrough
zassert_eq "$(pad_left 'hello' 3)" "hello"  "pad noop when too big"
ztest_run
