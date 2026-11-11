set -l a 1 2 3
set -l sum 0
for x in $a
    set sum (math $sum + $x)
end
echo $sum
echo (math "$sum / 4")
echo (math -s2 "$sum / 4")
