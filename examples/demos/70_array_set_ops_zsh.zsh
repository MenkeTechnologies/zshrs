#!/usr/bin/env zshrs
# Native zsh array set ops — :| (subtract) and :* (intersect).
# Ported from zsh's subst.c paramsubst array-arithmetic branch.

A=(alpha beta gamma delta epsilon)
B=(beta delta zeta eta theta)

echo "── A ──"
print -l ${(o)A}
echo "── B ──"
print -l ${(o)B}

echo "── A:|B  (subtract — elements in A not in B) ──"
diff=( ${A:|B} )
print -l ${(o)diff}

echo "── A:*B  (intersect — elements in both A and B) ──"
inter=( ${A:*B} )
print -l ${(o)inter}

echo "── union (no native; use sort -u on concat) ──"
print -l ${(ou)A:|B}    # but :| with empty exclude doesn't work as union
# Build union via (u) on concat:
union=( ${A[@]} ${B[@]} )
print -l ${(ou)union}

echo "── symmetric difference (A \\\\ B) ∪ (B \\\\ A) ──"
ab=( ${A:|B} )
ba=( ${B:|A} )
print -l ${(ou)ab[@]} ${ba[@]}

echo "── set membership test ──"
needle=delta
if (( ${A[(I)$needle]} > 0 )); then
    echo "$needle ∈ A (at index ${A[(I)$needle]})"
else
    echo "$needle ∉ A"
fi
for needle in alpha zeta omega; do
    if (( ${A[(I)$needle]} > 0 )); then
        echo "$needle ∈ A"
    else
        echo "$needle ∉ A"
    fi
done

echo "── subset check (A ⊆ B?) ──"
small=(beta delta)
big=(alpha beta gamma delta epsilon)
all_in=1
for x in $small; do
    if (( ${big[(I)$x]} == 0 )); then
        all_in=0
        break
    fi
done
(( all_in )) && echo "small ⊆ big" || echo "small ⊄ big"
