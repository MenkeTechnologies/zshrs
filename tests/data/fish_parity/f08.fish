set -l a 1 2 3
for i in (seq (count $a))
    echo $i=$a[$i]
end
set -l r
for x in $a
    set r $x $r
end
echo $r
