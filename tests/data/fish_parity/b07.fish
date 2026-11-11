set -l a (echo hello | string upper)
echo $a
echo "x"(echo y)"z"
echo (echo a b | string split " ")[2]
echo -n (echo -n abc)
echo
