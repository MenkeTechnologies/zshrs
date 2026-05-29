#!/usr/bin/env zshrs
# Join / split parameter flags — (j:str:) (s:str:) (f) (F) (p).
# Ported from zsh's subst.c paramsubst flag dispatch.

arr=(alpha beta gamma delta)

echo "── (j:str:) join with separator ──"
echo "(j:,:)  : ${(j:,:)arr}"
echo "(j:-:)  : ${(j:-:)arr}"
echo "(j: -> :) : ${(j: -> :)arr}    # arrow joiner"

echo "── (s:str:) split by separator ──"
csv="one,two,three,four"
print -l ${(s:,:)csv}

echo "── (s:str:) on multi-char separator ──"
arrowed="a -> b -> c -> d"
print -l ${(s: -> :)arrowed}

echo "── (f) split on newlines ──"
multi=$'line1\nline2\nline3'
print -l ${(f)multi}

echo "── (F) join with newlines (counterpart of f) ──"
echo "${(F)arr}"

echo "── round-trip: split → join ──"
src="a:b:c:d:e"
arr2=( ${(s/:/)src} )
echo "split count: ${#arr2[@]}"
joined="${(j/-/)arr2}"
echo "joined: $joined"

echo "── pipe through both: split then re-join differently ──"
src="a:b:c:d"
arr3=( ${(s/:/)src} )
echo "via (j: , :): ${(j: , :)arr3}"
