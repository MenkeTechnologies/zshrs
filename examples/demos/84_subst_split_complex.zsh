#!/usr/bin/env zshrs
# Complex substitution + split patterns — IFS-driven, $= word-splitting,
# (s/sep/) flag, (z) shell-word, (f) lines.
# Ported from zsh's subst.c + lex.c split routines.

echo "── \$= forces word-splitting on assignment ──"
sentence="alpha beta gamma delta"
arr=( $=sentence )
echo "count=${#arr[@]}"
for w in $arr; do echo "  $w"; done

echo "── (s::) split by arbitrary separator ──"
csv="one,two,three,four,five"
print -l ${(s:,:)csv}

echo "── (s::) on multi-char ──"
arrowed="a -> b -> c -> d -> e"
print -l ${(s: -> :)arrowed}

echo "── IFS-based field split (one-liner) ──"
s="path1:path2:path3:path4"
parts=( ${(s/:/)s} )
for p in $parts; do echo "  $p"; done

echo "── (f) split on newlines (in heredoc data) ──"
multiline=$(cat <<EOF
line A
line B
line C
EOF
)
arr2=( ${(f)multiline} )
for l in $arr2; do echo "  line: $l"; done

echo "── (z) split as shell would word-split ──"
cmd='echo hello world "quoted phrase"'
tokens=( ${(z)cmd} )
echo "token count: ${#tokens[@]}"
for t in $tokens; do echo "  [$t]"; done

echo "── inverse: build a delim-joined string ──"
parts=(alpha beta gamma delta)
echo "(j:,:)   = ${(j:,:)parts}"
echo "(j: \\| :) = ${(j: | :)parts}"

echo "── nested split + filter ──"
raw="apple,banana,cherry,date,elderberry"
arr3=( ${(s:,:)raw} )
short=( ${(M)arr3:#?????} )    # keep only 5-char words
echo "5-char words: ${short[@]}"

echo "── round-trip split/join is idempotent ──"
orig="x:y:z:a:b"
arr4=( ${(s/:/)orig} )
back=${(j/:/)arr4}
echo "orig: $orig"
echo "back: $back"
[[ "$orig" == "$back" ]] && echo "round-trip OK"

# === ztest assertions ===
# $= word-splitting
sentence="alpha beta gamma delta"
arr=( $=sentence )
zassert_eq "${#arr[@]}"     4              "\$= splits into 4 words"
zassert_eq "${arr[1]}"      "alpha"        "\$= first word"
zassert_eq "${arr[4]}"      "delta"        "\$= last word"
# (s::) split
csv="one,two,three,four,five"
csv_arr=( ${(s:,:)csv} )
zassert_eq "${#csv_arr[@]}" 5              "(s:,:) splits 5 fields"
zassert_eq "${csv_arr[3]}"  "three"        "(s:,:) 3rd field"
# (f) split on newlines
multiline=$(printf 'a\nb\nc')
ml_arr=( ${(f)multiline} )
zassert_eq "${#ml_arr[@]}"  3              "(f) split on newlines"
zassert_eq "${ml_arr[2]}"   "b"            "(f) 2nd line"
# (j::) join
parts=(alpha beta gamma delta)
zassert_eq "${(j:,:)parts}"      "alpha,beta,gamma,delta"     "(j:,:) join"
zassert_eq "${(j: | :)parts}"    "alpha | beta | gamma | delta" "(j::) multichar join"
# round-trip
orig="x:y:z:a:b"
arr_rt=( ${(s/:/)orig} )
back=${(j/:/)arr_rt}
zassert_eq "$back" "$orig"                 "split/join round-trip"
ztest_run

