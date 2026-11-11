set x 1
set x[3] 3 2>/dev/null
echo (count $x)
set y a b c
set y[2] B
echo $y
set -e y[1]
echo $y
echo $y[1]
