#!/usr/bin/env zshrs
# Array pattern filtering — :# (drop), (M):# (keep), (i)/(I) subscript flag.
# Ported from zsh's subst.c paramsubst (pattern-match branch) +
# pattern.c (Src/pattern.c patcompile/patmatch).

files=(main.zsh test.zsh README.md install.sh build.zsh notes.txt)

echo "── ${files:#pattern}  drop matching ──"
print -l ${files:#*.zsh}    # drop .zsh files

echo "── ${(M)files:#pattern}  keep matching ──"
print -l ${(M)files:#*.zsh}

echo "── chained filter: drop *.zsh then drop test.* ──"
no_zsh=( ${files:#*.zsh} )
no_test=( ${no_zsh:#test.*} )
print -l ${no_test}

echo "── (i)name  exact-case index ──"
arr=(apple banana cherry date)
echo "(i)apple  = ${arr[(i)apple]}"
echo "(i)cherry = ${arr[(i)cherry]}"
echo "(i)nope   = ${arr[(i)nope]}   # 0 = not found (zsh: len+1)"

echo "── (I)pat   last-matching index ──"
items=(foo bar foo baz foo)
echo "first (i)foo = ${items[(i)foo]}"
echo "last  (I)foo = ${items[(I)foo]}"

echo "── pipe through pattern filter ──"
src=(test_1.log test_2.log test_3.log live.log)
filtered=( ${(M)src:#test_*} )
echo "test logs: ${filtered[@]}"
not_filtered=( ${src:#test_*} )
echo "non-test:  ${not_filtered[@]}"

echo "── count entries matching pattern ──"
total=0
for f in $files; do
    [[ $f == *.zsh ]] && (( total++ ))
done
echo ".zsh count: $total"

echo "── partition into two arrays ──"
zsh_only=()
other=()
for f in $files; do
    if [[ $f == *.zsh ]]; then
        zsh_only+=($f)
    else
        other+=($f)
    fi
done
echo "zsh files: ${zsh_only[@]}"
echo "others:    ${other[@]}"

# === ztest assertions ===
nozsh=( ${files:#*.zsh} )
zassert_eq "${nozsh[*]}" "README.md install.sh notes.txt" ":# drops .zsh"
keep=( ${(M)files:#*.zsh} )
zassert_eq "${keep[*]}"  "main.zsh test.zsh build.zsh"     "(M):# keeps .zsh"
arr=(apple banana cherry date)
zassert_eq "${arr[(i)apple]}"  1 "(i)apple = 1"
zassert_eq "${arr[(i)cherry]}" 3 "(i)cherry = 3"
zassert_eq "${arr[(i)nope]}"   5 "(i) not-found = len+1 (4+1=5)"
items=(foo bar foo baz foo)
zassert_eq "${items[(i)foo]}"  1 "(i) first foo idx"
zassert_eq "${items[(I)foo]}"  5 "(I) last foo idx"
zassert_eq "${#zsh_only[@]}"   3 "partition zsh count"
zassert_eq "${#other[@]}"      3 "partition other count"
zassert_eq "${zsh_only[*]}"   "main.zsh test.zsh build.zsh" "partition zsh files"
ztest_run
