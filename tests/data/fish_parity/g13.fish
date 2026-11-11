set -l p (echo "a  b")
echo $p
echo "$p"
set -l q "x y"
echo (count $q)
echo (string split " " -- $q | count)
