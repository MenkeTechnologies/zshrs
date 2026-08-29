#!/usr/bin/env zshrs
# Set operations — union, intersection, difference via assoc-array sentinels.

a=(apple banana cherry date)
b=(banana date elderberry fig grape)

# Union via assoc-array key set.
typeset -A U
for x in "${a[@]}" "${b[@]}"; do U[$x]=1; done
union=( ${(@k)U} )
echo "── union (sorted) ──"
print -l ${(o)union}

# Intersection: items in A that also appear in B.
typeset -A B_SET
for x in "${b[@]}"; do B_SET[$x]=1; done
inter=()
for x in "${a[@]}"; do
    [[ -n ${B_SET[$x]+x} ]] && inter+=($x)
done
echo "── intersection ──"
print -l ${(o)inter}

# Difference A - B: items in A not in B.
diff=()
for x in "${a[@]}"; do
    [[ -z ${B_SET[$x]+x} ]] && diff+=($x)
done
echo "── difference A - B ──"
print -l ${(o)diff}

# Symmetric difference: in one but not both.
typeset -A A_SET
for x in "${a[@]}"; do A_SET[$x]=1; done
symdiff=()
for x in "${a[@]}"; do
    [[ -z ${B_SET[$x]+x} ]] && symdiff+=($x)
done
for x in "${b[@]}"; do
    [[ -z ${A_SET[$x]+x} ]] && symdiff+=($x)
done
echo "── symmetric difference ──"
print -l ${(o)symdiff}

# === ztest assertions ===
# NB: `${(o)union}` inside a nested substitution joins before it sorts —
# subscript with [@] so the sort sees the array elements.
zassert_eq "${(j: :)${(o)union[@]}}"   "apple banana cherry date elderberry fig grape" "union"
zassert_eq "${(j: :)${(o)inter}}"   "banana date"                                   "intersection"
zassert_eq "${(j: :)${(o)diff}}"    "apple cherry"                                  "A - B"
zassert_eq "${(j: :)${(o)symdiff}}" "apple cherry elderberry fig grape"             "symmetric diff"
zassert_eq "${#union}"  "7" "union size"
zassert_eq "${#inter}"  "2" "intersection size"
zassert_eq "${#diff}"   "2" "difference size"
zassert_eq "${#symdiff}" "5" "symdiff size"
ztest_run
