#!/usr/bin/env zshrs
# Greedy/non-greedy parameter strip — ${var#pat} ${var##pat} ${var%pat} ${var%%pat}.
# Ported from Src/subst.c paramsubst (strip dispatch).

p=/usr/local/share/zsh/site-functions/_completion.gz

echo "── #pat: trim shortest from start ──"
echo "${p#*/}     # one segment off"

echo "── ##pat: trim longest from start ──"
echo "${p##*/}   # all path → tail"

echo "── %pat: trim shortest from end ──"
echo "${p%/*}    # one segment off end"

echo "── %%pat: trim longest from end ──"
echo "${p%%/*}   # all the way to root"

echo "── multiple strips chained ──"
echo "${p##*/}      # name with ext"
echo "${${p##*/}%.*}  # strip ext too"

echo "── numeric ranges + strip ──"
s="abc123def456"
echo "strip trailing digits: ${s%%[0-9]##}" 2>/dev/null \
    || echo "strip trailing digits (basic): ${s%[0-9]*}"

echo "── leading whitespace trim ──"
spaced="    hello world   "
echo "before: '$spaced' len=${#spaced}"
trimmed=${spaced##[[:space:]]##}
trimmed=${trimmed%%[[:space:]]##}
echo "after:  '$trimmed' len=${#trimmed}"

echo "── strip prefix only if present ──"
url=https://example.com/path
echo "strip https://: ${url#https://}"
echo "strip http:// (absent): ${url#http://}    # unchanged"

echo "── strip file extension ──"
for f in main.rs script.zsh README data.tar.gz; do
    echo "$f → ${f%.*}"
done

echo "── strip leading 0s ──"
for n in 00042 007 12345 0; do
    stripped=${n##0##}
    echo "$n → ${stripped:-0}"
done

echo "── replace + strip combined ──"
path=/usr/share/zsh
echo "uppercase last dir: ${path:t:u}"

# === ztest assertions ===
pp=/usr/local/share/zsh/site-functions/_completion.gz
# #pat: trim shortest prefix
zassert_eq "${pp#*/}"  "usr/local/share/zsh/site-functions/_completion.gz"  "#*/ shortest start"
# ##pat: trim longest prefix
zassert_eq "${pp##*/}" "_completion.gz"  "##*/ longest start (basename)"
# %pat: trim shortest suffix
zassert_eq "${pp%/*}"  "/usr/local/share/zsh/site-functions"  "%/* shortest end (dirname)"
# %%pat: trim longest suffix
zassert_eq "${pp%%/*}" ""  "%%/* longest end (empty before root)"
# Chained: file stem
zassert_eq "${${pp##*/}%.*}"  "_completion"  "chained ## then %"
# strip protocol via #*://
url=https://example.com/path
zassert_eq "${url#*://}"     "example.com/path"           "strip protocol"
zassert_eq "${url#http://}"  "https://example.com/path"   "unchanged when prefix absent"
# file extension strip
zassert_eq "${${:-main.rs}%.*}"   "main"   "strip .rs"
zassert_eq "${${:-data.tar.gz}%.*}" "data.tar"  "strip last .gz only (%.*)"
# tail+upper modifier chain
zassert_eq "${path:t:u}"  "ZSH"  "(:t:u) basename uppercased"
ztest_run
