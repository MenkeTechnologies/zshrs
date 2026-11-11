set -l x a b c
set -l y $x[2..-1]
echo $y
echo $x[(count $x)]
