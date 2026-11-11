set x 1
set x $x 2
set x $x 3
echo $x
set x[2] 9
echo $x
set -e x[1]
echo $x
set -a x 7 8
echo $x
set -p x 0
echo $x
