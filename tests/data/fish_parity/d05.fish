set x a b c d e
echo $x[-1..1]
echo $x[-2..-4]
set -l i 2
set -l j -1
echo $x[$i..$j]
echo $x[4..2]
echo $x[1..10]
