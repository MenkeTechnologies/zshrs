#!/usr/bin/env zshrs
# Pattern + glob flags — (#i), (#l), (#a), (#m).
# Ported from Src/pattern.c patcompile + Src/subst.c (#X) prefix flag.

setopt extended_glob

echo "── (#i) case-insensitive match ──"
files=(README.md readme.MD ReadMe.md notes.txt RESTAPI.md)
for f in $files; do
    if [[ $f == (#i)readme* ]]; then echo "  match: $f"; fi
done

echo "── (#a) approximate match ──"
words=(color colour clr color1 colors)
for w in $words; do
    if [[ $w == (#a1)color ]]; then echo "  ~match: $w"; fi
done

echo "── (#l) lowercase normalize ──"
echo "${(L)NAME:-${(L):-MIXED-Case-Here}}"

echo "── (#m) ─ MATCH variable for matched text ──"
text="abc123def456ghi"
for w in $text; do
    if [[ $w =~ "([a-z]+)([0-9]+)" ]]; then
        echo "  letters: ${match[1]} digits: ${match[2]}"
    fi
done

echo "── case-sensitive vs case-insensitive in glob ──"
tmpdir=/tmp/zshrs_glob_case_$$
mkdir -p "$tmpdir"
touch "$tmpdir"/{Alpha.txt,beta.TXT,GAMMA.Txt,delta.txt}
cd "$tmpdir"

echo "case-sensitive *.txt:"
print -l *.txt
echo "case-insensitive (#i)*.txt:"
print -l (#i)*.txt 2>/dev/null || echo "(falling back) *.[tT][xX][tT]:"; print -l *.[tT][xX][tT]

cd /tmp
rm -rf "$tmpdir"

echo "── ranges with character classes ──"
words=(alpha BETA gamma DELTA epsilon)
for w in $words; do
    [[ $w == [A-Z]* ]] && echo "  uppercase first: $w"
done
for w in $words; do
    [[ $w == [a-z]* ]] && echo "  lowercase first: $w"
done

echo "── numeric range pattern ──"
nums=(file001 file042 file100 file999 fileabc)
for n in $nums; do
    [[ $n == file<0-100> ]] && echo "  in range: $n"
done

# === ztest assertions ===
setopt extended_glob
# (#i) case-insensitive prefix match.
[[ "README.md" == (#i)readme* ]] && r1=ok || r1=no
[[ "readme.MD" == (#i)readme* ]] && r2=ok || r2=no
[[ "notes.txt" == (#i)readme* ]] && r3=ok || r3=no
zassert_eq "$r1" "ok" "(#i) matches uppercase README"
zassert_eq "$r2" "ok" "(#i) matches mixed-case readme"
zassert_eq "$r3" "no" "(#i) skips non-prefix"
# Case-insensitive range pattern via [A-Z][a-z].
words=(alpha BETA gamma DELTA epsilon)
up=()
for w in $words; do
    [[ $w == [A-Z]* ]] && up+=("$w")
done
zassert_eq "${(j: :)up}" "BETA DELTA" "[A-Z] picks uppercase-first"
# Numeric range <0-100>.
nums=(file001 file042 file100 file999)
ok=()
for n in $nums; do
    [[ $n == file<0-100> ]] && ok+=("$n")
done
zassert_eq "${(j: :)ok}" "file001 file042 file100" "<0-100> numeric range"
# =~ with capture groups populates match array.
[[ "abc123" =~ "([a-z]+)([0-9]+)" ]]
zassert_eq "${match[1]}" "abc" "match[1] captures letters"
zassert_eq "${match[2]}" "123" "match[2] captures digits"
ztest_run
