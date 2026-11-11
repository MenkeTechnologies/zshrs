set -l x "a b"
echo $x
echo "$x"
set -l y $x
echo (count $y)
set z "$x"
echo (count $z)
